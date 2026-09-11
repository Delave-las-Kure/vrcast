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

/// A film long enough that cutting it takes real, observable wall-clock time — long enough
/// to reliably still be running after the few hundred milliseconds these tests wait before
/// acting, on a loaded CI machine as well as a fast one.
fn make_slow_film(server: &TestServer, name: &str) -> Result<(), String> {
    server.exec_inside(&format!(
        "ffmpeg -nostdin -y -loglevel error \
         -f lavfi -i testsrc2=size=960x540:rate=24:duration=40 \
         -f lavfi -i sine=frequency=440:duration=40 \
         -c:v libx264 -preset veryslow -b:v 4000k -g 48 -keyint_min 48 \
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
}
