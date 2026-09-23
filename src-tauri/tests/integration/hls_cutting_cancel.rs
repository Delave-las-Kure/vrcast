//! T597 — cancelling `ladder_build` during the cutting phase actually stops the detached
//! server-side process, not merely the local poll. T605 — and the stop is **confirmed**:
//! the whole group, the wrapper dead before its `ffmpeg`, a member ignoring TERM.
//!
//! **What was found by an independent QA audit 2026-09-11 (round 16).** `Cutting::run`
//! (`server/hls_package.rs`) is a `setsid nohup` process, detached from the SSH session on
//! purpose (its own module doc explains why: a session cut mid-cutting must not take the
//! work with it). Before T597, nothing about that loop knew of `TaskContext` at all — a
//! person pressing "stop" during cutting waited out the whole of `ASK_EVERY`, and even then
//! nothing on the server was ever asked to end.
//!
//! **What was found by the next one, 2026-09-23 (round 18) — T605.** T598's stop found the
//! wrapper by `pgrep -f`, sent the group one TERM and returned at once. A wrapper already
//! dead (so `pgrep` found nothing), a member ignoring TERM, a connection dead before the
//! kill: in each of them `Cancelled` was written while `ffmpeg` went on writing. The checks
//! below are those cases, on a real server:
//! - a) the wrapper killed on its own, `ffmpeg` left running: the cutting still counts as
//!   running, and the stop ends `ffmpeg` and says so;
//! - b) a member of the group ignoring TERM: the stop escalates to KILL and says so;
//! - c) `start()` records the group, and the record names a live leader of a group of its
//!   own — not the SSH shell's;
//! - and the T597/T598 checks, unchanged in what they ask.
//!
//! The unconfirmed stop — the task staying running and holding its slug until the server
//! confirms — is `tests/unit/cutting_stop.rs`, against the real engine; see its header for
//! why a network cut would prove nothing more about that rule.

use std::time::Duration;

use vrcast_studio_lib::domain::hls_package::ToCut;
use vrcast_studio_lib::server::hls_package::{Cutting, CuttingError, Stopped};
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::tasks::engine::TaskContext;

use super::fixture::TestServer;
use super::hls_fixture::VIDEO_DIR;
use super::ssh_live::connect;

fn detached_ctx() -> TaskContext {
    TaskContext::detached(std::sync::Arc::new(Db::open_in_memory().unwrap()))
}

/// A film large enough that its `-c copy` HLS remux takes several real seconds — not merely
/// milliseconds — so that a test calling a stop partway through the remux has a wide,
/// reliable window in which to tell "really killed at once" apart from "merely outlived the
/// wrapper and finished remuxing on its own a few seconds later" (T598).
///
/// **Why this big, and not the smaller fixture T597 used.** `-c copy` does no re-encoding —
/// it is a data copy shaped into segments — so its wall-clock cost tracks the amount of data
/// moved, not the video's nominal duration. T597's fixture (a modest 1280x720/6000k film)
/// remuxes in well under a second once cutting actually starts, which turned out to leave
/// too short a window for this test's purpose: measured directly while writing T598, a
/// `pkill -f` that hits only the wrapper and never the child `ffmpeg` let that small fixture
/// finish remuxing on its own so quickly that a test asserting "`ffmpeg` gone soon after
/// the stop" could pass by coincidence even against the unfixed bug. A five-plus gigabyte
/// file, measured the same way, took several real seconds to remux.
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

/// A tiny film — for the checks where the real `ffmpeg` is never what runs.
fn make_small_film(server: &TestServer, name: &str) -> Result<(), String> {
    server.exec_inside(&format!(
        "ffmpeg -nostdin -y -loglevel error \
         -f lavfi -i testsrc2=size=320x240:rate=24:duration=4 \
         -f lavfi -i sine=frequency=440:duration=4 \
         -c:v libx264 -preset ultrafast -g 24 -pix_fmt yuv420p -c:a aac -shortest \
         '{VIDEO_DIR}/{name}' && echo made"
    ))?;
    Ok(())
}

/// Wait until the server agrees the cutting is running (or is not).
///
/// Since T605 `still_running` asks about **every** process of the cutting — the mark in
/// their environment — not only the wrapper script's command line; "not running" is now the
/// stronger claim, and so this helper's `false` is too.
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

/// Live (not zombie) processes of this name on the server, asked around our own code.
///
/// `ps` rather than `pgrep -x`: `pgrep` counts a zombie as a match, and a zombie executes
/// nothing — it is not what these checks are about.
fn live_named(server: &TestServer, name: &str) -> Vec<String> {
    server
        .exec_inside(&format!(
            "ps -eo pid=,stat=,comm= | awk '$2 !~ /^Z/ && $3 == \"{name}\" {{ print $1 }}'"
        ))
        .map(|out| out.split_whitespace().map(str::to_owned).collect())
        .unwrap_or_default()
}

/// Wait until no live `ffmpeg` is running on the server.
///
/// T598 — [`wait_until_running`] once asked only about the wrapper script, which is exactly
/// the gap that let T597's `pkill -f` regression through unnoticed: the wrapper can be long
/// dead while the `ffmpeg` it spawned lives on, reparented to init, still writing segments.
async fn wait_until_ffmpeg_gone(server: &TestServer, limit: Duration) {
    let deadline = std::time::Instant::now() + limit;
    loop {
        if live_named(server, "ffmpeg").is_empty() {
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

/// Wait until a live `ffmpeg` has appeared — the cutting has got past `ffprobe`.
async fn wait_until_ffmpeg_runs(server: &TestServer, limit: Duration) -> Vec<String> {
    let deadline = std::time::Instant::now() + limit;
    loop {
        let found = live_named(server, "ffmpeg");
        if !found.is_empty() {
            return found;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "ffmpeg never started on the server"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn one_variant<'a>(
    conn: &'a vrcast_studio_lib::ssh::Connection,
    base: &'a str,
    variants: &'a [ToCut],
) -> Cutting<'a> {
    Cutting {
        conn,
        video_dir: VIDEO_DIR,
        owner: "root:root",
        base,
        variants,
    }
}

fn variant(file: &str) -> Vec<ToCut> {
    vec![ToCut {
        sub: String::from("v1"),
        file: file.to_owned(),
    }]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stop_ends_the_detached_process_on_the_server() {
    let server = TestServer::start().expect("the container would not come up");
    make_slow_film(&server, "stop_remote.mp4").expect("the fixture film would not encode");

    let conn = connect(&server).await;
    let variants = variant("stop_remote.mp4");
    let cutting = one_variant(&conn, "stopremote", &variants);

    let started = cutting.start().await.expect("the cutting would not start");
    wait_until_running(&cutting, true, Duration::from_secs(10)).await;

    let how = cutting
        .stop(&started)
        .await
        .expect("the stop was not confirmed");
    assert_eq!(
        how,
        Stopped::AfterTerm,
        "ffmpeg and the wrapper end on TERM"
    );

    // Confirmed means gone *now*, not "soon": no waiting here, unlike T598's version of
    // this check, which could only hope the one TERM it sent had landed.
    assert!(
        !cutting
            .still_running()
            .await
            .expect("the server would not say"),
        "the stop said it was confirmed, and something of the cutting is still alive"
    );
    wait_until_ffmpeg_gone(&server, Duration::from_millis(1)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelling_the_task_context_stops_the_server_side_cutting_promptly() {
    let server = TestServer::start().expect("the container would not come up");
    make_slow_film(&server, "cancel_run.mp4").expect("the fixture film would not encode");

    let conn = connect(&server).await;
    let variants = variant("cancel_run.mp4");
    let cutting = one_variant(&conn, "cancelrun", &variants);

    let ctx = detached_ctx();
    let cancel_token = ctx.cancel_token();

    // Cancel shortly after `run` starts, from a task of its own — `run` itself calls
    // `start()` and only then begins polling.
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
    // ~500ms — not merely eventually.
    assert!(
        elapsed < Duration::from_secs(4),
        "run() took {elapsed:?} to notice the cancellation — this is what T597 exists to keep \
         under one ASK_EVERY cycle, not several"
    );

    // T605 — `Cancelled` is returned only after the stop is confirmed, so by the time it is
    // there is nothing left to wait for: not the wrapper, not `ffmpeg`.
    assert!(
        !cutting
            .still_running()
            .await
            .expect("the server would not say"),
        "run() returned Cancelled while something of the cutting was still alive"
    );
    wait_until_ffmpeg_gone(&server, Duration::from_millis(1)).await;
}

/// a) The wrapper killed on its own, its `ffmpeg` left running — the case `pgrep -f` on the
/// wrapper's command line could not see at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cutting_whose_wrapper_died_is_still_running_and_its_ffmpeg_is_stopped() {
    let server = TestServer::start().expect("the container would not come up");
    make_slow_film(&server, "orphan.mp4").expect("the fixture film would not encode");

    let conn = connect(&server).await;
    let variants = variant("orphan.mp4");
    let cutting = one_variant(&conn, "orphan", &variants);

    let started = cutting.start().await.expect("the cutting would not start");
    let ffmpeg = wait_until_ffmpeg_runs(&server, Duration::from_secs(15)).await;

    // Only the wrapper, by its own pid, and with KILL — it gets no chance to pass anything on.
    server
        .exec_inside(&format!("kill -9 {}", started.group.pid))
        .expect("the wrapper would not be killed");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        live_named(&server, "ffmpeg"),
        ffmpeg,
        "the fixture itself is broken: ffmpeg did not outlive its wrapper, so this check \
         would prove nothing"
    );

    // What `run()` asks between polls: with the wrapper gone and `ffmpeg` writing, the
    // answer has to be "running" — "not running" fails the build and lets go of the
    // directory `ffmpeg` is writing into.
    assert!(
        cutting
            .still_running()
            .await
            .expect("the server would not say"),
        "a cutting whose ffmpeg is still writing reads as not running because its wrapper \
         is gone"
    );

    let how = cutting
        .stop(&started)
        .await
        .expect("the stop was not confirmed");
    assert_ne!(
        how,
        Stopped::AlreadyGone,
        "the stop saw nothing to stop while ffmpeg was running"
    );
    // Confirmed means gone now: no grace period for a natural finish to hide behind.
    assert!(
        live_named(&server, "ffmpeg").is_empty(),
        "the stop was reported confirmed ({how:?}) and ffmpeg is still running"
    );
    assert!(!cutting
        .still_running()
        .await
        .expect("the server would not say"));
}

/// b) A member of the group that ignores TERM — the stop has to escalate, and say so.
///
/// An `ffmpeg` put in front of the real one in `PATH` (`/usr/local/bin` comes before
/// `/usr/bin` on this system), that ignores TERM and sleeps. `trap '' TERM` sets the signal
/// to ignored, and an ignored signal stays ignored across `exec`, so the `sleep` it becomes
/// ignores it too. The real `ffmpeg` could not be used for this: it installs a TERM handler
/// of its own, which overrides an inherited "ignore".
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_member_that_ignores_term_is_killed_and_the_stop_says_so() {
    let server = TestServer::start().expect("the container would not come up");
    make_small_film(&server, "stubborn.mp4").expect("the fixture film would not encode");
    server
        .exec_inside(
            "printf '%s\\n' '#!/bin/bash' \"trap '' TERM\" 'exec sleep 600' \
             > /usr/local/bin/ffmpeg && chmod +x /usr/local/bin/ffmpeg && echo shim",
        )
        .expect("the shim would not be put in place");

    let conn = connect(&server).await;
    let variants = variant("stubborn.mp4");
    let cutting = one_variant(&conn, "stubborn", &variants);

    let started = cutting.start().await.expect("the cutting would not start");
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while live_named(&server, "sleep").is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "the stubborn member never started"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let begun = std::time::Instant::now();
    let how = cutting
        .stop(&started)
        .await
        .expect("the stop was not confirmed");
    let took = begun.elapsed();
    assert_eq!(
        how,
        Stopped::AfterKill,
        "a member that ignores TERM cannot have ended on TERM"
    );
    assert!(
        live_named(&server, "sleep").is_empty(),
        "the stop was reported confirmed and the member that ignores TERM is still alive"
    );
    assert!(!cutting
        .still_running()
        .await
        .expect("the server would not say"));
    // TERM's grace (5 s) and then the kill — not the kill's whole ceiling on top.
    assert!(
        took < Duration::from_secs(12),
        "the escalation took {took:?}"
    );
    eprintln!("T605 measured: escalation to KILL confirmed after {took:?}");
}

/// c) What `start()` records, checked around our own code: a live leader of a group and a
/// session of its own — not the SSH shell's group, which the launch ran in.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn start_records_a_group_of_its_own_and_refuses_a_second_start_beside_it() {
    let server = TestServer::start().expect("the container would not come up");
    make_slow_film(&server, "grouped.mp4").expect("the fixture film would not encode");

    let conn = connect(&server).await;
    let variants = variant("grouped.mp4");
    let cutting = one_variant(&conn, "grouped", &variants);

    let started = cutting.start().await.expect("the cutting would not start");
    let record = server
        .exec_inside(&format!("cat '{}'", cutting.pgid_path()))
        .expect("no group record on the server");
    let fields: Vec<&str> = record.split_whitespace().collect();
    assert_eq!(
        fields.len(),
        4,
        "the record is not `pid pgid sid job`: {record:?}"
    );
    let (pid, pgid, sid, job) = (fields[0], fields[1], fields[2], fields[3]);
    assert_eq!(pid, started.group.pid.to_string());
    assert_eq!(job, started.mark.as_str());
    assert_eq!(pid, pgid, "the wrapper does not lead its group");
    assert_eq!(pid, sid, "the wrapper did not get a session of its own");

    // The kernel's own view, not the record's: the leader is alive and is who it says.
    let kernel = server
        .exec_inside(&format!("ps -o pgid=,sid=,args= -p {pid}"))
        .expect("the recorded leader is not alive");
    let kernel: Vec<&str> = kernel.split_whitespace().collect();
    assert_eq!(
        kernel[0], pgid,
        "the kernel puts the leader in another group"
    );
    assert_eq!(kernel[1], sid);
    assert!(
        kernel.iter().any(|w| w.contains("vrcast-hls-grouped.sh")),
        "the recorded leader is not the cutting's wrapper: {kernel:?}"
    );

    // Not the group of the shell the launch ran in: that shell was a session leader under
    // sshd, and is gone by now — so what is checked is that the group is not that of any
    // sshd session still standing, and that `ffmpeg` is in the recorded one.
    let ffmpeg = wait_until_ffmpeg_runs(&server, Duration::from_secs(15)).await;
    let ffmpeg_group = server
        .exec_inside(&format!("ps -o pgid= -p {}", ffmpeg[0]))
        .expect("ffmpeg's group could not be read");
    assert_eq!(
        ffmpeg_group.trim(),
        pgid,
        "ffmpeg is not in the recorded group"
    );
    let sshd_groups = server
        .exec_inside("ps -eo pgid=,comm= | awk '$2 == \"sshd\" { print $1 }'")
        .expect("sshd's groups could not be read");
    assert!(
        !sshd_groups.split_whitespace().any(|g| g == pgid),
        "the recorded group is an SSH session's: {sshd_groups}"
    );
    let environ = server
        .exec_inside(&format!("tr '\\0' '\\n' < /proc/{}/environ", ffmpeg[0]))
        .expect("ffmpeg's environment could not be read");
    assert!(
        environ
            .lines()
            .any(|l| l == format!("VRCAST_HLS_JOB={job}")),
        "ffmpeg does not carry the start's mark, so a stop could not find it once the \
         wrapper is gone"
    );

    // A second start beside a live one is refused, and starts nothing.
    let second = cutting.start().await;
    assert!(
        matches!(second, Err(CuttingError::AlreadyRunning { .. })),
        "a second cutting of the same directory was started beside a live one: {second:?}"
    );
    assert_eq!(live_named(&server, "ffmpeg").len(), 1);

    cutting
        .stop(&started)
        .await
        .expect("the stop was not confirmed");
    cutting.tidy_up().await.expect("the leftovers would not go");
    assert!(
        server
            .exec_inside(&format!("test -e '{}'", cutting.pgid_path()))
            .is_err(),
        "tidy_up left the group record behind"
    );
}
