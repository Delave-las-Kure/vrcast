//! T610 — a rollback does not touch the quality-limit rules.
//!
//! **The bug.** `/etc/caddy/vrcast-limits.conf` was in `upgrade::OWNED`, so every run copied
//! it into `/etc/vrcast/backup/<stamp>/` and `server_rollback` copied it back — silently
//! wiping every `limit_set`/`limit_clear` made since the run, and handing the file an OLD
//! `# vrcast-generation` number. T603's compare-and-swap relies on that number only ever
//! growing; a rollback putting an old one back is the ABA it exists to rule out. And the copy
//! was a bare `cp`, outside the limits lock, free to land in the middle of a transaction.
//!
//! **The decision (owner, 2026-09-23).** The file is in neither the copy nor the restore. Its
//! life belongs to the limit commands alone; the deployment only lays it down when it is not
//! there at all.
//!
//! Two cases, both red on the code before the fix:
//!
//! - (a) the owner's own wording — a limit set after a run, then a rollback, and the rule is
//!   still there. "The run" is the real `upgrade::back_up`, what a run does first, not a copy
//!   laid down by hand;
//! - (b) a `latest` left by an EARLIER client, which did copy the rules file: the restore has
//!   no arm for it any more and must pass it by.
//!
//! Both carry a positive control — the Caddyfile, which a rollback MUST put back — so a
//! rollback that quietly did nothing at all cannot pass for one that left the rules alone.
//! The plain test container and a key profile, for the reasons `rollback_refused.rs` gives.
//!
//! The container has no systemd, so the rollback's `systemctl reload caddy || true` does
//! nothing here; on a real server it would re-read the files from disk. The tests do that
//! reload by hand and then ask the serving, as a viewer, what it now gives out.

use futures::future::BoxFuture;
use vrcast_studio_lib::commands::deploy::api as deploy;
use vrcast_studio_lib::domain::dns_verdict::{Ipv6Choice, ServerAddresses};
use vrcast_studio_lib::domain::limits_conf::read_generation;
use vrcast_studio_lib::server::deploy::{machine, Context, Proofs};
use vrcast_studio_lib::server::upgrade;

use super::fixture::TestServer;
use super::hls_fixture::lay_out_ladder;
use super::limits_live::{
    contents, exists, good_url, offered, offered_to_everybody, put_limit, short_at, the_ladder,
    CONF,
};
use super::rollback_refused::setup;
use super::ssh_live::connect;
use super::viewer::Viewer;

const CADDYFILE: &str = "/etc/caddy/Caddyfile";
const LATEST: &str = "/etc/vrcast/backup/latest";

/// Reload the web server from the files on disk — what `systemctl reload caddy` does on a
/// real server and cannot do in this container.
fn reload_from_disk(server: &TestServer) {
    let said = server
        .exec_inside(&format!(
            "caddy reload --config {CADDYFILE} --adapter caddyfile 2>&1 && echo RELOADED"
        ))
        .expect("the web server would not reload");
    assert!(
        said.contains("RELOADED"),
        "the web server refused the files the rollback left: {said}"
    );
}

/// Append a comment to the live Caddyfile — still a valid configuration, and a different one.
fn mark_caddyfile(server: &TestServer, mark: &str) {
    server
        .exec_inside(&format!("printf '\\n# T610: {mark}\\n' >> {CADDYFILE}"))
        .expect("the Caddyfile was not marked");
}

/// What a limited viewer and everybody else are offered, after the rollback and a reload,
/// must be what the limit made them: the shortened set and the whole one.
async fn the_limit_still_serves(server: &TestServer, viewer: &Viewer, cap: u64, rungs: usize) {
    reload_from_disk(server);
    let theirs = offered(viewer, "demo");
    assert!(
        !theirs.is_empty() && theirs.len() < rungs && theirs.iter().all(|b| *b <= cap),
        "after the rollback the limited viewer is no longer limited: offered {theirs:?}, \
         cap {cap}"
    );
    assert_eq!(
        offered_to_everybody(server, "demo").await.len(),
        rungs,
        "after the rollback somebody without a limit lost rungs"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_limit_set_after_a_run_survives_a_rollback() {
    let (server, state, id) = setup().await;
    lay_out_ladder(&server, "demo").expect("the quality set was not laid out");
    let all = the_ladder(&server);
    let cap = all[1].bandwidth;
    let viewer = Viewer::attach(&server).expect("the viewer would not attach");

    // The run: the real copy a run takes before its first change, with a Context made the
    // way `commands/deploy.rs::server_rollback` makes one.
    mark_caddyfile(&server, "as it was when the run copied it");
    let conn = connect(&server).await;
    let facts = machine::look(&conn).await.expect("no machine facts");
    let never = || -> BoxFuture<'_, bool> { Box::pin(async { false }) };
    let ctx = Context {
        conn: &conn,
        domain: "stream.example.com",
        video_dir: super::hls_fixture::VIDEO_DIR,
        ipv6: Ipv6Choice::Keep,
        server: ServerAddresses { v4: None, v6: None },
        public_key: String::new(),
        machine: facts,
        already_ours: true,
        replace_caddyfile: false,
        run: vrcast_studio_lib::server::deploy::RunMark::fresh(),
        proofs: Proofs {
            key_works: &never,
            password_refused: &never,
        },
    };
    upgrade::back_up(&ctx).await.expect("the run's copy failed");
    conn.close().await;
    let copied_caddyfile = contents(&server, CADDYFILE);
    // What the run then changed — the thing a rollback exists to undo.
    mark_caddyfile(&server, "changed by the run");
    assert_ne!(contents(&server, CADDYFILE), copied_caddyfile);

    // After the run: somebody limits a viewer.
    put_limit(&server, viewer.ip(), "demo", cap, &good_url(&server))
        .await
        .expect("the limit would not go on");
    let rules = contents(&server, CONF);
    assert!(
        rules.contains(viewer.ip()),
        "the limit is not in the rules file: {rules}"
    );
    let generation = read_generation(&rules);
    assert!(generation >= 1, "the limit did not move the generation on");

    deploy::server_rollback(&state, &id)
        .await
        .expect("the rollback on our own current server failed");

    let after = contents(&server, CONF);
    assert_eq!(
        after,
        rules,
        "the rollback changed the quality-limit rules — the limit set after the run is gone \
         (generation {} → {})",
        generation,
        read_generation(&after)
    );
    assert!(
        read_generation(&after) >= generation,
        "the generation went backwards"
    );
    assert!(
        exists(&server, &short_at(cap)),
        "the shortened description the rule points at is gone"
    );
    let kept = server
        .exec_inside(&format!("ls {LATEST}/"))
        .expect("the copy would not be listed");
    assert!(
        !kept.contains("vrcast-limits.conf"),
        "the run copied the rules file aside, so a rollback can put an old one back: {kept}"
    );
    // The positive control: what the rollback must put back, it did.
    assert_eq!(
        contents(&server, CADDYFILE),
        copied_caddyfile,
        "the rollback reported success and did not put the Caddyfile back"
    );

    the_limit_still_serves(&server, &viewer, cap, all.len()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_copy_left_by_an_earlier_client_does_not_bring_old_rules_back() {
    let (server, state, id) = setup().await;
    lay_out_ladder(&server, "demo").expect("the quality set was not laid out");
    let all = the_ladder(&server);
    let cap = all[1].bandwidth;
    let viewer = Viewer::attach(&server).expect("the viewer would not attach");

    // Two changes, so the live generation stands clearly above the old copy's. The first
    // at the lightest rung, the second at the middle one it is replaced by.
    put_limit(
        &server,
        viewer.ip(),
        "demo",
        all[2].bandwidth,
        &good_url(&server),
    )
    .await
    .expect("the first limit would not go on");
    put_limit(&server, viewer.ip(), "demo", cap, &good_url(&server))
        .await
        .expect("the second limit would not go on");
    let rules = contents(&server, CONF);
    let generation = read_generation(&rules);
    assert!(generation >= 2, "two changes did not make generation 2");

    // `latest` the way a client before T610 left it: the Caddyfile (different from the live
    // one), the state file, and the rules file as it was then — no rules, an old generation.
    server
        .exec_inside(&format!(
            "set -e
dir=/etc/vrcast/backup/20260101T000000Z
mkdir -p \"$dir\"
cp -a {CADDYFILE} \"$dir/Caddyfile\"
printf '\\n# T610: the copy an earlier client took\\n' >> \"$dir/Caddyfile\"
cp -a /etc/vrcast/state.json \"$dir/state.json\"
printf '%s\\n' \
  '# The quality-limit rules. This file belongs to VRCast Studio: it is rewritten whole' \
  '# on every change, and anything added here by hand will be lost.' \
  '# vrcast-generation 1' > \"$dir/vrcast-limits.conf\"
ln -sfn \"$dir\" {LATEST}"
        ))
        .expect("the earlier client's copy was not laid down");
    let old_caddyfile = contents(&server, &format!("{LATEST}/Caddyfile"));
    let old_rules = contents(&server, &format!("{LATEST}/vrcast-limits.conf"));
    assert_ne!(old_caddyfile, contents(&server, CADDYFILE));
    assert_ne!(old_rules, rules);

    deploy::server_rollback(&state, &id)
        .await
        .expect("the rollback on our own current server failed");

    let after = contents(&server, CONF);
    assert_eq!(
        after,
        rules,
        "the rollback put the earlier client's copy of the rules file back (generation {} → {})",
        generation,
        read_generation(&after)
    );
    assert!(
        read_generation(&after) >= generation,
        "the generation went backwards"
    );
    assert!(
        exists(&server, &short_at(cap)),
        "the shortened description the rule points at is gone"
    );
    // The positive control.
    assert_eq!(
        contents(&server, CADDYFILE),
        old_caddyfile,
        "the rollback reported success and did not put the Caddyfile back"
    );

    the_limit_still_serves(&server, &viewer, cap, all.len()).await;
}
