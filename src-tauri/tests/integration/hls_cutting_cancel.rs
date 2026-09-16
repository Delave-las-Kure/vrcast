//! T597 — cancelling `ladder_build` during the cutting phase actually stops the detached
//! server-side process, not merely the local poll.
//!
//! **What was found by an independent QA audit 2026-09-11 (round 16).** `Cutting::run`
//! (`server/hls_package.rs`) is a `setsid nohup` process, detached from the SSH session on
//! purpose (its own module doc explains why: a session cut mid-cutting must not take the
//! work with it). Before T597, nothing about that loop knew of `TaskContext` at all — a
//! person pressing "stop" during cutting waited out the whole of `ASK_EVERY`, and even then
//! nothing on the server was ever asked to end: the detached process ran to completion
//! regardless, spending CPU and disk on a build the interface already showed as cancelled.
//!
//! Two things are checked here, matching the two pieces the task names:
//! 1. [`Cutting::stop_remote`] on its own: start a cutting, wait until `pgrep` sees the
//!    process on the server, ask it to stop, and confirm `pgrep` no longer finds it.
//! 2. The wiring: `Cutting::run` given a cancelled `TaskContext` returns `Cancelled` promptly
//!    (not after `ASK_EVERY`) and the server-side process is gone by the time it returns —
//!    proving `run` really calls `stop_remote` on the cancellation path, not just that the
//!    method exists.

use std::time::Duration;

use vrcast_studio_lib::domain::hls_package::ToCut;
use vrcast_studio_lib::server::hls_package::{Cutting, CuttingError};
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::tasks::engine::TaskContext;

use super::fixture::TestServer;
use super::hls_fixture::VIDEO_DIR;
use super::ssh_live::connect;

fn detached_ctx() -> TaskContext {
    TaskContext::detached(std::sync::Arc::new(Db::open_in_memory().unwrap()))
}

/// A film large enough that its `-c copy` HLS remux takes several real seconds — not merely
/// milliseconds — so that a test calling [`Cutting::stop_remote`] partway through the remux
/// has a wide, reliable window in which to tell "really killed at once" apart from "merely
/// outlived the wrapper and finished remuxing on its own a few seconds later" (T598).
///
/// **Why this big, and not the smaller fixture T597 used.** `-c copy` does no re-encoding —
/// it is a data copy shaped into segments — so its wall-clock cost tracks the amount of data
/// moved, not the video's nominal duration. T597's fixture (a modest 1280x720/6000k film)
/// remuxes in well under a second once cutting actually starts, which turned out to leave
/// too short a window for this test's purpose: measured directly while writing T598, a
/// `pkill -f` that hits only the wrapper and never the child `ffmpeg` let that small fixture
/// finish remuxing on its own so quickly that a test asserting "`ffmpeg` gone soon after
/// `stop_remote`" could pass by coincidence even against the unfixed bug — "soon" and
/// "finished on its own" were not reliably distinguishable at that size. A five-plus
/// gigabyte file, measured the same way, took several real seconds to remux — long enough
/// that "gone within a couple of seconds of being told to stop" and "gone only once the
/// whole remux finished on its own five-plus seconds later" are two clearly different,
/// reliably observable outcomes.
fn make_slow_film(server: &TestServer, name: &str) -> Result<(), String> {
    server.exec_inside(&format!(
        "ffmpeg -nostdin -y -loglevel error \
         -f lavfi -i testsrc2=size=1920x1080:rate=30:duration=1200 \
         -f lavfi -i sine=frequency=440:duration=1200 \
         -c:v libx264 -preset ultrafast -b:v 80000k -g 60 -keyint_min 60 \
         -pix_fmt yuv420p -c:a aac -b:a 128k -shortest \
         '{VIDEO_DIR}/{name}' && echo made"
    ))?;
    Ok(())
}

/// Wait until `pgrep` on the server agrees the cutting script is running (or is not).
async fn wait_until_running(cutting: &Cutting<'_>, want: bool, limit: Duration) {
    let deadline = std::time::Instant::now() + limit;
    loop {
        let running = cutting.still_running().await.unwrap_or(!want);
        if running == want {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the cutting process's running state never became {want} in the time allowed"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Wait until `pgrep -x ffmpeg` on the server agrees no `ffmpeg` process is running.
///
/// T598 — [`wait_until_running`] above only ever asks about the wrapper **script**
/// (`pgrep -f <the self-excluding wrapper pattern>`), which is exactly the gap that let
/// T597's `pkill -f` regression through unnoticed: the wrapper can be long dead while the
/// `ffmpeg` it spawned lives on, reparented to init, still writing segments. `pgrep -x
/// ffmpeg` asks about the *executable name* exactly, not a command-line substring — unlike
/// `pgrep -f`, it needs no self-excluding bracket trick, because the invocation doing the
/// asking is itself named `pgrep`, never `ffmpeg`.
async fn wait_until_ffmpeg_gone(server: &TestServer, limit: Duration) {
    let deadline = std::time::Instant::now() + limit;
    loop {
        let gone = server
            .exec_inside("pgrep -x ffmpeg >/dev/null && echo yes || echo no")
            .map(|out| out.trim() == "no")
            .unwrap_or(false);
        if gone {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "an ffmpeg process on the server was still running {limit:?} after the cutting \
             was told to stop — T598: killing only the wrapper script leaves the child \
             ffmpeg orphaned and writing indefinitely"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stop_remote_ends_the_detached_process_on_the_server() {
    let server = TestServer::start().expect("the container would not come up");
    make_slow_film(&server, "stop_remote.mp4").expect("the fixture film would not encode");

    let conn = connect(&server).await;
    let cutting = Cutting {
        conn: &conn,
        video_dir: VIDEO_DIR,
        owner: "root:root",
        base: "stopremote",
        variants: &[ToCut {
            sub: String::from("v1"),
            file: String::from("stop_remote.mp4"),
        }],
    };

    cutting.start().await.expect("the cutting would not start");
    wait_until_running(&cutting, true, Duration::from_secs(10)).await;

    cutting.stop_remote().await;

    wait_until_running(&cutting, false, Duration::from_secs(10)).await;

    // T598 — the check above only proves the wrapper script is gone; the whole point of
    // this task is that the child `ffmpeg` it spawned must be gone too, not merely
    // orphaned and still writing segments under a different parent. A tight 5s bound
    // (not the generous 10s used elsewhere) matters here: measured directly while writing
    // this test, the unfixed bug does not hang forever — the orphaned `ffmpeg` simply
    // finishes remuxing on its own, ~6s after being "stopped" for this fixture's size. A
    // generous bound would let that natural completion slip through as a false pass; 5s
    // is comfortably above the fix's actual response (well under a second) and
    // comfortably below the bug's ~6s natural-completion time for this fixture.
    wait_until_ffmpeg_gone(&server, Duration::from_secs(5)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelling_the_task_context_stops_the_server_side_cutting_promptly() {
    let server = TestServer::start().expect("the container would not come up");
    make_slow_film(&server, "cancel_run.mp4").expect("the fixture film would not encode");

    let conn = connect(&server).await;
    let cutting = Cutting {
        conn: &conn,
        video_dir: VIDEO_DIR,
        owner: "root:root",
        base: "cancelrun",
        variants: &[ToCut {
            sub: String::from("v1"),
            file: String::from("cancel_run.mp4"),
        }],
    };

    let ctx = detached_ctx();
    let cancel_token = ctx.cancel_token();

    // Cancel shortly after `run` starts, from a task of its own — `run` itself calls
    // `start()` and only then begins polling, so cancelling too early would race the very
    // first `pgrep` before the process exists at all.
    let canceller = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        cancel_token.cancel();
    });

    let started = std::time::Instant::now();
    let result = cutting.run(&ctx, |_| {}).await;
    let elapsed = started.elapsed();

    canceller.await.expect("the canceller task panicked");

    assert!(
        matches!(result, Err(CuttingError::Cancelled)),
        "run() did not report Cancelled for a cancelled TaskContext: {result:?}"
    );

    // Answered promptly — well inside one ASK_EVERY (5s) of the cancellation firing at
    // ~500ms — not merely eventually. A regression back to polling-only cancellation would
    // make this take upwards of 5 seconds from the cancel instant.
    assert!(
        elapsed < Duration::from_secs(4),
        "run() took {elapsed:?} to notice the cancellation — this is what T597 exists to keep \
         under one ASK_EVERY cycle, not several"
    );

    // And the server-side process is really gone, not merely reported as cancelled locally —
    // the whole point of T597 is that the detached work does not go on regardless.
    wait_until_running(&cutting, false, Duration::from_secs(10)).await;

    // T598 — and the child `ffmpeg` the wrapper spawned must be gone too: killing only the
    // wrapper's own process left `ffmpeg` orphaned onto init, still writing segments long
    // after `run()` had already reported `Cancelled` to the caller. Same tight 5s bound as
    // `stop_remote_ends_the_detached_process_on_the_server` above, and for the same reason:
    // the unfixed bug's orphaned `ffmpeg` finishes remuxing on its own after ~6s for this
    // fixture's size, which a more generous bound would let slip through as a false pass.
    wait_until_ffmpeg_gone(&server, Duration::from_secs(5)).await;
}
