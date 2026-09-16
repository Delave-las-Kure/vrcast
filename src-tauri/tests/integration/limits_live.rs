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

const CONF: &str = "/etc/caddy/vrcast-limits.conf";
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
            &[(String::from("demo"), short.clone())],
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
            &[(String::from("demo"), shorten(&all, cap, PREFIX, "demo"))],
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
            &[],
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
    let now = server
        .exec_inside(&format!("cat {CONF}"))
        .expect("the rules would not be read");
    assert_eq!(
        now.trim(),
        good.trim(),
        "the previous rules did not come back as they were"
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
            &[(String::from("demo"), shorten(&all, cap, PREFIX, "demo"))],
            0,
        )
        .await
        .expect("the limit would not go on");

    serving
        .clear(&[], "demo", 1)
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
            "test -f {VIDEO_DIR}/_slow/demo/master.m3u8 && echo still-there || echo gone"
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
            &[(String::from("demo"), short)],
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
    let outcome = serving.apply(&limits, &[], generation).await;
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
