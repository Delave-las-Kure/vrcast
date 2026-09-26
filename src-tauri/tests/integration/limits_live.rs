//! T209, T210 — a quality limit against a real web server.
//!
//! Two questions, and neither can be answered without one:
//!
//!   * does the limited viewer really get a different answer, while everybody else gets the
//!     full one, without the video data being copied (FR-061, FR-062, SC-007);
//!   * does a bad rule really roll back, leaving the serving working (FR-063).
//!
//! The second is the one that matters most. A quality limit edits the configuration of the
//! thing that is serving somebody's film **at that moment**, and the promise is that a
//! mistake costs the limit rather than the showing.

use std::time::Duration;

use vrcast_studio_lib::domain::hls_master::{parse, Variant};
use vrcast_studio_lib::domain::limits_conf::Limit;
use vrcast_studio_lib::domain::slow_master::shorten;
use vrcast_studio_lib::server::limits::{LimitError, Serving};

use super::fixture::TestServer;
use super::hls_fixture::{lay_out_ladder, VIDEO_DIR};
use super::ssh_live::connect;
use super::viewer::Viewer;

pub(super) const CONF: &str = "/etc/caddy/vrcast-limits.conf";
const MAIN_CONF: &str = "/etc/caddy/Caddyfile";
const PREFIX: &str = "/videos";

fn when() -> String {
    String::from("2026-08-26T10:00:00Z")
}

/// How much room everything under the serving directory takes.
fn disk_bytes(server: &TestServer) -> u64 {
    server
        .exec_inside(&format!("du -sb {VIDEO_DIR} | cut -f1"))
        .ok()
        .and_then(|out| out.trim().parse().ok())
        .unwrap_or(0)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_limited_viewer_gets_the_shortened_set_and_everyone_else_the_whole_one() {
    let server = TestServer::start().expect("the container would not come up");
    lay_out_ladder(&server, "demo").expect("the quality set was not laid out");
    let viewer = Viewer::attach(&server).expect("the viewer would not attach");

    let before = disk_bytes(&server);
    let whole = viewer
        .fetch("/videos/demo/master.m3u8")
        .expect("the description was not served");
    let all: Vec<Variant> = parse(&whole).expect("the description would not read");
    assert_eq!(all.len(), 3, "the fixture's set is not what it was");

    // The cap sits between the middle rung and the top one.
    let cap = all[1].bandwidth;
    let short = shorten(&all, cap, PREFIX, "demo");
    assert_eq!(short.kept.len(), 2);

    let conn = connect(&server).await;
    let serving = Serving {
        conn: &conn,
        video_dir: VIDEO_DIR,
        conf_path: CONF,
        main_conf: MAIN_CONF,
        serving_prefix: PREFIX,
        check_url: &format!(
            "http://{}:{}/videos/demo/master.m3u8",
            server.host(),
            server.http_port
        ),
        owner: "root:root",
    };

    serving
        .apply(
            &[Limit {
                ip: viewer.ip().to_owned(),
                slug: String::from("demo"),
                cap_bps: cap,
                set_at: when(),
            }],
            Some("demo"),
            0,
        )
        .await
        .expect("the limit would not go on");

    // The limited address gets the shortened one...
    let limited = viewer
        .fetch("/videos/demo/master.m3u8")
        .expect("the limited viewer was served nothing");
    let theirs: Vec<Variant> = parse(&limited).expect("what they got would not read");
    assert_eq!(
        theirs.len(),
        2,
        "the limited viewer was still offered every rung:\n{limited}"
    );
    assert!(
        theirs.iter().all(|v| v.bandwidth <= cap),
        "a rung above the cap was offered anyway:\n{limited}"
    );

    // ...and the paths in it are absolute, or the player would look for the segments inside
    // the shortened directory, where there are none. This is the recorded mistake.
    for variant in &theirs {
        assert!(
            variant.path.starts_with("/videos/demo/"),
            "a relative path in the shortened description: {}",
            variant.path
        );
    }
    // The segments really are served at those addresses — the whole point of not copying
    // them is that they are the same files.
    let segment = viewer
        .fetch(&theirs[0].path)
        .expect("a variant named in the shortened description is not served");
    assert!(segment.contains("#EXTM3U"));

    // Everyone else still gets everything. Asked from outside the container's network, which
    // is a different address as far as the serving is concerned.
    let others = reqwest::get(&format!(
        "http://{}:{}/videos/demo/master.m3u8",
        server.host(),
        server.http_port
    ))
    .await
    .expect("the serving would not answer")
    .text()
    .await
    .expect("no answer body");
    assert_eq!(
        parse(&others)
            .expect("the full description would not read")
            .len(),
        3,
        "an unlimited viewer lost rungs they were entitled to:\n{others}"
    );

    // SC-007: only a description was made, not another copy of the film.
    let after = disk_bytes(&server);
    let grew = after.saturating_sub(before);
    assert!(
        grew * 100 < before.max(1),
        "the serving directory grew by {grew} bytes on {before} — the video data was copied"
    );

    // And the caching rule was narrowed rather than declared a second time. A description
    // stuck in a player's cache for thirty days would make lifting the limit meaningless.
    let headers = server
        .exec_inside("curl -sS -D - -o /dev/null http://127.0.0.1/videos/demo/master.m3u8")
        .expect("the serving would not answer from inside");
    assert!(
        headers.to_lowercase().contains("cache-control: no-cache"),
        "the description is still cached like a segment:\n{headers}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_rule_the_web_server_refuses_is_rolled_back_and_the_serving_keeps_working() {
    // FR-063. The thing being edited is serving somebody's film at that moment.
    let server = TestServer::start().expect("the container would not come up");
    lay_out_ladder(&server, "demo").expect("the quality set was not laid out");
    let viewer = Viewer::attach(&server).expect("the viewer would not attach");

    let conn = connect(&server).await;
    let serving = Serving {
        conn: &conn,
        video_dir: VIDEO_DIR,
        conf_path: CONF,
        main_conf: MAIN_CONF,
        serving_prefix: PREFIX,
        check_url: &format!(
            "http://{}:{}/videos/demo/master.m3u8",
            server.host(),
            server.http_port
        ),
        owner: "root:root",
    };

    // A sound limit first, so there is something worth keeping to roll back to.
    let whole = viewer
        .fetch("/videos/demo/master.m3u8")
        .expect("the description was not served");
    let all: Vec<Variant> = parse(&whole).expect("the description would not read");
    let cap = all[1].bandwidth;
    serving
        .apply(
            &[Limit {
                ip: viewer.ip().to_owned(),
                slug: String::from("demo"),
                cap_bps: cap,
                set_at: when(),
            }],
            Some("demo"),
            0,
        )
        .await
        .expect("the first limit would not go on");
    let good = server
        .exec_inside(&format!("cat {CONF}"))
        .expect("the rules would not be read");

    // Now something the web server will not have: an address that is not an address.
    let refused = serving
        .apply(
            &[Limit {
                ip: String::from("this-is-not-an-address"),
                slug: String::from("demo"),
                cap_bps: cap,
                set_at: when(),
            }],
            None,
            1,
        )
        .await;

    match refused {
        Err(LimitError::ValidateFailed(said)) => {
            assert!(
                !said.trim().is_empty(),
                "the refusal says nothing a person could act on"
            );
        }
        other => panic!("a nonsensical rule was accepted: {other:?}"),
    }

    // What was there before is back, to the letter. Not something rebuilt from memory of
    // what it used to be: what it is and what we think it is are two different things.
    //
    // To the letter except the generation line (T603): the content comes back under a
    // NEW number, never the old one — a reader that saw the refused change holds the
    // number it wrote, and handing an old number out again would let a later compare-and-
    // swap pass over somebody else's change.
    let now = server
        .exec_inside(&format!("cat {CONF}"))
        .expect("the rules would not be read");
    let without_generation = |text: &str| -> String {
        text.lines()
            .filter(|l| !l.starts_with("# vrcast-generation "))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(
        without_generation(now.trim()),
        without_generation(good.trim()),
        "the previous rules did not come back as they were"
    );
    assert_eq!(
        vrcast_studio_lib::domain::limits_conf::read_generation(&now),
        3,
        "the rollback must write a new generation: 1 before, 2 by the refused change, 3 by \
         putting the old rules back"
    );
    assert!(
        !now.contains("this-is-not-an-address"),
        "the bad rule stayed in the file"
    );

    // The serving works, and the limit that was there is still in force. Nobody watching
    // noticed anything.
    let after = viewer
        .fetch("/videos/demo/master.m3u8")
        .expect("the serving stopped answering the viewer");
    assert_eq!(
        parse(&after).expect("what they got would not read").len(),
        2,
        "the limit that was already in force was lost:\n{after}"
    );

    let kept = serving
        .limits()
        .await
        .expect("the limits would not be read");
    assert_eq!(kept.0.len(), 1);
    assert_eq!(kept.0[0].ip, viewer.ip());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn taking_a_limit_off_gives_the_viewer_the_whole_set_again() {
    // FR-065. Both halves go: the rule and the shortened description. A description left
    // behind would be served again the day somebody set a limit on that medium and expected
    // a fresh one.
    let server = TestServer::start().expect("the container would not come up");
    lay_out_ladder(&server, "demo").expect("the quality set was not laid out");
    let viewer = Viewer::attach(&server).expect("the viewer would not attach");

    let conn = connect(&server).await;
    let serving = Serving {
        conn: &conn,
        video_dir: VIDEO_DIR,
        conf_path: CONF,
        main_conf: MAIN_CONF,
        serving_prefix: PREFIX,
        check_url: &format!(
            "http://{}:{}/videos/demo/master.m3u8",
            server.host(),
            server.http_port
        ),
        owner: "root:root",
    };

    let all: Vec<Variant> = parse(
        &viewer
            .fetch("/videos/demo/master.m3u8")
            .expect("the description was not served"),
    )
    .expect("the description would not read");
    let cap = all[1].bandwidth;
    serving
        .apply(
            &[Limit {
                ip: viewer.ip().to_owned(),
                slug: String::from("demo"),
                cap_bps: cap,
                set_at: when(),
            }],
            Some("demo"),
            0,
        )
        .await
        .expect("the limit would not go on");

    serving
        .apply(&[], None, 1)
        .await
        .expect("the limit would not come off");

    let back = viewer
        .fetch("/videos/demo/master.m3u8")
        .expect("the serving stopped answering");
    assert_eq!(
        parse(&back).expect("what they got would not read").len(),
        3,
        "the viewer did not get their rungs back:\n{back}"
    );
    assert!(serving.limits().await.unwrap().0.is_empty());

    let left = server
        .exec_inside(&format!(
            "test -e {VIDEO_DIR}/_slow/demo && echo still-there || echo gone"
        ))
        .expect("the server would not answer");
    assert!(
        left.contains("gone"),
        "the shortened description was left behind"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cap_under_everything_serves_the_lightest_rather_than_nothing() {
    // Scenario 6, step 8, and the only one of its steps that was checked without a
    // server. FR-067 is about what a viewer sees, and "an empty description" and "the
    // lightest rung" look identical in a unit test and nothing alike to a person.
    let server = TestServer::start().expect("the container would not come up");
    lay_out_ladder(&server, "demo").expect("the quality set was not laid out");
    let viewer = Viewer::attach(&server).expect("the viewer would not attach");

    let all: Vec<Variant> = parse(
        &viewer
            .fetch("/videos/demo/master.m3u8")
            .expect("the description was not served"),
    )
    .expect("the description would not read");
    let lightest = all.iter().map(|v| v.bandwidth).min().unwrap_or(0);

    // A cap a tenth of the lightest rung: nothing on this server can meet it.
    let cap = lightest / 10;
    let short = shorten(&all, cap, PREFIX, "demo");
    assert!(
        short.below_lightest,
        "the fixture's rungs are lighter than expected"
    );

    let conn = connect(&server).await;
    let serving = Serving {
        conn: &conn,
        video_dir: VIDEO_DIR,
        conf_path: CONF,
        main_conf: MAIN_CONF,
        serving_prefix: PREFIX,
        check_url: &format!(
            "http://{}:{}/videos/demo/master.m3u8",
            server.host(),
            server.http_port
        ),
        owner: "root:root",
    };
    serving
        .apply(
            &[Limit {
                ip: viewer.ip().to_owned(),
                slug: String::from("demo"),
                cap_bps: cap,
                set_at: when(),
            }],
            Some("demo"),
            0,
        )
        .await
        .expect("the limit would not go on");

    let theirs: Vec<Variant> = parse(
        &viewer
            .fetch("/videos/demo/master.m3u8")
            .expect("the limited viewer was served nothing at all"),
    )
    .expect("what they got would not read");

    assert_eq!(
        theirs.len(),
        1,
        "a cap under everything should leave exactly the lightest rung"
    );
    assert_eq!(theirs[0].bandwidth, lightest);
    // And it plays: a description naming a variant that is not served is the same as no
    // video, which is what this whole rule exists to avoid.
    assert!(viewer
        .fetch(&theirs[0].path)
        .expect("the one rung they were left is not served")
        .contains("#EXTM3U"));
}

/// Give a reload a moment on a loaded machine.
#[allow(dead_code)]
const PATIENCE: Duration = Duration::from_secs(30);

// ---------- T600: concurrent limit_set/limit_clear do not silently lose one another ----------

/// One end-to-end "set a limit" cycle, exactly the shape `commands/limits.rs::api::limit_set`
/// itself follows: read the list and its generation in one round-trip, add one rule to the
/// in-memory copy, `apply()` passing that same generation back. Each call opens its own
/// connection — two clients working with one server never share a connection either.
async fn add_one_limit(
    server: &TestServer,
    ip: &str,
    slug: &str,
    cap_bps: u64,
) -> Result<(), LimitError> {
    let conn = connect(server).await;
    let serving = Serving {
        conn: &conn,
        video_dir: VIDEO_DIR,
        conf_path: CONF,
        main_conf: MAIN_CONF,
        serving_prefix: PREFIX,
        check_url: &format!(
            "http://{}:{}/videos/{slug}/master.m3u8",
            server.host(),
            server.http_port
        ),
        owner: "root:root",
    };
    let (existing, generation) = serving.limits().await?;
    let mut limits = existing;
    limits.push(Limit {
        ip: ip.to_owned(),
        slug: slug.to_owned(),
        cap_bps,
        set_at: when(),
    });
    let outcome = serving.apply(&limits, Some(slug), generation).await;
    conn.close().await;
    outcome
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_concurrent_limit_sets_never_silently_lose_one_another() {
    // T600. Without the generation check, `apply()` was a plain read-modify-write: two
    // concurrent calls each read the list before either had written, each added their own
    // rule to their own in-memory copy, and whichever wrote last won — the other's rule
    // vanished with no error at all. `tokio::join!` races the two calls over real SSH
    // round-trips rather than any `sleep()` hook inside the production code, the same
    // approach `ladder_build_race.rs`/`deploy_run_race.rs` already take for their own races.
    //
    // Run several times in one test (races are not deterministic): every run must land in
    // one of the two ACCEPTABLE outcomes named in the task — (a) both succeed and the file
    // ends up with both rules (a genuinely sequential pair of round-trips, not a real race),
    // or (b) exactly one succeeds and the other gets `LimitsConflict` naming the rule that
    // did land. What must NEVER happen, on any run, is both calls returning `Ok(())` while
    // the file holds only one of the two rules — that is the silent loss this task exists to
    // rule out.
    for round in 0..5 {
        let server = TestServer::start().expect("the container would not come up");
        lay_out_ladder(&server, "demo").expect("the quality set was not laid out");

        let ip_a = format!("203.0.113.{}", 10 + round);
        let ip_b = format!("203.0.113.{}", 110 + round);

        let (result_a, result_b) = tokio::join!(
            add_one_limit(&server, &ip_a, "demo", 6_000_000),
            add_one_limit(&server, &ip_b, "demo", 6_000_000),
        );

        let conn = connect(&server).await;
        let serving = Serving {
            conn: &conn,
            video_dir: VIDEO_DIR,
            conf_path: CONF,
            main_conf: MAIN_CONF,
            serving_prefix: PREFIX,
            check_url: &format!(
                "http://{}:{}/videos/demo/master.m3u8",
                server.host(),
                server.http_port
            ),
            owner: "root:root",
        };
        let (on_server, _) = serving
            .limits()
            .await
            .expect("the rules would not be read back after the race");
        conn.close().await;
        let ips_present: Vec<&str> = on_server.iter().map(|l| l.ip.as_str()).collect();

        match (result_a, result_b) {
            (Ok(()), Ok(())) => {
                assert!(
                    ips_present.contains(&ip_a.as_str()) && ips_present.contains(&ip_b.as_str()),
                    "round {round}: both calls reported success, but the file does not hold \
                     both rules — one was silently lost. On the server: {ips_present:?}"
                );
            }
            (Ok(()), Err(LimitError::Conflict { .. })) => {
                assert!(
                    ips_present.contains(&ip_a.as_str()),
                    "round {round}: the successful call's own rule is not on the server: \
                     {ips_present:?}"
                );
            }
            (Err(LimitError::Conflict { .. }), Ok(())) => {
                assert!(
                    ips_present.contains(&ip_b.as_str()),
                    "round {round}: the successful call's own rule is not on the server: \
                     {ips_present:?}"
                );
            }
            other => panic!(
                "round {round}: an outcome outside the two acceptable ones from the task: \
                 {other:?}"
            ),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_limit_set_on_a_file_with_no_generation_line_at_all_succeeds() {
    // T600 backward compatibility. A server this application was already deployed to before
    // this change has a `vrcast-limits.conf` written by the OLD `build()` — one line short of
    // the new generation marker. The very first `limit_set` after an application upgrade must
    // not fail with a false `LimitsConflict` on such a server: "no line" reads as generation
    // zero on both sides of the check (see `read_generation`'s own doc for why), the same as
    // an absent catalogue reads as `Manifest::empty()`.
    let server = TestServer::start().expect("the container would not come up");
    lay_out_ladder(&server, "demo").expect("the quality set was not laid out");

    let conn = connect(&server).await;
    // Simulating the OLD file shape directly: no `# vrcast-generation` line, only what the
    // pre-T600 `build()` ever wrote (nothing here, since the medium has no limit yet).
    conn.exec(&format!(
        "printf '%s\\n' \\
         '# The quality-limit rules. This file belongs to VRCast Studio: it is rewritten whole' \\
         '# on every change, and anything added here by hand will be lost.' \\
         > {CONF}"
    ))
    .await
    .expect("could not lay out the pre-T600 file shape")
    .require_ok("writing the old-shaped file failed")
    .expect("writing the old-shaped file failed");
    conn.close().await;

    add_one_limit(&server, "203.0.113.50", "demo", 6_000_000)
        .await
        .expect(
            "a limit_set against a file with no generation line at all was refused as a \
             false conflict",
        );

    let conn = connect(&server).await;
    let serving = Serving {
        conn: &conn,
        video_dir: VIDEO_DIR,
        conf_path: CONF,
        main_conf: MAIN_CONF,
        serving_prefix: PREFIX,
        check_url: &format!(
            "http://{}:{}/videos/demo/master.m3u8",
            server.host(),
            server.http_port
        ),
        owner: "root:root",
    };
    let (on_server, generation) = serving
        .limits()
        .await
        .expect("the rules would not be read back");
    conn.close().await;
    assert_eq!(on_server.len(), 1);
    assert_eq!(on_server[0].ip, "203.0.113.50");
    // And the file now carries a generation line of its own, for the NEXT call to check
    // against — the upgrade path only forgives the very first call, not every call forever.
    assert_eq!(generation, 1);
}

// ---------- T603: one transaction for the whole change ----------
//
// T600 made the compare-and-swap atomic, but only the swap itself: the shortened
// descriptions were written before it, the checking and the reload after it, and a rollback
// put back a shared `.previous` without asking whose it was. These checks are about
// everything around the swap — a step of it failing, and another change landing while this
// one is still being checked.

const LOCK: &str = "/etc/caddy/vrcast-limits.conf.lock";
const SLOW: &str = "/var/lib/vrcast/videos/_slow";

/// Where the description for one ceiling on `demo` sits (T602).
pub(super) fn short_at(cap_bps: u64) -> String {
    format!("{SLOW}/demo/{cap_bps}/master.m3u8")
}

/// Everything under `_slow/`, exactly: every directory, and every file with its hash —
/// read around our own code. Two equal snapshots mean nothing under it changed.
fn slow_snapshot(server: &TestServer) -> String {
    server
        .exec_inside(&format!(
            "cd {SLOW} 2>/dev/null || {{ echo NO_SLOW; exit 0; }}; \
             find . -type d | LC_ALL=C sort; echo ---; \
             find . -type f -print0 | LC_ALL=C sort -z | xargs -0 -r sha256sum"
        ))
        .expect("the shortened descriptions would not be listed")
}

pub(super) fn good_url(server: &TestServer) -> String {
    format!(
        "http://{}:{}/videos/demo/master.m3u8",
        server.host(),
        server.http_port
    )
}

fn serving_at<'a>(conn: &'a vrcast_studio_lib::ssh::Connection, check_url: &'a str) -> Serving<'a> {
    Serving {
        conn,
        video_dir: VIDEO_DIR,
        conf_path: CONF,
        main_conf: MAIN_CONF,
        serving_prefix: PREFIX,
        check_url,
        owner: "root:root",
    }
}

/// A file's exact bytes as the container sees them, or `ABSENT` — read around our own code.
pub(super) fn contents(server: &TestServer, path: &str) -> String {
    server
        .exec_inside(&format!(
            "if [ -e '{path}' ]; then cat '{path}'; else printf ABSENT; fi"
        ))
        .expect("the file would not be read")
}

/// Whether nobody holds the transaction lock right now — asked of `flock` itself.
fn lock_is_free(server: &TestServer) -> bool {
    server
        .exec_inside(&format!(
            "flock -n -x '{LOCK}' true && echo FREE || echo HELD"
        ))
        .expect("the lock would not be probed")
        .contains("FREE")
}

/// The same, allowing the server a moment to notice a channel that was just closed.
async fn lock_frees_within(server: &TestServer, limit: Duration) -> bool {
    let started = std::time::Instant::now();
    loop {
        if lock_is_free(server) {
            return true;
        }
        if started.elapsed() > limit {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Every staged file a transaction could have left behind.
fn leftovers(server: &TestServer) -> String {
    server
        .exec_inside(&format!(
            "find /etc/caddy {VIDEO_DIR}/_slow -name '*.tmp' 2>/dev/null || true"
        ))
        .expect("the leftovers would not be listed")
        .trim()
        .to_owned()
}

/// Put a tool in front of the real one that refuses exactly when its last argument is
/// `target`, and passes everything else through. `/usr/local/bin` comes before `/usr/bin`
/// in the PATH sshd gives a command, so the application's own scripts run into it.
fn install_failing(server: &TestServer, tool: &str, target: &str) {
    server
        .exec_inside(&format!(
            "printf '%s\\n' '#!/bin/bash' \
             'for last in \"$@\"; do :; done' \
             'if [ \"$last\" = \"{target}\" ]; then echo \"{tool} refused on purpose\" >&2; exit 1; fi' \
             'exec /usr/bin/{tool} \"$@\"' > /usr/local/bin/{tool} && chmod 755 /usr/local/bin/{tool}"
        ))
        .expect("the failing tool would not go in");
}

fn remove_failing(server: &TestServer, tool: &str) {
    server
        .exec_inside(&format!("/usr/bin/rm -f /usr/local/bin/{tool}"))
        .expect("the failing tool would not come out");
}

fn a_rule(ip: &str, cap_bps: u64) -> Limit {
    Limit {
        ip: ip.to_owned(),
        slug: String::from("demo"),
        cap_bps,
        set_at: when(),
    }
}

/// The ladder the fixture lays out, read the way a viewer would.
pub(super) fn the_ladder(server: &TestServer) -> Vec<Variant> {
    let viewer = Viewer::attach(server).expect("the viewer would not attach");
    parse(
        &viewer
            .fetch("/videos/demo/master.m3u8")
            .expect("the description was not served"),
    )
    .expect("the description would not read")
}

/// A limit already in force (generation 1) with its shortened description, so a failed
/// change has something real to leave alone.
async fn one_limit_in_force(server: &TestServer, all: &[Variant]) {
    let conn = connect(server).await;
    let url = good_url(server);
    serving_at(&conn, &url)
        .apply(&[a_rule("203.0.113.7", all[1].bandwidth)], Some("demo"), 0)
        .await
        .expect("the first limit would not go on");
    conn.close().await;
}

/// A second limit on the same medium, with a ceiling of its own — the change the failure
/// checks try to make.
async fn add_a_second_limit(server: &TestServer, all: &[Variant]) -> Result<(), LimitError> {
    let conn = connect(server).await;
    let url = good_url(server);
    let outcome = serving_at(&conn, &url)
        .apply(
            &[
                a_rule("203.0.113.7", all[1].bandwidth),
                a_rule("203.0.113.8", all[2].bandwidth),
            ],
            Some("demo"),
            1,
        )
        .await;
    conn.close().await;
    outcome
}

/// What a failed change must leave: the same rules to the byte, everything under `_slow/`
/// to the byte, nothing staged lying about, and the lock free.
fn nothing_changed(server: &TestServer, conf_before: &str, slow_before: &str, what: &str) {
    assert_eq!(
        contents(server, CONF),
        conf_before,
        "{what}: the rules in force changed although the change failed"
    );
    assert_eq!(
        slow_snapshot(server),
        slow_before,
        "{what}: the shortened descriptions were not put back as they were"
    );
    assert_eq!(
        leftovers(server),
        "",
        "{what}: staged files were left behind"
    );
    assert!(lock_is_free(server), "{what}: the lock was left held");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_refused_move_into_place_is_an_error_and_changes_nothing() {
    // T603 (a). The swap's `mv` was not checked and `echo OK` came regardless: a refused
    // move told the caller the rule was in force when nothing had been put in place, and
    // the shortened description written ahead of it stayed changed.
    let server = TestServer::start().expect("the container would not come up");
    lay_out_ladder(&server, "demo").expect("the quality set was not laid out");
    let all = the_ladder(&server);
    one_limit_in_force(&server, &all).await;
    let conf_before = contents(&server, CONF);
    let slow_before = slow_snapshot(&server);

    install_failing(&server, "mv", CONF);
    let outcome = add_a_second_limit(&server, &all).await;
    remove_failing(&server, "mv");

    assert!(
        outcome.is_err(),
        "the rules could not be moved into place and the caller was told they were"
    );
    assert!(
        !matches!(outcome, Err(LimitError::Conflict { .. })),
        "a refused move is not somebody else's change: {outcome:?}"
    );
    nothing_changed(&server, &conf_before, &slow_before, "refused mv");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_refused_backup_means_the_new_rules_never_go_in() {
    // T603 (b). A change is never put in place without what it replaces kept first.
    let server = TestServer::start().expect("the container would not come up");
    lay_out_ladder(&server, "demo").expect("the quality set was not laid out");
    let all = the_ladder(&server);
    one_limit_in_force(&server, &all).await;
    let conf_before = contents(&server, CONF);
    let slow_before = slow_snapshot(&server);

    install_failing(&server, "cp", &format!("{CONF}.previous"));
    let outcome = add_a_second_limit(&server, &all).await;
    remove_failing(&server, "cp");

    assert!(
        outcome.is_err(),
        "the previous rules could not be kept and the new ones went in anyway"
    );
    nothing_changed(&server, &conf_before, &slow_before, "refused backup");
}

/// Somewhere that takes a connection and never answers it: a check against it runs into
/// `ANSWER_TIMEOUT` the way a serving that stopped answering would.
async fn a_silent_address() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("no local port for the silent address");
    let port = listener.local_addr().expect("no local address").port();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });
    format!("http://127.0.0.1:{port}/never")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_rollback_never_undoes_a_change_that_landed_after_it() {
    // T603 (c). A puts its rule in and waits on a serving that does not answer; B, starting
    // while A is still waiting, puts its own rule in. Before the fix A's rollback moved the
    // shared `.previous` back — by then the copy B had kept of A's own file — so B's rule
    // vanished, A's came back to life, and the generation went backwards.
    let server = TestServer::start().expect("the container would not come up");
    lay_out_ladder(&server, "demo").expect("the quality set was not laid out");
    let all = the_ladder(&server);
    one_limit_in_force(&server, &all).await;
    let silent = a_silent_address().await;
    let good = good_url(&server);

    let mut b_rules: Vec<String> = Vec::new();
    for round in 0..3u32 {
        let slow_before = slow_snapshot(&server);
        let ip_a = format!("198.51.100.{}", 10 + round);
        let ip_b = format!("203.0.113.{}", 100 + round);

        let conn_read = connect(&server).await;
        let (_, generation_before) = serving_at(&conn_read, &good)
            .limits()
            .await
            .expect("the rules would not be read");

        let stop = std::sync::atomic::AtomicBool::new(false);
        let a = async {
            let conn = connect(&server).await;
            let serving = serving_at(&conn, &silent);
            let (mut rules, generation) = serving.limits().await.expect("A could not read");
            rules.push(a_rule(&ip_a, all[2].bandwidth));
            let started = std::time::Instant::now();
            let outcome = serving.apply(&rules, Some("demo"), generation).await;
            eprintln!(
                "round {round}: A's whole change took {:.1}s and ended {outcome:?}",
                started.elapsed().as_secs_f64()
            );
            conn.close().await;
            outcome
        };
        let b = async {
            // Late enough that A's file is already in place and A is waiting on the check.
            tokio::time::sleep(Duration::from_secs(4)).await;
            let conn = connect(&server).await;
            let serving = serving_at(&conn, &good);
            let mut outcome = None;
            for _ in 0..40 {
                let (mut rules, generation) = serving.limits().await.expect("B could not read");
                rules.retain(|l| l.ip != ip_b);
                rules.push(a_rule(&ip_b, all[1].bandwidth));
                match serving.apply(&rules, Some("demo"), generation).await {
                    Err(LimitError::Conflict { .. }) => {
                        tokio::time::sleep(Duration::from_millis(300)).await;
                    }
                    other => {
                        outcome = Some(other);
                        break;
                    }
                }
            }
            conn.close().await;
            outcome.expect("B never got past the conflicts")
        };
        // Every generation anybody could have read without the lock, in order.
        let watch = async {
            let conn = connect(&server).await;
            let serving = serving_at(&conn, &good);
            let mut seen: Vec<u64> = Vec::new();
            while !stop.load(std::sync::atomic::Ordering::SeqCst) {
                if let Ok((_, g)) = serving.limits().await {
                    if seen.last() != Some(&g) {
                        seen.push(g);
                    }
                }
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
            conn.close().await;
            seen
        };
        let both = async {
            let r = tokio::join!(a, b);
            stop.store(true, std::sync::atomic::Ordering::SeqCst);
            r
        };
        let ((result_a, result_b), seen) = tokio::join!(both, watch);

        assert!(
            result_a.is_err(),
            "round {round}: A's check could not have passed: {result_a:?}"
        );
        result_b.unwrap_or_else(|e| panic!("round {round}: B failed: {e:?}"));
        b_rules.push(ip_b.clone());

        let (rules, generation_after) = serving_at(&conn_read, &good)
            .limits()
            .await
            .expect("the rules would not be read back");
        conn_read.close().await;
        let ips: Vec<&str> = rules.iter().map(|l| l.ip.as_str()).collect();
        for kept in &b_rules {
            assert!(
                ips.contains(&kept.as_str()),
                "round {round}: B's successful rule {kept} was lost: {ips:?}"
            );
        }
        assert!(
            !ips.contains(&ip_a.as_str()),
            "round {round}: A's rolled-back rule is in force: {ips:?}"
        );
        assert!(
            ips.contains(&"203.0.113.7"),
            "round {round}: the rule from before was lost: {ips:?}"
        );
        assert!(
            generation_after > generation_before,
            "round {round}: generation went from {generation_before} to {generation_after}"
        );
        assert!(
            seen.windows(2).all(|w| w[0] < w[1]),
            "round {round}: the generation did not only grow — a reader could have been \
             handed an old number again: {seen:?}"
        );
        assert_eq!(
            slow_snapshot(&server),
            slow_before,
            "round {round}: A's shortened description (or its directory) stayed after its \
             rollback"
        );
        assert_eq!(leftovers(&server), "", "round {round}: leftovers");
        assert!(
            lock_frees_within(&server, Duration::from_secs(5)).await,
            "round {round}: the lock was left held"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_change_that_cannot_get_the_lock_touches_nothing_and_says_so() {
    // T603 (d). Somebody else holds the lock for longer than a change waits. Before the fix
    // the shortened description was written before the lock was even asked for, and a
    // timed-out lock left it changed and the staged rules lying next to the real ones.
    let server = TestServer::start().expect("the container would not come up");
    lay_out_ladder(&server, "demo").expect("the quality set was not laid out");
    let all = the_ladder(&server);
    one_limit_in_force(&server, &all).await;
    let conf_before = contents(&server, CONF);
    let slow_before = slow_snapshot(&server);

    // Held by the test, in the container, around our own code: detached so `docker exec`
    // returns, and for longer than any change waits.
    server
        .exec_inside(&format!(
            "setsid nohup flock -x '{LOCK}' sleep 400 >/dev/null 2>&1 < /dev/null & \
             echo $! > /tmp/zz-t603-holder.pid"
        ))
        .expect("the test could not take the lock");
    let mut held = false;
    for _ in 0..50 {
        if !lock_is_free(&server) {
            held = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(held, "the test's own lock never took hold");

    let started = std::time::Instant::now();
    let outcome = add_a_second_limit(&server, &all).await;
    eprintln!(
        "the change gave up on the lock after {:.1}s: {outcome:?}",
        started.elapsed().as_secs_f64()
    );
    assert!(
        matches!(outcome, Err(LimitError::Busy)),
        "a change that never got the lock must say another change is in progress: {outcome:?}"
    );

    // By the process group `setsid` made, not by a pattern: `pkill -f` would match the very
    // shell running it, whose command line holds the same words.
    server
        .exec_inside("kill -- -\"$(cat /tmp/zz-t603-holder.pid)\"; rm -f /tmp/zz-t603-holder.pid")
        .expect("the test's lock would not be let go");
    assert!(
        lock_frees_within(&server, Duration::from_secs(10)).await,
        "the lock was not free once the test let go of it"
    );
    nothing_changed(&server, &conf_before, &slow_before, "lock timeout");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn clearing_keeps_the_description_while_a_rule_still_points_at_it() {
    // T603, point 2. The description is removed inside the same transaction as the rule,
    // and only when no rule that stays names the medium. Two viewers limited on one medium:
    // clearing one leaves the description for the other; clearing the second removes it.
    let server = TestServer::start().expect("the container would not come up");
    lay_out_ladder(&server, "demo").expect("the quality set was not laid out");
    let all = the_ladder(&server);
    one_limit_in_force(&server, &all).await;
    add_a_second_limit(&server, &all)
        .await
        .expect("the second limit would not go on");
    let short_of_seven = contents(&server, &short_at(all[1].bandwidth));
    assert_ne!(short_of_seven, "ABSENT");
    assert_ne!(contents(&server, &short_at(all[2].bandwidth)), "ABSENT");

    let conn = connect(&server).await;
    let url = good_url(&server);
    let serving = serving_at(&conn, &url);

    let (rules, generation) = serving.limits().await.expect("the rules would not read");
    assert_eq!(generation, 2);
    let remaining: Vec<Limit> = rules
        .into_iter()
        .filter(|l| l.ip != "203.0.113.8")
        .collect();
    serving
        .apply(&remaining, None, generation)
        .await
        .expect("the first of two limits would not come off");
    assert_eq!(
        contents(&server, &short_at(all[1].bandwidth)),
        short_of_seven,
        "the description went while a rule still pointed at it"
    );
    assert_eq!(
        contents(&server, &short_at(all[2].bandwidth)),
        "ABSENT",
        "the description of a ceiling no rule names any more stayed"
    );

    let (rules, generation) = serving.limits().await.expect("the rules would not read");
    assert_eq!(generation, 3);
    let remaining: Vec<Limit> = rules
        .into_iter()
        .filter(|l| l.ip != "203.0.113.7")
        .collect();
    assert!(remaining.is_empty());
    serving
        .apply(&remaining, None, generation)
        .await
        .expect("the last limit would not come off");
    conn.close().await;

    assert_eq!(
        slow_snapshot(&server),
        ".\n---\n",
        "something stayed under _slow/ after the last rule went"
    );
    assert_eq!(leftovers(&server), "");
    assert!(lock_is_free(&server));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_clear_whose_check_fails_brings_the_description_back() {
    // T603, point 2: "restored on ANY failure" includes a removal. The serving stops
    // answering after the last rule is taken off; the rule comes back, and so does the
    // description it points at — a rule left pointing at nothing would serve the limited
    // viewer nothing at all.
    let server = TestServer::start().expect("the container would not come up");
    lay_out_ladder(&server, "demo").expect("the quality set was not laid out");
    let all = the_ladder(&server);
    one_limit_in_force(&server, &all).await;
    let slow_before = slow_snapshot(&server);

    let silent = a_silent_address().await;
    let conn = connect(&server).await;
    let outcome = serving_at(&conn, &silent).apply(&[], None, 1).await;
    conn.close().await;
    assert!(
        outcome.is_err(),
        "the check could not have passed: {outcome:?}"
    );

    let conn = connect(&server).await;
    let url = good_url(&server);
    let (rules, generation) = serving_at(&conn, &url)
        .limits()
        .await
        .expect("the rules would not read");
    conn.close().await;
    assert_eq!(rules.len(), 1, "the rule did not come back: {rules:?}");
    assert_eq!(
        generation, 3,
        "the rollback must write a new generation (1 → 2 by the clear → 3 by its undo)"
    );
    // Since T602 nothing is removed before the check passes, so the description the rule
    // points at never went in the first place.
    assert_eq!(
        slow_snapshot(&server),
        slow_before,
        "the description the rule points at did not stay"
    );
    assert_eq!(leftovers(&server), "");
    assert!(lock_frees_within(&server, Duration::from_secs(5)).await);
}

// ---------- T602: one shortened description per ceiling ----------
//
// Every rule of a medium used to rewrite onto one `_slow/<slug>/master.m3u8`, and each
// `limit_set` wrote the shortened description of its own request there — so the last one
// written was what every limited viewer of that medium got, whatever their own ceiling.

/// Put `ip`'s limit on `slug` the way `commands/limits.rs::api::limit_set` does: read the
/// rules and their generation, replace this address's rule for this medium, apply.
pub(super) async fn put_limit(
    server: &TestServer,
    ip: &str,
    slug: &str,
    cap_bps: u64,
    check_url: &str,
) -> Result<(), LimitError> {
    let conn = connect(server).await;
    let serving = serving_at(&conn, check_url);
    let outcome = async {
        let (mut rules, generation) = serving.limits().await?;
        rules.retain(|l| !(l.ip == ip && l.slug == slug));
        rules.push(Limit {
            ip: ip.to_owned(),
            slug: slug.to_owned(),
            cap_bps,
            set_at: when(),
        });
        serving.apply(&rules, Some(slug), generation).await
    }
    .await;
    conn.close().await;
    outcome
}

/// Take `ip`'s limit on `slug` off the way `commands/limits.rs::api::limit_clear` does.
async fn take_limit_off(
    server: &TestServer,
    ip: &str,
    slug: &str,
    check_url: &str,
) -> Result<(), LimitError> {
    let conn = connect(server).await;
    let serving = serving_at(&conn, check_url);
    let outcome = async {
        let (mut rules, generation) = serving.limits().await?;
        rules.retain(|l| !(l.ip == ip && l.slug == slug));
        serving.apply(&rules, None, generation).await
    }
    .await;
    conn.close().await;
    outcome
}

/// The rungs `viewer` is offered for `slug`, heaviest first, by bandwidth.
pub(super) fn offered(viewer: &Viewer, slug: &str) -> Vec<u64> {
    let text = viewer
        .fetch(&format!("/videos/{slug}/master.m3u8"))
        .unwrap_or_else(|e| panic!("{} was served nothing for {slug}: {e}", viewer.ip()));
    let variants =
        parse(&text).unwrap_or_else(|e| panic!("what was served would not read ({e:?}):\n{text}"));
    for v in &variants {
        assert!(
            v.path.starts_with(&format!("/videos/{slug}/")) || !v.path.starts_with('/'),
            "a rung of {slug} points somewhere else: {}",
            v.path
        );
    }
    variants.iter().map(|v| v.bandwidth).collect()
}

/// What somebody with no limit is offered — asked from outside the server's network.
pub(super) async fn offered_to_everybody(server: &TestServer, slug: &str) -> Vec<u64> {
    let text = reqwest::get(&format!(
        "http://{}:{}/videos/{slug}/master.m3u8",
        server.host(),
        server.http_port
    ))
    .await
    .expect("the serving would not answer")
    .text()
    .await
    .expect("no answer body");
    parse(&text)
        .expect("the full description would not read")
        .iter()
        .map(|v| v.bandwidth)
        .collect()
}

pub(super) fn exists(server: &TestServer, path: &str) -> bool {
    server
        .exec_inside(&format!("test -e '{path}' && echo YES || echo NO"))
        .expect("the server would not answer")
        .contains("YES")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_viewers_of_one_medium_keep_their_own_ceilings() {
    // T602 (a), the bug itself. A is capped to the lightest rung, B to two. A is set first,
    // B second — and A must still get only the lightest after B's limit went in.
    let server = TestServer::start().expect("the container would not come up");
    lay_out_ladder(&server, "demo").expect("the quality set was not laid out");
    let all = the_ladder(&server);
    let a = Viewer::attach(&server).expect("A would not attach");
    let b = Viewer::attach(&server).expect("B would not attach");
    assert_ne!(a.ip(), b.ip(), "the two viewers share an address");
    let url = good_url(&server);
    let cap_a = all[2].bandwidth;
    let cap_b = all[1].bandwidth;

    put_limit(&server, a.ip(), "demo", cap_a, &url)
        .await
        .expect("A's limit would not go on");
    assert_eq!(offered(&a, "demo"), vec![all[2].bandwidth]);

    put_limit(&server, b.ip(), "demo", cap_b, &url)
        .await
        .expect("B's limit would not go on");
    assert_eq!(
        offered(&b, "demo"),
        vec![all[1].bandwidth, all[2].bandwidth],
        "B was not given their own two rungs"
    );
    assert_eq!(
        offered(&a, "demo"),
        vec![all[2].bandwidth],
        "A was handed B's set once B's limit went in — the last limit written decides for \
         everybody"
    );
    assert_eq!(
        offered_to_everybody(&server, "demo").await.len(),
        3,
        "somebody with no limit lost rungs"
    );

    assert!(
        exists(&server, &short_at(cap_a)),
        "A's description is not on disk"
    );
    assert!(
        exists(&server, &short_at(cap_b)),
        "B's description is not on disk"
    );

    // The caching rule reaches a limited viewer's description at its new place, one level
    // deeper: a description cached for thirty days would make lifting the limit meaningless.
    let headers = a
        .headers("/videos/demo/master.m3u8")
        .expect("A's description would not be served");
    assert!(
        headers.to_lowercase().contains("cache-control: no-cache"),
        "a limited viewer's description is cached like a segment:\n{headers}"
    );
    assert_eq!(leftovers(&server), "");
    assert!(lock_is_free(&server));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn taking_one_viewers_limit_off_leaves_the_other_alone() {
    // T602 (b). In the state of (a), A's limit comes off: A gets everything, B's answer
    // and B's file stay exactly as they were, and A's ceiling's directory is gone.
    let server = TestServer::start().expect("the container would not come up");
    lay_out_ladder(&server, "demo").expect("the quality set was not laid out");
    let all = the_ladder(&server);
    let a = Viewer::attach(&server).expect("A would not attach");
    let b = Viewer::attach(&server).expect("B would not attach");
    let url = good_url(&server);
    let cap_a = all[2].bandwidth;
    let cap_b = all[1].bandwidth;
    put_limit(&server, a.ip(), "demo", cap_a, &url)
        .await
        .expect("A's limit would not go on");
    put_limit(&server, b.ip(), "demo", cap_b, &url)
        .await
        .expect("B's limit would not go on");

    let b_answer_before = b
        .fetch("/videos/demo/master.m3u8")
        .expect("B was served nothing");
    let b_file_before = contents(&server, &short_at(cap_b));

    take_limit_off(&server, a.ip(), "demo", &url)
        .await
        .expect("A's limit would not come off");

    assert_eq!(
        offered(&a, "demo").len(),
        3,
        "A did not get every rung back"
    );
    assert_eq!(
        b.fetch("/videos/demo/master.m3u8")
            .expect("B was served nothing"),
        b_answer_before,
        "B's answer changed when A's limit came off"
    );
    assert_eq!(
        contents(&server, &short_at(cap_b)),
        b_file_before,
        "B's description changed when A's limit came off"
    );
    assert!(
        !exists(&server, &format!("{SLOW}/demo/{cap_a}")),
        "A's ceiling's directory stayed after the last rule for it went"
    );
    assert_eq!(leftovers(&server), "");
    assert!(lock_is_free(&server));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_viewers_with_one_ceiling_share_a_file_until_both_are_gone() {
    // T602 (c).
    let server = TestServer::start().expect("the container would not come up");
    lay_out_ladder(&server, "demo").expect("the quality set was not laid out");
    let all = the_ladder(&server);
    let a = Viewer::attach(&server).expect("A would not attach");
    let b = Viewer::attach(&server).expect("B would not attach");
    let url = good_url(&server);
    let cap = all[1].bandwidth;
    put_limit(&server, a.ip(), "demo", cap, &url)
        .await
        .expect("A's limit would not go on");
    put_limit(&server, b.ip(), "demo", cap, &url)
        .await
        .expect("B's limit would not go on");

    let listed = server
        .exec_inside(&format!("cd {SLOW} && find . -type f | LC_ALL=C sort"))
        .expect("the descriptions would not be listed");
    assert_eq!(
        listed.trim(),
        format!("./demo/{cap}/master.m3u8"),
        "two viewers with one ceiling did not share one file"
    );

    take_limit_off(&server, a.ip(), "demo", &url)
        .await
        .expect("A's limit would not come off");
    assert!(
        exists(&server, &short_at(cap)),
        "the shared file went while B's rule still points at it"
    );
    assert_eq!(offered(&b, "demo").len(), 2, "B lost their limit");
    assert_eq!(
        offered(&a, "demo").len(),
        3,
        "A kept a limit that was taken off"
    );

    take_limit_off(&server, b.ip(), "demo", &url)
        .await
        .expect("B's limit would not come off");
    assert!(
        !exists(&server, &format!("{SLOW}/demo")),
        "_slow/demo/ stayed"
    );
    assert!(exists(&server, SLOW), "_slow/ itself was removed");
    assert_eq!(offered(&b, "demo").len(), 3);
    assert_eq!(leftovers(&server), "");
    assert!(lock_is_free(&server));
}

/// Lay out the state a client built before T602 leaves (T603's): a rules file at
/// `generation` whose rules rewrite onto `_slow/<slug>/master.m3u8`, those files, and the
/// serving reloaded onto it.
fn lay_out_pre_t602(server: &TestServer, rules: &[Limit], generation: u64, all: &[Variant]) {
    let mut text = vrcast_studio_lib::domain::limits_conf::build(rules, PREFIX, generation);
    for rule in rules {
        text = text.replace(
            &format!("/_slow/{}/{}/master.m3u8", rule.slug, rule.cap_bps),
            &format!("/_slow/{}/master.m3u8", rule.slug),
        );
        assert!(
            text.contains(&format!("{PREFIX}/_slow/{}/master.m3u8\n", rule.slug)),
            "the old rule shape was not made:\n{text}"
        );
    }
    write_inside(server, CONF, &text);
    for rule in rules {
        let short = shorten(all, rule.cap_bps, PREFIX, &rule.slug);
        server
            .exec_inside(&format!("mkdir -p {SLOW}/{}", rule.slug))
            .expect("the old directory would not be made");
        write_inside(
            server,
            &format!("{SLOW}/{}/master.m3u8", rule.slug),
            &short.text,
        );
    }
    server
        .exec_inside(&format!(
            "caddy reload --config {MAIN_CONF} --adapter caddyfile 2>&1"
        ))
        .expect("the serving would not take the old rules");
}

fn write_inside(server: &TestServer, path: &str, text: &str) {
    server
        .exec_inside(&format!(
            "cat > '{path}' <<'VRCAST_T602_EOF'\n{text}VRCAST_T602_EOF"
        ))
        .expect("the file would not be written");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_first_change_moves_every_rule_off_the_old_shared_files() {
    // T602 (d). A server a pre-T602 client changed last: generation 5, rules for A on
    // `demo` and C on `other` rewriting onto the old one-per-medium files. The first change
    // by this client — B's limit on `demo` — puts every rule on the new files, removes the
    // old ones, and nobody is left without their set.
    let server = TestServer::start().expect("the container would not come up");
    lay_out_ladder(&server, "demo").expect("the quality set was not laid out");
    lay_out_ladder(&server, "other").expect("the second quality set was not laid out");
    let all = the_ladder(&server);
    let a = Viewer::attach(&server).expect("A would not attach");
    let b = Viewer::attach(&server).expect("B would not attach");
    let c = Viewer::attach(&server).expect("C would not attach");
    let cap_a = all[2].bandwidth;
    let cap_b = all[1].bandwidth;
    let cap_c = all[1].bandwidth;
    let old = [
        Limit {
            ip: a.ip().to_owned(),
            slug: String::from("demo"),
            cap_bps: cap_a,
            set_at: when(),
        },
        Limit {
            ip: c.ip().to_owned(),
            slug: String::from("other"),
            cap_bps: cap_c,
            set_at: when(),
        },
    ];
    lay_out_pre_t602(&server, &old, 5, &all);
    // The old state really is in force: A and C are limited through the old files.
    assert_eq!(offered(&a, "demo"), vec![all[2].bandwidth]);
    assert_eq!(offered(&c, "other").len(), 2);

    put_limit(&server, b.ip(), "demo", cap_b, &good_url(&server))
        .await
        .expect("the first change after the old client would not go in");

    assert!(
        !exists(&server, &format!("{SLOW}/demo/master.m3u8")),
        "the old file of the medium being changed stayed"
    );
    assert!(
        !exists(&server, &format!("{SLOW}/other/master.m3u8")),
        "the old file of another medium stayed, and no rule reaches it any more"
    );
    assert_eq!(
        offered(&a, "demo"),
        vec![all[2].bandwidth],
        "A lost their set"
    );
    assert_eq!(
        offered(&b, "demo"),
        vec![all[1].bandwidth, all[2].bandwidth],
        "B did not get their set"
    );
    assert_eq!(
        offered(&c, "other"),
        vec![all[1].bandwidth, all[2].bandwidth],
        "C, on a medium nobody touched, lost their set"
    );
    assert_eq!(offered_to_everybody(&server, "other").await.len(), 3);

    let conn = connect(&server).await;
    let url = good_url(&server);
    let (rules, generation) = serving_at(&conn, &url)
        .limits()
        .await
        .expect("the rules would not read");
    conn.close().await;
    assert_eq!(generation, 6);
    assert_eq!(rules.len(), 3);
    let conf = contents(&server, CONF);
    assert!(
        !conf.contains("/_slow/demo/master.m3u8") && !conf.contains("/_slow/other/master.m3u8"),
        "a rule still points at an old file:\n{conf}"
    );
    assert_eq!(leftovers(&server), "");
    assert!(lock_is_free(&server));
}

/// The rules as they read without the generation line — what a rollback brings back
/// exactly (the number itself only grows, T603).
fn without_generation(text: &str) -> String {
    text.lines()
        .filter(|l| !l.starts_with("# vrcast-generation "))
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_change_that_fails_its_check_leaves_everything_under_slow_as_it_was() {
    // T602 (e). The pre-T602 state of (d) — old files, a rule of another ceiling — and a
    // change whose check fails. The change had made new directories and written new files
    // for every rule before its rules went in; all of it goes, nothing old was removed, and
    // the old rules keep serving through the old files.
    let server = TestServer::start().expect("the container would not come up");
    lay_out_ladder(&server, "demo").expect("the quality set was not laid out");
    lay_out_ladder(&server, "other").expect("the second quality set was not laid out");
    let all = the_ladder(&server);
    let a = Viewer::attach(&server).expect("A would not attach");
    let old = [
        Limit {
            ip: a.ip().to_owned(),
            slug: String::from("demo"),
            cap_bps: all[2].bandwidth,
            set_at: when(),
        },
        Limit {
            ip: String::from("203.0.113.30"),
            slug: String::from("other"),
            cap_bps: all[1].bandwidth,
            set_at: when(),
        },
    ];
    lay_out_pre_t602(&server, &old, 5, &all);
    let conf_before = contents(&server, CONF);
    let slow_before = slow_snapshot(&server);

    let silent = a_silent_address().await;
    let outcome = put_limit(&server, "203.0.113.20", "demo", all[1].bandwidth, &silent).await;
    // `RollbackFailed` rather than `ServingStopped`: the silent address does not answer
    // after the rollback either, so the rollback's own check fails too. What matters here
    // is what is on disk afterwards, checked byte for byte below.
    assert!(
        matches!(
            outcome,
            Err(LimitError::ServingStopped | LimitError::RollbackFailed(_))
        ),
        "the check could not have passed: {outcome:?}"
    );

    let conf_after = contents(&server, CONF);
    assert_eq!(
        without_generation(&conf_after),
        without_generation(&conf_before),
        "the rules did not come back as they were"
    );
    assert_eq!(
        vrcast_studio_lib::domain::limits_conf::read_generation(&conf_after),
        7,
        "5 before, 6 by the failed change, 7 by putting the old rules back"
    );
    assert_eq!(
        slow_snapshot(&server),
        slow_before,
        "something under _slow/ is not as it was"
    );
    assert_eq!(leftovers(&server), "");
    assert!(lock_frees_within(&server, Duration::from_secs(5)).await);
    assert_eq!(
        offered(&a, "demo"),
        vec![all[2].bandwidth],
        "A lost their set through the failed change"
    );

    // And taking a limit off whose check fails, in the new layout: nothing removed either.
    let url = good_url(&server);
    put_limit(&server, "203.0.113.20", "demo", all[1].bandwidth, &url)
        .await
        .expect("a sound change would not go in");
    let conf_before = contents(&server, CONF);
    let slow_before = slow_snapshot(&server);
    let outcome = take_limit_off(&server, a.ip(), "demo", &silent).await;
    assert!(
        outcome.is_err(),
        "the check could not have passed: {outcome:?}"
    );
    assert_eq!(
        without_generation(&contents(&server, CONF)),
        without_generation(&conf_before)
    );
    assert_eq!(slow_snapshot(&server), slow_before);
    assert_eq!(leftovers(&server), "");
    assert!(lock_frees_within(&server, Duration::from_secs(5)).await);
    assert_eq!(offered(&a, "demo"), vec![all[2].bandwidth]);
}

// ---------- T618: the lock is renewed for as long as the change is alive ----------
//
// T603's holder was `timeout 300 cat`: a fixed lease. One command of a change may run up to
// `EXEC_CEILING` (600 s), so a live change could lose its lock in the middle — another took
// it and wrote over the first, or the first one's rollback was refused (`LOST_LOCK`) and its
// failed rules stayed in force (QA-19 №3).

/// How long the web server's own checking is made to take — longer than T603's lease.
const SLOW_VALIDATE_S: u64 = 330;

/// Put a `caddy` in front of the real one whose `validate` takes `SLOW_VALIDATE_S` and then
/// refuses; everything else passes through. The real one is in `/usr/local/bin`, so this one
/// goes into `/usr/local/sbin`, which comes first in the PATH sshd gives a command.
fn install_slow_refusing_validate(server: &TestServer) {
    server
        .exec_inside(&format!(
            "printf '%s\\n' '#!/bin/bash' \
             'if [ \"$1\" = validate ]; then sleep {SLOW_VALIDATE_S}; echo \"validate refused on purpose\"; exit 1; fi' \
             'exec /usr/local/bin/caddy \"$@\"' > /usr/local/sbin/caddy && chmod 755 /usr/local/sbin/caddy"
        ))
        .expect("the slow caddy would not go in");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_change_longer_than_the_old_lease_keeps_its_lock_to_the_end_of_its_rollback() {
    // T618. A's checking takes 330 s and fails. All that time B keeps trying and is turned
    // away (`Busy` — `LIMITS_CONFLICT` to the caller) — including across the 300 s mark,
    // where the old lease let B in to write over A. Then A's rollback runs under the lock
    // it still holds, and succeeds.
    let server = TestServer::start().expect("the container would not come up");
    lay_out_ladder(&server, "demo").expect("the quality set was not laid out");
    let all = the_ladder(&server);
    one_limit_in_force(&server, &all).await;
    let conf_before = contents(&server, CONF);
    let slow_before = slow_snapshot(&server);
    install_slow_refusing_validate(&server);
    {
        let conn = connect(&server).await;
        let which = conn
            .exec("command -v caddy")
            .await
            .expect("the server would not answer");
        conn.close().await;
        assert_eq!(
            which.trimmed(),
            "/usr/local/sbin/caddy",
            "the slow caddy is not the one a command finds"
        );
    }

    let started = std::time::Instant::now();
    let a_done = std::sync::atomic::AtomicBool::new(false);
    let a = async {
        let outcome = add_a_second_limit(&server, &all).await;
        let took = started.elapsed();
        // What A's rollback left, before anybody else may take the lock after it.
        let conf = contents(&server, CONF);
        a_done.store(true, std::sync::atomic::Ordering::SeqCst);
        (outcome, took, conf)
    };
    let b = async {
        // A's rules are in place and A is inside `caddy validate` by then.
        tokio::time::sleep(Duration::from_secs(5)).await;
        let url = good_url(&server);
        let mut tries: Vec<(Duration, Duration, String)> = Vec::new();
        while !a_done.load(std::sync::atomic::Ordering::SeqCst) {
            let from = started.elapsed();
            let outcome = put_limit(&server, "203.0.113.99", "demo", all[1].bandwidth, &url).await;
            let to = started.elapsed();
            eprintln!(
                "B tried from {:.1}s to {:.1}s: {outcome:?}",
                from.as_secs_f64(),
                to.as_secs_f64()
            );
            let got_in = !matches!(outcome, Err(LimitError::Busy));
            tries.push((from, to, format!("{outcome:?}")));
            if got_in {
                break;
            }
        }
        tries
    };
    let ((outcome_a, took_a, conf_after), tries) = tokio::join!(a, b);
    server
        .exec_inside("/usr/bin/rm -f /usr/local/sbin/caddy")
        .expect("the slow caddy would not come out");
    eprintln!(
        "A took {:.1}s and ended {outcome_a:?}",
        took_a.as_secs_f64()
    );

    assert!(
        took_a > Duration::from_secs(300),
        "A was not longer than the old lease: {took_a:?}"
    );
    match &outcome_a {
        Err(LimitError::ValidateFailed(said)) => assert!(said.contains("refused on purpose")),
        other => panic!(
            "A's rollback did not go through under its own lock (LOST_LOCK would read as \
             RollbackFailed): {other:?}"
        ),
    }
    // Every try of B that ended while A was still at it was turned away, and one of them
    // was waiting across the 300 s mark.
    for (from, to, said) in &tries {
        if *to < took_a {
            assert_eq!(
                said, "Err(Busy)",
                "B got in while A still held the lock ({from:?}–{to:?})"
            );
        }
    }
    assert!(
        tries.iter().any(|(from, to, _)| *from < Duration::from_secs(300)
            && *to > Duration::from_secs(300)),
        "no try of B spanned the moment the old lease would have run out: {tries:?}"
    );

    // What A's rollback left: the rules as before (under a new number), nothing of A's new
    // rule or of B's, `_slow/` to the byte, nothing staged, the lock free.
    assert_eq!(
        without_generation(&conf_after),
        without_generation(&conf_before)
    );
    assert!(!conf_after.contains("203.0.113.99"));
    assert!(!conf_after.contains("203.0.113.8"));
    // B's last try — the one that got the lock once A let go — read the rules while A's
    // were still in place, so it must have been refused as a conflict, not written.
    if let Some((_, _, said)) = tries.last() {
        assert!(
            said == "Err(Busy)" || said.starts_with("Err(Conflict"),
            "B wrote over the rules A was rolling back: {said}"
        );
    }
    assert_eq!(slow_snapshot(&server), slow_before);
    assert_eq!(leftovers(&server), "");
    assert!(lock_frees_within(&server, Duration::from_secs(20)).await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_holder_that_hears_nothing_lets_the_lock_go_after_the_silence() {
    // T618. A client that took the lock and then went silent without its channel closing —
    // a half-open connection, or a client frozen in place. The very holder the application
    // runs, on a real SSH channel of ours that stays open and never says a word: the server
    // must let the lock go after `LOCK_SILENCE`, not hold it for ever and not five minutes.
    use vrcast_studio_lib::server::limits::{holder_command, LOCK_SILENCE};

    let server = TestServer::start().expect("the container would not come up");
    lay_out_ladder(&server, "demo").expect("the quality set was not laid out");
    let all = the_ladder(&server);

    let conn = connect(&server).await;
    let started = std::time::Instant::now();
    let silent = async {
        let out = conn
            .exec_with_timeout(
                &holder_command("silentclient", LOCK),
                LOCK_SILENCE + Duration::from_secs(60),
            )
            .await;
        (out, started.elapsed())
    };
    let watch = async {
        // Held while the silence lasts...
        tokio::time::sleep(Duration::from_secs(5)).await;
        let early = lock_is_free(&server);
        tokio::time::sleep(LOCK_SILENCE - Duration::from_secs(15)).await;
        let late = lock_is_free(&server);
        // ...and a change started now, while the silent client still holds the lock, gets
        // it once the silence has run out — well within its own wait.
        let b_from = started.elapsed();
        let b = put_limit(
            &server,
            "203.0.113.60",
            "demo",
            all[1].bandwidth,
            &good_url(&server),
        )
        .await;
        (early, late, b_from, b, started.elapsed())
    };
    let ((out, freed_at), (early, late, b_from, b, b_to)) = tokio::join!(silent, watch);
    conn.close().await;
    eprintln!(
        "the silent holder ended after {:.1}s; B from {:.1}s to {:.1}s: {b:?}",
        freed_at.as_secs_f64(),
        b_from.as_secs_f64(),
        b_to.as_secs_f64()
    );

    let out = out.expect("the silent holder's channel failed");
    assert!(
        out.stdout.starts_with("LOCKED "),
        "the silent client never had the lock: {out:?}"
    );
    assert!(
        !early && !late,
        "the lock was let go while the silence lasted"
    );
    assert!(
        freed_at >= LOCK_SILENCE - Duration::from_secs(1)
            && freed_at <= LOCK_SILENCE + Duration::from_secs(15),
        "the lock was let go after {freed_at:?}, not after the silence ({LOCK_SILENCE:?})"
    );
    b.expect("a change behind a silent client did not get the lock after the silence");
    assert!(b_to >= freed_at, "B got in before the silent holder let go");
    assert!(lock_is_free(&server));
    assert_eq!(leftovers(&server), "");
}
