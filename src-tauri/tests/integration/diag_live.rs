//! T321 — diagnosis against a real container.
//!
//! Three questions, and each is one the pure logic cannot answer for itself, because each
//! rests on what a real machine actually says:
//!
//! - **a stopped serving must be seen, and named.** "Something is down" and "caddy is down"
//!   send a person to different places, and only one of them is a place;
//! - **the slow viewer of the fixture must come back as a viewer whose link is short**, not
//!   as a server in trouble. This is the whole method: the server is asked first whether it
//!   is asleep, and when it is, the answer is not about the server;
//! - **a viewer with a full buffer must not be called starving.** Their requests come in
//!   bursts with long gaps between them, which is exactly what a stall looks like to anything
//!   reading timing on its own.
//!
//! **Why the readings are taken from the container rather than mocked.** Every one of them is
//! a line of shell whose output this parses, and shell that answers slightly differently than
//! expected is the failure this catches: `ufw` not installed at all, `free -m` laying its
//! columns out otherwise, `systemd-detect-virt` absent. A mocked answer checks the parser
//! against itself.

use std::time::Duration;

use futures::future::BoxFuture;

use super::deploy_clean::by_password;
use super::deploy_fixture::{DeployTarget, Flavour};
use super::fixture::{key_path, TestServer, KEY_PASSPHRASE};
use super::hls_fixture::{lay_out_direct_file, lay_out_ladder, VIDEO_DIR};
use super::viewer::Viewer;
use vrcast_studio_lib::domain::access_log::parse_line;
use vrcast_studio_lib::domain::deploy_steps::StepId;
use vrcast_studio_lib::domain::dns_verdict::{Ipv6Choice, ServerAddresses};
use vrcast_studio_lib::domain::health::{self, Rating, Reading};
use vrcast_studio_lib::domain::stalls::{self, Cause};
use vrcast_studio_lib::domain::wording::DetailCode;
use vrcast_studio_lib::server::deploy::{self, machine, Context, Proofs};
use vrcast_studio_lib::server::health as server_health;
use vrcast_studio_lib::ssh::keygen;
use vrcast_studio_lib::ssh::{fingerprint, Connection, Credentials, ServerAddress};

/// The domain the four steps are given. Never resolved: the serving is asked over the
/// loopback with this name in the Host, which is what a deployed Caddy answers to.
const DEPLOY_DOMAIN: &str = "vrcast-container.invalid";

async fn connect(server: &TestServer) -> Connection {
    let a = ServerAddress::new(server.host(), server.port);
    let fp = fingerprint::probe(&a)
        .await
        .expect("the fingerprint was not obtained");
    Connection::connect(
        a,
        "root",
        Credentials::Key {
            path: key_path(),
            passphrase: Some(KEY_PASSPHRASE.to_owned()),
        },
        &fp,
    )
    .await
    .expect("connecting failed")
}

fn rating_of(readings: &[health::Rated], about: Reading) -> Rating {
    readings
        .iter()
        .find(|r| r.about == about)
        .unwrap_or_else(|| panic!("{about:?} was not judged at all"))
        .rating
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stopped_serving_is_seen_and_named() {
    // **On a machine with a service manager**, because that is what the reading asks. The
    // serving fixture runs Caddy as a bare process with no systemd at all, and there
    // "is-active" cannot answer — which is its own finding, checked separately below.
    //
    // Only four steps are run, not a deployment: the packages, the directories, the
    // configuration and the service. The key, the hardening, the firewall and the domain have
    // nothing to do with the question and cost a minute and a half.
    let target = DeployTarget::start(Flavour::Clean).expect("the bare container would not come up");
    let made = keygen::make("vrcast-studio: the diagnosis check").expect("no key was made");
    let conn = by_password(&target).await;
    let facts = machine::look(&conn).await.expect("no machine facts");

    let key_proof = || -> BoxFuture<'_, bool> { Box::pin(async { true }) };
    let password_proof = || -> BoxFuture<'_, bool> { Box::pin(async { false }) };
    let ctx = Context {
        conn: &conn,
        domain: DEPLOY_DOMAIN,
        video_dir: VIDEO_DIR,
        ipv6: Ipv6Choice::Keep,
        server: ServerAddresses { v4: None, v6: None },
        public_key: made.public_openssh.clone(),
        machine: facts,
        already_ours: false,
        replace_caddyfile: false,
        run: vrcast_studio_lib::server::deploy::RunMark::fresh(),
        proofs: Proofs {
            key_works: &key_proof,
            password_refused: &password_proof,
        },
    };
    let steps: Vec<_> = deploy::all()
        .into_iter()
        .filter(|s| {
            matches!(
                s.id,
                StepId::Packages | StepId::UserDirs | StepId::Configs | StepId::Services
            )
        })
        .collect();
    let never = || false;
    deploy::run(&ctx, &steps, &never, &mut |_| {})
        .await
        .expect("the serving would not go in");

    let before = server_health::look(&conn, VIDEO_DIR, DEPLOY_DOMAIN)
        .await
        .expect("the machine would not answer");
    assert_eq!(
        rating_of(&health::judge(&before), Reading::Serving),
        Rating::Fine,
        "a running serving was not seen as running: {:?}",
        before.services
    );

    // And now it does not serve.
    target
        .exec_inside("systemctl stop caddy")
        .expect("the serving would not stop");

    let after = server_health::look(&conn, VIDEO_DIR, DEPLOY_DOMAIN)
        .await
        .expect("the machine would not answer");
    let judged = health::judge(&after);
    let serving = judged
        .iter()
        .find(|r| r.about == Reading::Serving)
        .expect("the serving was not judged");

    assert_eq!(serving.rating, Rating::Trouble);
    assert_eq!(
        serving.say.params.get("service").and_then(|v| v.as_str()),
        Some("caddy"),
        "the stopped service was not named: {:?}",
        serving.say
    );
    // And the whole snapshot says trouble rather than averaging itself out to "worth a look".
    assert_eq!(health::worst(&judged), Rating::Trouble);
}

#[tokio::test(flavor = "multi_thread")]
async fn what_a_container_cannot_answer_comes_back_as_not_established() {
    // The container is the one place where this can be checked at all: kernel settings and a
    // real disk are the host's, and a reading that invented a value for them here would be
    // inventing one on every machine.
    let server = TestServer::start().expect("the container would not come up");
    lay_out_direct_file(&server, "film.mp4", 3_000_000).expect("the file was not laid out");
    let conn = connect(&server).await;

    let snap = server_health::look(&conn, VIDEO_DIR, "vrcast-container.invalid")
        .await
        .expect("the machine would not answer");
    // This fixture has no systemd in it at all, which is why the question can be asked here:
    // `systemd-detect-virt` comes with systemd, and a reading that relied on it alone would
    // answer "not a container" inside one — the exact wrong way round.
    assert!(
        snap.container,
        "the container did not recognise itself as one, so the rest of this proves nothing"
    );

    let judged = health::judge(&snap);
    assert_eq!(rating_of(&judged, Reading::Network), Rating::Unknown);
    assert_eq!(rating_of(&judged, Reading::Readahead), Rating::Unknown);
    // Never dressed up as fine — that is the failure that hides every other failure.
    assert_ne!(rating_of(&judged, Reading::Network), Rating::Fine);

    // **And the serving is not called broken here.** Caddy runs as a bare process, so the
    // service manager has never heard of it and answers `unknown`. Read as "stopped", that
    // produced a snapshot saying the serving was down and the delivery was fine in the same
    // breath — not a judgement but a contradiction. Found here on 2026-08-27.
    assert_eq!(rating_of(&judged, Reading::Serving), Rating::Unknown);
    assert_eq!(
        rating_of(&judged, Reading::Delivery),
        Rating::Fine,
        "the container is serving and the delivery check did not see it: {:?}",
        snap.delivery
    );

    conn.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_slow_viewer_is_a_short_link_and_not_a_server_in_trouble() {
    let server = TestServer::start().expect("the container would not come up");
    // A quality set rather than a single file: the ratio of content received to time lived
    // is counted in segments, and a directly served film is one long pull with no segments
    // in it at all.
    lay_out_ladder(&server, "demo").expect("the quality set was not laid out");
    let slow = Viewer::attach(&server).expect("the viewer would not attach");
    let conn = connect(&server).await;

    // **The rate is picked against the rung, not out of the air.** The lightest rung's
    // segments run about 800 kB for four seconds of film, so keeping up needs 200 kB/s.
    // Held to fifty, this viewer receives one second of film for every four lived — roughly
    // 0.25×, which is worse than the recorded case and comfortably the wrong side of 1.0.
    slow.start_watching_a_set("demo", "v3", 3, Some("50k"))
        .expect("the watching would not start");
    // Long enough for several segments to have **finished**: the serving writes its line when
    // a request ends, and at sixteen seconds a segment a shorter wait would leave one line,
    // from which no stretch and no speed can be worked out at all.
    std::thread::sleep(Duration::from_secs(50));

    // T706: the viewer is counted as connected while they pull. The count used to match the
    // peer's port rather than this machine's, saw nobody, and the cache reading said "nobody
    // is watching" beside a list of people watching.
    let snap = server_health::look(&conn, VIDEO_DIR, DEPLOY_DOMAIN)
        .await
        .expect("the machine would not answer");
    assert!(
        snap.watching_now >= 1,
        "a viewer pulling right now was not counted as connected: {}",
        snap.watching_now
    );

    let live = server_health::load(&conn)
        .await
        .expect("the live readings would not come");
    let log = server.access_log().expect("the access log would not read");
    let requests: Vec<_> = log.lines().filter_map(|l| parse_line(l).ok()).collect();
    let sifted = stalls::sift(&requests, &live.addresses);

    let watcher = sifted
        .watchers
        .iter()
        .find(|w| w.client_ip == slow.ip())
        .unwrap_or_else(|| {
            panic!(
                "the slow viewer is not in the report at all. Addresses seen: {:?}",
                sifted
                    .watchers
                    .iter()
                    .map(|w| &w.client_ip)
                    .collect::<Vec<_>>()
            )
        });

    assert!(
        watcher.starving(),
        "a viewer held to 100 kB/s is keeping up: ratio {:?}, {} segments over {} s",
        watcher.content_ratio,
        watcher.segments,
        watcher.elapsed_s
    );

    // What the rung needs, read off the set's own description on the server, as
    // `diag_explain_stalls` does (T705) — without it the link is not blamed at all.
    let needs =
        vrcast_studio_lib::server::viewers::rung_needs(&conn, VIDEO_DIR, &[String::from("demo")])
            .await;
    let mut watcher = watcher.clone();
    assert_eq!(
        watcher.rung.as_deref(),
        Some("v3"),
        "the rung was not read off the log"
    );
    watcher.need_mbit = needs
        .get("demo")
        .and_then(|rungs| rungs.get("v3"))
        .map(|bps| *bps as f64 / 1_000_000.0);
    assert!(
        watcher.need_mbit.is_some(),
        "the rung's need was not read off the set on the server: {needs:?}"
    );

    let verdict = stalls::explain(&watcher, Some(&live.load), None);
    // ⚠ **The link or the player — both are the viewer's side, and that is the question.**
    // Measured here 2026-10-09: held to 50 kB/s, this viewer still showed 6.16 Mbit/s "inside
    // the downloads" against a rung needing 2. The server finishes a request when the last
    // byte is in the socket buffers, not when the viewer has it, so from the log a slow link
    // can look like a player that stops asking — and `STALLS_THE_PLAYER` now names both
    // (T706). What must never happen is the idle server taking the blame, or no answer.
    assert!(
        matches!(verdict.cause, Cause::ViewerLink | Cause::ThePlayer),
        "an idle container was blamed for a viewer's own narrow link: {:?}",
        verdict.say
    );
    // Either way with what the rung needs, in the figures the conclusion stands on.
    let need = verdict
        .say
        .params
        .get("need_mbit")
        .or_else(|| verdict.say.params.get("average_mbit"))
        .and_then(|v| v.as_f64());
    assert_eq!(
        need,
        watcher.need_mbit.map(|n| (n * 100.0).round() / 100.0),
        "the conclusion does not carry the number the rung needs: {:?}",
        verdict.say
    );

    slow.stop_watching().expect("the watching would not stop");
    conn.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_viewer_with_a_full_buffer_is_left_alone() {
    // The check that matters most. This viewer pulls at full speed and then waits — long
    // gaps between requests, which is what a stall looks like to anything reading timing on
    // its own. Calling them starving is how a panel earns the right to be ignored.
    let server = TestServer::start().expect("the container would not come up");
    lay_out_ladder(&server, "demo").expect("the quality set was not laid out");
    let fast = Viewer::attach(&server).expect("the viewer would not attach");
    let conn = connect(&server).await;

    fast.start_watching_a_set("demo", "v3", 3, None)
        .expect("the watching would not start");
    std::thread::sleep(Duration::from_secs(12));

    let live = server_health::load(&conn)
        .await
        .expect("the live readings would not come");
    let log = server.access_log().expect("the access log would not read");
    let requests: Vec<_> = log.lines().filter_map(|l| parse_line(l).ok()).collect();
    let sifted = stalls::sift(&requests, &live.addresses);

    let watcher = sifted
        .watchers
        .iter()
        .find(|w| w.client_ip == fast.ip())
        .expect("the fast viewer is not in the report at all");

    assert!(
        !watcher.starving(),
        "a viewer pulling at full speed was called starving: ratio {:?}, {} segments over {} s",
        watcher.content_ratio,
        watcher.segments,
        watcher.elapsed_s
    );
    assert_eq!(
        stalls::explain(watcher, Some(&live.load), None).cause,
        Cause::NothingWrong
    );

    fast.stop_watching().expect("the watching would not stop");
    conn.close().await;
}

/// Hold everything the server sends to `ip` to `rate` (tc's notation, `"800kbit"`) — a viewer
/// whose **link** is short, on the server's way out to them. Not curl's `--limit-rate`: that
/// is a viewer that reads slowly, which from the connection is the player not taking what
/// arrives, the other half of the question (T711).
fn shape_the_way_to(server: &TestServer, ip: &str, rate: &str) {
    let script = format!(
        "set -e
dev=$(ip -o route get {ip} | awk '{{for (i = 1; i < NF; i++) if ($i == \"dev\") print $(i + 1)}}')
tc qdisc add dev \"$dev\" root handle 1: prio priomap 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0
tc qdisc add dev \"$dev\" parent 1:3 handle 30: netem rate {rate}
tc filter add dev \"$dev\" parent 1: protocol ip prio 1 u32 match ip dst {ip}/32 flowid 1:3
echo shaped $dev"
    );
    let out = std::process::Command::new("docker")
        .args([
            "exec",
            "--privileged",
            server.container_id(),
            "sh",
            "-c",
            &script,
        ])
        .output()
        .expect("docker would not run");
    assert!(
        out.status.success() && String::from_utf8_lossy(&out.stdout).contains("shaped"),
        "the way to {ip} could not be narrowed: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// T711 — the owner's decision: look at the live connection. Two viewers fall behind the same
/// film at about the same speed, and the log alone cannot tell them apart (measured
/// 2026-10-09: it saw 6.16 Mbit/s "inside the downloads" of a viewer getting 0.4). One has a
/// short link; the other's player reads slowly. Their connections, read as the «Зрители»
/// screen reads them, tell which is which.
#[tokio::test(flavor = "multi_thread")]
async fn the_live_connection_tells_a_short_link_from_a_player_not_taking() {
    let server = TestServer::start().expect("the container would not come up");
    lay_out_ladder(&server, "demo").expect("the quality set was not laid out");
    let short_link = Viewer::attach(&server).expect("the first viewer would not attach");
    let slow_player = Viewer::attach(&server).expect("the second viewer would not attach");
    shape_the_way_to(&server, short_link.ip(), "800kbit");
    let conn = connect(&server).await;

    short_link
        .start_watching_a_set_on_one_connection("demo", "v3", 3, None)
        .expect("the first watching would not start");
    slow_player
        .start_watching_a_set_on_one_connection("demo", "v3", 3, Some("100k"))
        .expect("the second watching would not start");
    std::thread::sleep(Duration::from_secs(50));

    let (live, polls) = server_health::load_with_connections(&conn)
        .await
        .expect("the live readings would not come");
    let (before, after) = polls.expect("the connection table was not read around the load");
    let links = stalls::live_links(&before, &after);
    println!("live links: {links:#?}");

    let log = server.access_log().expect("the access log would not read");
    let requests: Vec<_> = log.lines().filter_map(|l| parse_line(l).ok()).collect();
    let sifted = stalls::sift(&requests, &live.addresses);
    let needs =
        vrcast_studio_lib::server::viewers::rung_needs(&conn, VIDEO_DIR, &[String::from("demo")])
            .await;
    println!("needs: {needs:?}");
    assert!(
        needs.get("demo").is_some_and(|r| r.contains_key("v3")),
        "the rung's need was not read off the set on the server: {needs:?}"
    );

    let verdict_for = |ip: &str| {
        let mut w = sifted
            .watchers
            .iter()
            .find(|w| w.client_ip == ip)
            .unwrap_or_else(|| panic!("{ip} is not in the report: {:?}", sifted.set_aside))
            .clone();
        w.need_mbit = needs
            .get("demo")
            .and_then(|rungs| rungs.get("v3"))
            .map(|bps| *bps as f64 / 1_000_000.0);
        w.live = links.get(ip).copied();
        assert!(w.starving(), "{ip} is keeping up: {w:?}");
        assert!(w.live.is_some(), "{ip}'s connection was not read live");
        let verdict = stalls::explain(&w, Some(&live.load), None);
        println!(
            "{ip}: rung {:?}, need {:?}: {:?} {:?}",
            w.rung, w.need_mbit, verdict.cause, verdict.say
        );
        verdict
    };

    let link = verdict_for(short_link.ip());
    assert_eq!(
        link.cause,
        Cause::ViewerLink,
        "a viewer whose way in is 0.8 Mbit/s was not told their link is short: {:?}",
        link.say
    );
    assert_eq!(link.say.key, DetailCode::StallsViewerLinkLive);

    let player = verdict_for(slow_player.ip());
    assert_eq!(
        player.cause,
        Cause::ThePlayer,
        "a viewer reading slowly on a wide link was blamed on the link: {:?}",
        player.say
    );
    // ⚠ **Either way of saying it, and that is measured, not lenient.** In four runs here
    // (2026-10-09) the slow reader's side was twice full for most of the sending (88%, 90%)
    // — said from the connection — and twice the connection stood idle 98% of the stretch,
    // a whole segment already in the reader's own buffers: nothing on the connection to go
    // on, and the log's reasoning (the server hands pieces out faster than needed) says it
    // instead. What may never happen is the link taking the blame.
    assert!(
        matches!(
            player.say.key,
            DetailCode::StallsPlayerLive | DetailCode::StallsThePlayer
        ),
        "{:?}",
        player.say
    );

    short_link
        .stop_watching()
        .expect("the watching would not stop");
    slow_player
        .stop_watching()
        .expect("the watching would not stop");
    conn.close().await;
}
