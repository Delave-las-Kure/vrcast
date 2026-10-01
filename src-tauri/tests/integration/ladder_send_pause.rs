//! T670(4) — a long pause while a ladder variant is being sent lets go of the file session on
//! the server, and carrying on writes on from where the sending stopped.
//!
//! **Why against a real container.** What is claimed is about the server: that the
//! `sftp-server` process the session ran is really gone while the pause lasts, that what was
//! handed over before the pause is really on the server at the size the sending counted, and
//! that the staged file is opened again without being truncated. None of that can be seen on
//! an invented writer — `tests/unit/ladder_build.rs::holding` checks the copying side alone.
//!
//! **How "written on, not started over" is proved.** While the session is closed the test
//! overwrites the first byte of the staged `.part` in place (its size unchanged). A sending
//! that carries on from where it stopped never writes that byte again, so the finished
//! variant starts with the marker; one that started over would have put the original byte
//! back.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use vrcast_studio_lib::commands::error::{AppError, ErrorCode};
use vrcast_studio_lib::domain::wording::DetailCode;
use vrcast_studio_lib::ssh::{fingerprint, Connection, Credentials, ServerAddress};
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::tasks::engine::{TaskEngine, TaskEvent};
use vrcast_studio_lib::tasks::ladder_build::send_file;
use vrcast_studio_lib::tasks::state::{TaskKind, TaskState};

use super::fixture::{key_path, TestServer, KEY_PASSPHRASE};

const DIR: &str = "/var/lib/vrcast/videos/t670";
const HOLD: Duration = Duration::from_secs(2);
const SIZE: usize = 96 * 1024 * 1024;

async fn connect(server: &TestServer) -> Connection {
    let addr = ServerAddress::new(server.host(), server.port);
    let fp = fingerprint::probe(&addr)
        .await
        .expect("the fingerprint was not obtained");
    Connection::connect(
        addr,
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

/// A variant-sized file of bytes that are not all alike, whose first byte is not the marker.
fn make_variant(dir: &Path) -> std::path::PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join("v0.mp4");
    let body: Vec<u8> = (0..SIZE).map(|i| (i % 251) as u8).collect();
    std::fs::write(&path, body).unwrap();
    path
}

fn sftp_sessions(server: &TestServer) -> usize {
    server
        .exec_inside("pgrep -c -x sftp-server || true")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

fn staged_size(server: &TestServer) -> Option<u64> {
    server
        .exec_inside(&format!("stat -c %s '{DIR}/v0.mp4.part'"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

/// Start the sending as a real task, and pause it once some of the variant has gone across.
async fn send_and_pause(
    server: &TestServer,
    local: &Path,
) -> (
    TaskEngine,
    String,
    tokio::sync::oneshot::Receiver<Result<(), String>>,
) {
    server
        .exec_inside(&format!("mkdir -p '{DIR}' && rm -f '{DIR}'/v0.mp4*"))
        .unwrap();
    let conn = connect(server).await;
    let engine = TaskEngine::new(Arc::new(Db::open_in_memory().unwrap()));
    let mut events = engine.subscribe();
    let (done, outcome) = tokio::sync::oneshot::channel();
    let local = local.to_path_buf();
    let id = engine
        .submit(TaskKind::BuildLadder, None, move |ctx| async move {
            let sent = send_file(&conn, &local, &format!("{DIR}/v0.mp4"), &ctx, HOLD).await;
            conn.close().await;
            let _ = done.send(sent.as_ref().map(|_| ()).map_err(|e| format!("{e:?}")));
            sent.map_err(|e| AppError::new(ErrorCode::Internal).with_cause(e))
        })
        .await
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        assert!(Instant::now() < deadline, "the sending never got going");
        match tokio::time::timeout(Duration::from_millis(200), events.recv()).await {
            Ok(Ok(TaskEvent::Progress {
                id: which,
                stage: Some(DetailCode::StageSendingVariant),
                progress,
                ..
            })) if which == id && progress > 0.0 => break,
            _ => {}
        }
    }
    engine.pause(&id).expect("the sending would not pause");
    (engine, id, outcome)
}

/// Wait out the hold, and check the session really went with it.
async fn after_the_hold(server: &TestServer) -> u64 {
    tokio::time::sleep(HOLD + Duration::from_secs(2)).await;
    assert_eq!(
        sftp_sessions(server),
        0,
        "the file session is still open on the server after a long pause"
    );
    let at = staged_size(server).expect("the staged variant is gone during the pause");
    assert!(
        at > 0 && at < SIZE as u64,
        "the pause did not land mid-variant: {at} of {SIZE}"
    );
    at
}

async fn final_state(engine: &TaskEngine, id: &str) -> TaskState {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let state = engine.get(id).unwrap().unwrap().state;
        if state.is_final() {
            return state;
        }
        assert!(Instant::now() < deadline, "the sending never ended");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_long_pause_closes_the_session_and_carrying_on_writes_on_from_where_it_stopped() {
    let server = TestServer::start().expect("the container would not come up");
    let work =
        std::env::temp_dir().join(format!("vrcast-t670-4-{}", uuid::Uuid::new_v4().simple()));
    let local = make_variant(&work);

    let (engine, id, outcome) = send_and_pause(&server, &local).await;
    // Within the hold the session is still there: a short pause costs nothing.
    assert!(
        sftp_sessions(&server) >= 1,
        "the session was dropped before the hold ran out"
    );
    let at = after_the_hold(&server).await;

    // The marker, in place: the size stays what was sent.
    server
        .exec_inside(&format!(
            "printf 'Z' | dd of='{DIR}/v0.mp4.part' bs=1 count=1 conv=notrunc status=none"
        ))
        .unwrap();
    assert_eq!(staged_size(&server), Some(at));

    engine.resume(&id).unwrap();
    assert_eq!(final_state(&engine, &id).await, TaskState::Completed);
    outcome.await.unwrap().expect("the sending failed");

    let size: u64 = server
        .exec_inside(&format!("stat -c %s '{DIR}/v0.mp4'"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_eq!(size, SIZE as u64, "the variant on the server is not whole");
    let first = server
        .exec_inside(&format!("head -c 1 '{DIR}/v0.mp4'"))
        .unwrap();
    assert_eq!(
        first, "Z",
        "the start of the variant was written again — the sending started over"
    );
    // Everything after the first byte is what was sent.
    let tail_there = server
        .exec_inside(&format!(
            "tail -c +2 '{DIR}/v0.mp4' | sha256sum | cut -d' ' -f1"
        ))
        .unwrap();
    let tail_here = {
        use sha2::Digest;
        let body = std::fs::read(&local).unwrap();
        hex::encode(sha2::Sha256::digest(&body[1..]))
    };
    assert_eq!(tail_there.trim(), tail_here);
    assert!(
        staged_size(&server).is_none(),
        "the staged .part was left behind"
    );
    let _ = std::fs::remove_dir_all(&work);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_staged_file_touched_during_the_pause_is_sent_again_whole() {
    let server = TestServer::start().expect("the container would not come up");
    let work =
        std::env::temp_dir().join(format!("vrcast-t670-4b-{}", uuid::Uuid::new_v4().simple()));
    let local = make_variant(&work);

    let (engine, id, outcome) = send_and_pause(&server, &local).await;
    let at = after_the_hold(&server).await;
    // Somebody cut it short: what is there is no longer what was sent.
    server
        .exec_inside(&format!("truncate -s {} '{DIR}/v0.mp4.part'", at - 1))
        .unwrap();

    engine.resume(&id).unwrap();
    assert_eq!(final_state(&engine, &id).await, TaskState::Completed);
    outcome.await.unwrap().expect("the sending failed");

    let there = server
        .exec_inside(&format!("sha256sum '{DIR}/v0.mp4' | cut -d' ' -f1"))
        .unwrap();
    let here = {
        use sha2::Digest;
        hex::encode(sha2::Sha256::digest(std::fs::read(&local).unwrap()))
    };
    assert_eq!(
        there.trim(),
        here,
        "a staged file that was not what was sent was written on rather than replaced"
    );
    let _ = std::fs::remove_dir_all(&work);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cancel_during_a_long_pause_still_removes_the_staged_file() {
    let server = TestServer::start().expect("the container would not come up");
    let work =
        std::env::temp_dir().join(format!("vrcast-t670-4c-{}", uuid::Uuid::new_v4().simple()));
    let local = make_variant(&work);

    let (engine, id, outcome) = send_and_pause(&server, &local).await;
    after_the_hold(&server).await;

    engine.cancel(&id).unwrap();
    assert_eq!(final_state(&engine, &id).await, TaskState::Cancelled);
    let said = outcome
        .await
        .unwrap()
        .expect_err("a cancelled sending succeeded");
    assert!(said.contains("Cancelled"), "{said}");
    assert!(
        staged_size(&server).is_none(),
        "the staged .part outlived a cancel during the pause"
    );
    assert!(
        server
            .exec_inside(&format!("test -e '{DIR}/v0.mp4'"))
            .is_err(),
        "a cancelled variant was put in place"
    );
    let _ = std::fs::remove_dir_all(&work);
}
