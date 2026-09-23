//! T605 — a build whose cutting could not be confirmed stopped on the server is not written
//! down as ended, and keeps its hold on the media's directory, until the stop is confirmed.
//!
//! **Why here, with a stand-in for the server, and not by cutting a real network.** What is
//! checked is the rule between three parts that do not need a server to meet:
//! `ladder_build::settle_stop` (does not return until a stop attempt says "confirmed"), the
//! real `TaskEngine` (writes `Cancelled`/`Failed` only once the work returns) and the real
//! `running_build_for` guard (reads the task's state out of the same engine). A real cut
//! (`docker network disconnect`) would add minutes of waiting for `russh`'s keepalive
//! (~120 s, measured in `upload_finish_network_cut.rs`) and prove nothing more about this
//! rule; whether the stop itself works on a real server — TERM, KILL, the wrapper dead
//! before `ffmpeg` — is `tests/integration/hls_cutting_cancel.rs`'s job.
//!
//! The guard is asked through the public command, `ladder_build`, which refuses a second
//! build of a slug while `running_build_for` still finds one — before any connection is
//! made (`confirmed: true` skips the only step before it that would reach a server).

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use vrcast_studio_lib::commands::api as core;
use vrcast_studio_lib::commands::error::{AppError, ErrorCode};
use vrcast_studio_lib::commands::ladder::{api as ladder, BuildRequest};
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::domain::ladder::{Quality, Rung};
use vrcast_studio_lib::domain::server_profile::{AuthKind, ServerProfile};
use vrcast_studio_lib::server::hls_package::{CuttingError, JobMark, PendingStop, Stopped};
use vrcast_studio_lib::ssh::SshError;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;
use vrcast_studio_lib::tasks::ladder_build::{settle_stop, BuildError};
use vrcast_studio_lib::tasks::state::{TaskKind, TaskState};

const SERVER: &str = "t605-server";
const SLUG: &str = "t605";

fn app_state() -> AppState {
    let state = AppState::with_db(
        Arc::new(Db::open_in_memory().unwrap()),
        Arc::new(InMemorySecretStore::new()),
    )
    .expect("the application state would not assemble");
    vrcast_studio_lib::store::profiles::insert(
        &state.db,
        &ServerProfile {
            id: SERVER.to_owned(),
            name: String::from("T605"),
            host: String::from("192.0.2.1"),
            port: 22,
            user: String::from("root"),
            auth_kind: AuthKind::Password,
            secret_ref: String::from("t605"),
            key_path: None,
            domain: String::from("stream.example.com"),
            video_dir: String::from("/var/lib/vrcast/videos"),
            cdn_base: None,
            host_fingerprint: None,
            ipv6_mode: None,
            is_active: true,
        },
    )
    .expect("the profile would not be written");
    state
}

/// A second build of the same slug — refused while the first still holds it.
async fn second_build(state: &AppState) -> Result<String, AppError> {
    ladder::ladder_build(
        state,
        BuildRequest {
            server_id: SERVER.to_owned(),
            path: String::from("Z:/t605/does-not-matter.mp4"),
            slug: SLUG.to_owned(),
            rungs: vec![Rung {
                index: 0,
                bitrate_bps: 500_000,
                maxrate_bps: 550_000,
                bufsize_bps: 550_000,
                width: 320,
                height: 240,
                level: String::from("3.0"),
                reasons: Vec::new(),
                quality: Quality::MeasuredHere { vmaf_x100: 9200 },
            }],
            audio_track: 0,
            prefer_hardware: false,
            batch: None,
            confirmed: true,
        },
    )
    .await
}

async fn wait_for_final(state: &AppState, id: &str, limit: Duration) -> TaskState {
    let deadline = std::time::Instant::now() + limit;
    loop {
        let task = core::task_get(state, id).expect("the task vanished");
        if task.state.is_final() {
            return task.state;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the task never ended although the stop was confirmed (state {:?})",
            task.state
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Submit a build task whose cutting ended as `then` with its stop unconfirmed, settled by
/// a stand-in stopper that confirms only once `server_back` is raised.
async fn submit_unconfirmed(
    state: &AppState,
    then: CuttingError,
    server_back: Arc<AtomicBool>,
    attempts: Arc<AtomicUsize>,
) -> String {
    let id = state
        .tasks
        .submit(
            TaskKind::BuildLadder,
            Some(SERVER.to_owned()),
            move |ctx| async move {
                // A cancellation's stop is asked only once the cancel comes; here it is asked,
                // and not answered — the connection is gone.
                if matches!(then, CuttingError::Cancelled) {
                    ctx.cancel_token().cancelled().await;
                }
                let outcome: Result<(), BuildError> = Err(BuildError::StopUnconfirmed(
                    CuttingError::StopUnconfirmed(PendingStop::new(
                        JobMark::for_base(SLUG),
                        then,
                        "the connection died before the stop could be asked",
                    )),
                ));
                let settled = settle_stop(outcome, |_mark| {
                    let server_back = server_back.clone();
                    let attempts = attempts.clone();
                    async move {
                        attempts.fetch_add(1, Ordering::SeqCst);
                        if server_back.load(Ordering::SeqCst) {
                            Ok(Stopped::AfterKill)
                        } else {
                            Err(String::from("could not reach the server"))
                        }
                    }
                })
                .await;
                settled.map_err(|e| AppError::new(ErrorCode::Internal).with_cause(e))
            },
        )
        .await
        .expect("the task was not submitted");
    // The same marker `ladder_build` writes for `running_build_for` to find.
    vrcast_studio_lib::tasks::store::save_resume_token(&state.db, &id, SLUG).unwrap();
    id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unconfirmed_stop_is_not_cancelled_and_keeps_the_slug_until_it_is_confirmed() {
    let state = app_state();
    let server_back = Arc::new(AtomicBool::new(false));
    let attempts = Arc::new(AtomicUsize::new(0));
    let id = submit_unconfirmed(
        &state,
        CuttingError::Cancelled,
        server_back.clone(),
        attempts.clone(),
    )
    .await;

    // Cancelled once the work is under way — a task cancelled while still queued is dropped
    // before its work ever runs, which is not the case here.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while core::task_get(&state, &id).unwrap().state != TaskState::Running {
        assert!(
            std::time::Instant::now() < deadline,
            "the task never started"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    state.tasks.cancel(&id).expect("the cancel was refused");

    // Past the first retry (2 s): at least one more attempt has failed by now.
    tokio::time::sleep(Duration::from_millis(3_000)).await;
    let task = core::task_get(&state, &id).unwrap();
    assert_eq!(
        task.state,
        TaskState::Running,
        "the build was written down as ended although nobody has confirmed that its \
         processes on the server are gone — constitution III"
    );
    assert!(
        attempts.load(Ordering::SeqCst) >= 1,
        "the unconfirmed stop was never tried again"
    );
    let refused = second_build(&state)
        .await
        .expect_err("the slug was let go while the cutting may still be writing into it");
    assert_eq!(refused.code, ErrorCode::NameExists);

    // The server answers again: the next attempt confirms, and only then does it end.
    server_back.store(true, Ordering::SeqCst);
    let ended = wait_for_final(&state, &id, Duration::from_secs(15)).await;
    assert_eq!(ended, TaskState::Cancelled);

    // And the slug is free: the refusal is gone (the build then fails for its own reasons —
    // there is no source file — which is not what is asked here).
    if let Err(e) = second_build(&state).await {
        assert_ne!(
            e.code,
            ErrorCode::NameExists,
            "the slug is still held after the stop was confirmed and the task ended"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failure_with_an_unconfirmed_stop_is_not_failed_until_it_is_confirmed() {
    // `Failed` releases the slug exactly as `Cancelled` does, so the same holds for it.
    let state = app_state();
    let server_back = Arc::new(AtomicBool::new(false));
    let attempts = Arc::new(AtomicUsize::new(0));
    let id = submit_unconfirmed(
        &state,
        CuttingError::Ssh(SshError::Exec(String::from(
            "the cutting stopped: v1: boom",
        ))),
        server_back.clone(),
        attempts.clone(),
    )
    .await;

    tokio::time::sleep(Duration::from_millis(3_000)).await;
    assert_eq!(
        core::task_get(&state, &id).unwrap().state,
        TaskState::Running,
        "a failure was written down while the stop on the server was still unconfirmed"
    );
    assert_eq!(
        second_build(&state)
            .await
            .expect_err("the slug was let go")
            .code,
        ErrorCode::NameExists
    );

    server_back.store(true, Ordering::SeqCst);
    assert_eq!(
        wait_for_final(&state, &id, Duration::from_secs(15)).await,
        TaskState::Failed
    );
}
