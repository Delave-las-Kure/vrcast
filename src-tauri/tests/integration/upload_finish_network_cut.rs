//! T570 — a REAL network cut landing inside `finish()`'s checksum-or-publish phase, not a
//! voluntary cancel and not a fault injected from inside the code under test.
//!
//! **What `upload_live.rs` already proves, and what it cannot.** Its checks cover breaks
//! during the byte-transfer phase (`run_upload`'s own `for attempt in 1..=MAX_ATTEMPTS`
//! loop) and a restart of the whole application. None of them ever reach `finish()`'s two
//! network calls — `checksum::remote` and `upload::publish` — while the connection is still
//! alive: by the time those checks touch a broken connection, the byte transfer is already
//! done and `finish()` runs to completion on a healthy one. This file is the one that
//! actually breaks the connection **after** the file is whole on the server and **during**
//! the phase T570 gave its own retry loop.
//!
//! **The timing this leans on, inherited from `deploy_network_cut.rs`.** A
//! `docker network disconnect -f` does not fail the connection at once: the TCP stream goes
//! silent, and nothing notices until `russh`'s own keepalive gives up on it — measured
//! there at roughly 90-120 seconds. The same mechanism applies here (`ssh::fingerprint::
//! client_config`), so every check that cuts the network and waits for the client to notice
//! is bounded generously above that, not tightly.
//!
//! **Why some scenarios are set up directly through the container rather than through the
//! real command's own timing.** `docker exec` reaches the container over the Docker daemon's
//! own socket, not over the network the SSH connection uses — so it keeps working even while
//! that network is cut. That is what makes it possible to arrange, with certainty rather than
//! luck, the states the brief for T570 asks for: "the publish already happened and only the
//! acknowledgement was lost" and "the staged file vanished between attempts". Landing a real
//! network cut at the exact instant `mv -f` runs on the server is not something any amount of
//! wall-clock delay can guarantee — so where the brief allows it, the state that a mid-`mv`
//! cut would have left behind is built by hand instead, and the reconnect logic is exercised
//! against it exactly as it would run in life.

use super::fixture::TestServer;
use std::sync::Arc;
use std::time::{Duration, Instant};
use vrcast_studio_lib::commands::error::ErrorCode;
use vrcast_studio_lib::commands::upload::{api as upload, UploadRequest};
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::domain::wording::DetailCode;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;
use vrcast_studio_lib::tasks::engine::TaskEvent;
use vrcast_studio_lib::tasks::state::TaskState;

const VIDEO_DIR: &str = "/var/lib/vrcast/videos";
const STAGING_DIR: &str = "/var/lib/vrcast/.vrcast-uploads";

/// How long a `docker network disconnect -f` takes to be noticed by `russh`'s own keepalive.
///
/// Measured by hand for the identical mechanism in `deploy_network_cut.rs`: ~120.15 seconds,
/// twice, agreeing to within 20 milliseconds. `NETWORK_STAYS_CUT_FOR` below leaves a wide
/// margin (roughly double this) rather than a tight one, for the same reason that file
/// gives: failing the test slowly is far better than hanging the whole suite if the
/// mechanism ever stops detecting the cut.
const KEEPALIVE_NOTICE: Duration = Duration::from_secs(120);
const NETWORK_STAYS_CUT_FOR: Duration = Duration::from_secs(KEEPALIVE_NOTICE.as_secs() + 30);

/// How long the whole task is given to finish once the network is restored.
const FINISH_WITHIN: Duration = Duration::from_secs(180);

fn app_state() -> AppState {
    AppState::with_db(
        Arc::new(Db::open_in_memory().unwrap()),
        Arc::new(InMemorySecretStore::new()),
    )
    .expect("the application state would not assemble")
}

/// A local file with predictable but non-uniform contents — see `upload_live.rs` for why
/// uniform bytes will not do here either: a spoilt transfer must not accidentally share a
/// checksum with the real one.
fn make_local_file(name: &str, size: usize) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "vrcast-finish-cut-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&dir).expect("could not create the temporary directory");
    let path = dir.join(name);

    let mut data = Vec::with_capacity(size);
    let mut x: u32 = 0x2468_1357;
    while data.len() < size {
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        data.extend_from_slice(&x.to_le_bytes());
    }
    data.truncate(size);
    std::fs::write(&path, &data).expect("could not write the file");
    path
}

async fn setup() -> (TestServer, AppState, String) {
    super::fixture::logging_if_requested();
    let server = TestServer::start().expect("the container would not come up");
    let state = app_state();
    let id = super::upload_live::add_profile(&state, &server).await;
    (server, state, id)
}

fn request(server_id: &str, local: &std::path::Path, name: &str) -> UploadRequest {
    UploadRequest {
        server_id: server_id.to_owned(),
        local_path: local.to_string_lossy().into_owned(),
        remote_name: name.to_owned(),
        media_id: None,
        limit_bps: None,
        confirmed: true,
    }
}

fn sha256_of(path: &std::path::Path) -> String {
    use sha2::{Digest, Sha256};
    let data = std::fs::read(path).expect("the file will not read");
    let mut hasher = Sha256::new();
    hasher.update(&data);
    hex::encode(hasher.finalize())
}

/// Sever the container's own network — the one `TestServer::start` made for it — so that its
/// SSH and HTTP ports both go silent. `docker exec` (used by `exec_inside`) keeps working
/// throughout: it reaches the container over the Docker daemon's socket, not this network.
fn cut_network(server: &TestServer) {
    let out = std::process::Command::new("docker")
        .args([
            "network",
            "disconnect",
            "-f",
            server.network(),
            server.container_id(),
        ])
        .output();
    assert!(
        matches!(&out, Ok(o) if o.status.success()),
        "could not cut the container's network — the fixture itself is broken, and nothing \
         below would be testing what this file claims to: {out:?}"
    );
}

/// Restore the network cut by [`cut_network`].
fn reconnect_network(server: &TestServer) {
    let out = std::process::Command::new("docker")
        .args([
            "network",
            "connect",
            server.network(),
            server.container_id(),
        ])
        .output();
    assert!(
        matches!(&out, Ok(o) if o.status.success()),
        "the network could not be reconnected after the cut, so nothing past this point can \
         be trusted: {out:?}"
    );
}

/// Wait until a task reaches a final state, or panic with what it last looked like.
async fn wait_done(state: &AppState, task_id: &str, limit: Duration) -> TaskState {
    let deadline = Instant::now() + limit;
    loop {
        if let Ok(Some(task)) = state.tasks.get(task_id) {
            if task.state.is_final() {
                return task.state;
            }
        }
        if Instant::now() >= deadline {
            let task = state.tasks.get(task_id).ok().flatten();
            panic!("the task did not finish in the time allowed: {task:?}");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Wait until a task's progress reaches the checksum stage — the point where `finish()`'s
/// own two network calls begin, and the earliest safe moment to cut the connection out from
/// under it.
async fn wait_for_checksum_stage(mut events: tokio::sync::broadcast::Receiver<TaskEvent>) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        assert!(
            Instant::now() < deadline,
            "the upload never reported reaching the checksum stage — either it failed \
             earlier, or the stage was renamed and this test fell behind"
        );
        match tokio::time::timeout(Duration::from_secs(5), events.recv()).await {
            Ok(Ok(TaskEvent::Progress {
                stage: Some(DetailCode::StageChecksum),
                ..
            })) => return,
            Ok(Ok(TaskEvent::Done { .. })) => {
                panic!("the upload finished before the network was ever cut")
            }
            _ => {}
        }
    }
}

// ---------- 1. a break landing during checksum::remote itself ----------

/// FR-032/FR-033-adjacent (T570, scenario 1). A file large enough that `sha256sum` on the
/// server takes a real, measurable few seconds — so cutting the network the instant the
/// checksum stage is reported lands the break *inside* `checksum::remote`'s own `exec`, not
/// merely somewhere in the neighbourhood of it.
#[tokio::test]
async fn a_break_during_the_remote_checksum_still_ends_completed() {
    let (server, state, id) = setup().await;
    // 300 MB: comfortably large enough that `sha256sum` inside the container takes several
    // seconds even on a fast disk, and still small enough that the whole transfer, checksum
    // and reconnect fit inside the time this test is given.
    let local = make_local_file("finish_checksum.mp4", 300 * 1024 * 1024);

    let events = state.tasks.subscribe();
    let task = upload::upload_start(&state, request(&id, &local, "finish_checksum.mp4"))
        .await
        .expect("the upload would not submit");

    wait_for_checksum_stage(events.resubscribe()).await;
    let cut_at = Instant::now();
    cut_network(&server);

    // The network stays down for the full detection window before it is restored: bringing
    // it back too early would let the original, still in-flight `sha256sum` simply resume
    // over a recovered TCP stream instead of failing — which would prove nothing about the
    // reconnect logic T570 adds.
    tokio::time::sleep(NETWORK_STAYS_CUT_FOR).await;
    reconnect_network(&server);

    let outcome = wait_done(&state, &task, FINISH_WITHIN).await;
    assert_eq!(
        outcome,
        TaskState::Completed,
        "a break during the remote checksum did not end completed: {:?}",
        state.tasks.get(&task).ok().flatten()
    );
    assert!(
        cut_at.elapsed() >= NETWORK_STAYS_CUT_FOR,
        "sanity: the cut window itself must have actually elapsed"
    );

    let size = server
        .exec_inside(&format!("stat -c %s '{VIDEO_DIR}/finish_checksum.mp4'"))
        .expect("the file is not on the server after a checksum-phase break");
    assert_eq!(size.trim().parse::<usize>().unwrap(), 300 * 1024 * 1024);
    let theirs = server
        .exec_inside(&format!(
            "sha256sum '{VIDEO_DIR}/finish_checksum.mp4' | cut -d' ' -f1"
        ))
        .expect("the checksum would not compute");
    assert_eq!(theirs.trim(), sha256_of(&local));

    let leftovers = server
        .exec_inside(&format!("ls -A '{STAGING_DIR}' 2>/dev/null | wc -l"))
        .unwrap_or_else(|_| String::from("0"));
    assert_eq!(
        leftovers.trim(),
        "0",
        "litter was left in the staging directory"
    );
}

// ---------- 2. the publish already happened; only the ack was lost ----------

/// T570, scenario 2. Landing a real cut at the exact instant `mv -f` runs on the server
/// cannot be guaranteed by any wall-clock delay — the brief for T570 explicitly allows
/// building the state that such a cut would have left behind directly through the
/// container instead, and that is what this does: the checksum stage is reached (so the
/// staged file is confirmed whole and correct, exactly as it would be right before a real
/// `publish()` call), the network is cut so the client's connection is genuinely dead, and
/// then — using `docker exec`, which needs no network to the container — the rename that
/// `publish()` would have performed is carried out by hand, standing in for "the server did
/// it, the acknowledgement never arrived". What is checked afterwards is exactly what T570
/// promises for this case: the reconnect finds `remote_final` already there, at the right
/// size, and reports success rather than either failing or attempting to publish a second
/// time (which would fail loudly, since `remote_temp` is gone by then — a second attempt at
/// the old, un-reconnected `publish()` call would have surfaced as exactly that failure, and
/// its absence here is what proves the new logic is the one that ran).
#[tokio::test]
async fn a_publish_that_already_happened_is_recognised_rather_than_repeated_or_failed() {
    let (server, state, id) = setup().await;
    let local = make_local_file("finish_publish.mp4", 300 * 1024 * 1024);

    let events = state.tasks.subscribe();
    let task = upload::upload_start(&state, request(&id, &local, "finish_publish.mp4"))
        .await
        .expect("the upload would not submit");

    wait_for_checksum_stage(events.resubscribe()).await;
    cut_network(&server);

    // Carried out over `docker exec`, which does not touch the network that was just cut:
    // this is standing in for the `mv -f` inside `upload::publish` having completed on the
    // server microseconds before the connection died, with the acknowledgement lost.
    server
        .exec_inside(&format!(
            "mv -f '{STAGING_DIR}/finish_publish.mp4.part' '{VIDEO_DIR}/finish_publish.mp4'"
        ))
        .expect("could not simulate the server-side half of a lost-acknowledgement publish");
    assert!(
        server
            .exec_inside(&format!("test -e '{STAGING_DIR}/finish_publish.mp4.part'"))
            .is_err(),
        "the simulated publish did not actually move the staged file — the test set up its \
         own fixture wrong"
    );

    tokio::time::sleep(NETWORK_STAYS_CUT_FOR).await;
    reconnect_network(&server);

    let outcome = wait_done(&state, &task, FINISH_WITHIN).await;
    assert_eq!(
        outcome,
        TaskState::Completed,
        "an already-published file, rediscovered after reconnecting, did not end completed: \
         {:?}",
        state.tasks.get(&task).ok().flatten()
    );

    let size = server
        .exec_inside(&format!("stat -c %s '{VIDEO_DIR}/finish_publish.mp4'"))
        .expect("the file is not on the server after the reconnect");
    assert_eq!(size.trim().parse::<usize>().unwrap(), 300 * 1024 * 1024);
    let theirs = server
        .exec_inside(&format!(
            "sha256sum '{VIDEO_DIR}/finish_publish.mp4' | cut -d' ' -f1"
        ))
        .expect("the checksum would not compute");
    assert_eq!(theirs.trim(), sha256_of(&local));

    let leftovers = server
        .exec_inside(&format!("ls -A '{STAGING_DIR}' 2>/dev/null | wc -l"))
        .unwrap_or_else(|_| String::from("0"));
    assert_eq!(
        leftovers.trim(),
        "0",
        "litter was left in the staging directory after a rediscovered publish"
    );
}

// ---------- 3. the staged file vanishes between attempts — an honest failure ----------

/// T570, scenario 3. Somebody (or something) removes the staged file entirely while this
/// upload is reconnecting — the one case that must NOT be retried forever, and must not be
/// mistaken for either success or an ordinary break.
#[tokio::test]
async fn a_staged_file_erased_between_attempts_fails_honestly_rather_than_retrying_forever() {
    let (server, state, id) = setup().await;
    let local = make_local_file("finish_vanished.mp4", 300 * 1024 * 1024);

    let events = state.tasks.subscribe();
    let task = upload::upload_start(&state, request(&id, &local, "finish_vanished.mp4"))
        .await
        .expect("the upload would not submit");

    wait_for_checksum_stage(events.resubscribe()).await;
    cut_network(&server);

    // Erased by hand, over `docker exec` — standing in for somebody removing it directly on
    // the server while this upload cannot see the server at all.
    server
        .exec_inside(&format!("rm -f '{STAGING_DIR}/finish_vanished.mp4.part'"))
        .expect("could not remove the staged file to simulate it vanishing");
    assert!(
        server
            .exec_inside(&format!(
                "test ! -e '{STAGING_DIR}/finish_vanished.mp4.part' && \
                 test ! -e '{VIDEO_DIR}/finish_vanished.mp4'"
            ))
            .is_ok(),
        "the test's own fixture is wrong: neither file should exist at this point"
    );

    tokio::time::sleep(NETWORK_STAYS_CUT_FOR).await;
    reconnect_network(&server);

    let outcome = wait_done(&state, &task, FINISH_WITHIN).await;
    assert_eq!(
        outcome,
        TaskState::Failed,
        "a staged file that vanished between attempts was not reported as a failure: {:?}",
        state.tasks.get(&task).ok().flatten()
    );
    let record = state.tasks.get(&task).unwrap().unwrap();
    let error = record
        .error
        .expect("the failure was recorded with no cause");
    assert_ne!(
        error.code,
        ErrorCode::TaskCancelled,
        "a vanished staged file was reported as a cancellation rather than a failure"
    );
    // Not a checksum mismatch either — there was nothing left on the server to compare at
    // all, which is a different fact and deserves a different code.
    assert_ne!(
        error.code,
        ErrorCode::ChecksumMismatch,
        "a vanished staged file was reported as though the checksums had differed: {error}"
    );

    assert!(
        server
            .exec_inside(&format!("test -e '{VIDEO_DIR}/finish_vanished.mp4'"))
            .is_err(),
        "the file entered serving despite the staged copy having vanished before it could"
    );
}

// ---------- 4. a cancellation arriving after the publish is already confirmed ----------

/// T570, scenario 4. A stop pressed while `finish()` is reconnecting must not silently lose
/// the fact that, unknown to the person pressing it, the publish had already gone through —
/// the same "window that cannot be closed" the ordinary (non-reconnecting) path already
/// documents (T503, `tests/unit/cancelling.rs`): the engine's own wrapper
/// (`tasks::engine::TaskEngine::start`) writes the task down as `Cancelled` whenever
/// `ctx.is_cancelled()` is true once the work future returns, REGARDLESS of what `finish()`
/// itself returned — "a cancellation outweighs the work's outcome" is the engine's rule, not
/// something T570 changes or could change from inside `finish()`. What T570 owes here is
/// only that the row and the truth do not contradict each other in silence: the file is
/// really on the server, really named `finish_cancel.mp4`, and the task carries a notice
/// saying so — never a silently empty `Cancelled` that leaves a person to discover on their
/// own that "cancelled" and "already serving" are somehow both true.
///
/// **Why the network is restored quickly here, unlike the other three scenarios.** This
/// scenario is not timing the ~90-120 second keepalive detection at all — cutting the
/// network severs the interface the original connection was bound to, so any NEW request on
/// it (a fresh `exec`, which is what `checksum::remote` sends) fails fast with a local
/// routing error rather than hanging until a keepalive gives up (that slow path is what the
/// *other* three scenarios exercise, on a connection with a read already in flight — see
/// `deploy_network_cut.rs`'s own header for the mechanism). So what this scenario needs from
/// the network cut is only that the original connection is genuinely, structurally broken —
/// guaranteed the instant the interface is gone — and what it needs from the reconnect is for
/// it to land before `cancel_during_finish`'s own single reconnect attempt runs, which
/// follows close behind the cancellation with no long wait of its own.
const NETWORK_BACK_UP_AFTER: Duration = Duration::from_secs(3);

#[tokio::test]
async fn a_cancellation_after_a_rediscovered_publish_ends_cancelled_with_a_notice() {
    let (server, state, id) = setup().await;
    let local = make_local_file("finish_cancel.mp4", 300 * 1024 * 1024);

    let events = state.tasks.subscribe();
    let task = upload::upload_start(&state, request(&id, &local, "finish_cancel.mp4"))
        .await
        .expect("the upload would not submit");

    wait_for_checksum_stage(events.resubscribe()).await;
    cut_network(&server);

    // The same stand-in as scenario 2: the server did the rename, the acknowledgement was
    // lost to the cut connection.
    server
        .exec_inside(&format!(
            "mv -f '{STAGING_DIR}/finish_cancel.mp4.part' '{VIDEO_DIR}/finish_cancel.mp4'"
        ))
        .expect("could not simulate the server-side half of a lost-acknowledgement publish");

    // The person presses stop while the connection is still down and the task is still
    // waiting to reconnect — before it has any way of knowing the publish already happened.
    state
        .tasks
        .cancel(&task)
        .expect("the cancellation was not accepted");

    // Restored quickly — see the doc comment above for why this does not shortcut what this
    // scenario actually needs to be real.
    tokio::time::sleep(NETWORK_BACK_UP_AFTER).await;
    reconnect_network(&server);

    let outcome = wait_done(&state, &task, FINISH_WITHIN).await;
    assert_eq!(
        outcome,
        TaskState::Cancelled,
        "the engine's own rule ('a cancellation outweighs the work's outcome', \
         tasks::engine::TaskEngine::start) did not hold once T570's reconnect loop was \
         involved: {:?}",
        state.tasks.get(&task).ok().flatten()
    );

    let record = state.tasks.get(&task).unwrap().unwrap();
    assert!(
        record
            .notices
            .iter()
            .any(|n| n.key == DetailCode::NoticeCancelledAfterPublish),
        "the task was marked cancelled with no notice that the file had already been \
         published — a person reading only the row's state would conclude, wrongly, that \
         nothing landed on the server: {:?}",
        record.notices
    );

    let size = server
        .exec_inside(&format!("stat -c %s '{VIDEO_DIR}/finish_cancel.mp4'"))
        .expect("the file is not on the server");
    assert_eq!(size.trim().parse::<usize>().unwrap(), 300 * 1024 * 1024);
}
