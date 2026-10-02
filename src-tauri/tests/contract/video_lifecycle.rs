//! QA-25 — a video's life from the controls' side: pause and carry on from either place, take
//! it off the list, remove its server, stop it across a restart, replace its set (T682, T683,
//! T685, T686, T687).
//!
//! The work is a task this test controls, filed under the video's batch exactly as the
//! measurement and the build are: the real engine, the real store and the real watcher of the
//! video commands, without a film or a server.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use vrcast_studio_lib::commands::servers::api as servers;
use vrcast_studio_lib::commands::video::{api as video, VideoView};
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::domain::video::{after_restart, AfterRestart, VideoStage, VideoState};
use vrcast_studio_lib::domain::wording::DetailCode;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;
use vrcast_studio_lib::store::videos::{self as rows, VideoRow};
use vrcast_studio_lib::tasks::state::{TaskKind, TaskState};
use vrcast_studio_lib::tasks::store::Batch;

use super::support::{state, valid_input};

/// A video on its way, at encoding, with its server.
fn going(state: &AppState, id: &str) -> VideoRow {
    let server = servers::server_add(state, valid_input(id), "in-memory-only").unwrap();
    let mut row = VideoRow::new(id, &server, "C:/nowhere/film.mp4", id, id);
    row.state = VideoState::Working;
    row.stage = VideoStage::Encoding;
    rows::save(&state.db, &row).unwrap();
    // The watcher of the task stream starts with the list, as it does on the screen.
    video::video_list(state).unwrap();
    row
}

async fn until(state: &AppState, id: &str, what: &str, ok: impl Fn(&VideoView) -> bool) {
    let waited = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if ok(&video::video_get(state, id).unwrap()) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    if waited.is_err() {
        let now = video::video_get(state, id).unwrap();
        panic!(
            "{what}: never got there; the video is {:?} at {:?}, paused_by_person={}, task {:?}",
            now.state,
            now.stage,
            now.paused_by_person,
            now.progress.map(|p| p.task_state)
        );
    }
}

/// A build of this video that reports what it is told to and ends when it is stopped.
async fn build(
    state: &AppState,
    row: &VideoRow,
) -> (String, mpsc::UnboundedSender<(f64, DetailCode)>) {
    let (tx, mut rx) = mpsc::unbounded_channel::<(f64, DetailCode)>();
    let task = state
        .tasks
        .submit_in_batch(
            TaskKind::BuildLadder,
            Some(row.server_id.clone()),
            Some(Batch {
                id: row.id.clone(),
                label: row.title.clone(),
            }),
            move |ctx| async move {
                ctx.report_important(0.1, DetailCode::StageConverting);
                loop {
                    tokio::select! {
                        _ = ctx.cancel_token().cancelled_owned() => break,
                        got = rx.recv() => match got {
                            Some((p, c)) => {
                                ctx.wait_while_paused().await;
                                ctx.report_important(p, c);
                            }
                            None => break,
                        }
                    }
                }
                Ok(())
            },
        )
        .await
        .unwrap();
    until(state, &row.id, "the build is the video's", |v| {
        v.task_id.as_deref() == Some(&task) && v.progress.is_some()
    })
    .await;
    (task, tx)
}

/// The same database, opened by a new run of the application.
fn restarted(state: &AppState) -> AppState {
    AppState::with_db(state.db.clone(), Arc::new(InMemorySecretStore::new())).unwrap()
}

// ---------- T685: pause and carry on, from the card or from «Tasks» ----------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pause_from_tasks_is_the_video_s_pause_and_survives_a_restart() {
    let state = state();
    let row = going(&state, "pause-from-tasks");
    let (task, _tx) = build(&state, &row).await;

    // «Pause» in «Tasks».
    state.tasks.pause(&task).unwrap();
    until(&state, &row.id, "the video paused with its task", |v| {
        v.state == VideoState::Paused && v.paused_by_person
    })
    .await;
    // Kept as a person's pause: a restart leaves it paused.
    let kept = rows::get(&state.db, &row.id).unwrap().unwrap();
    assert_eq!(kept.state, VideoState::Paused);
    assert!(kept.paused_by_person);
    assert_eq!(after_restart(kept.state), AfterRestart::Leave);
    let next = restarted(&state);
    assert_eq!(video::restore_videos(&next).unwrap(), 0);
    assert_eq!(
        video::video_get(&next, &row.id).unwrap().state,
        VideoState::Paused
    );
    state.tasks.cancel(&task).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn carrying_on_from_tasks_after_a_pause_on_the_card_carries_on() {
    let state = state();
    let row = going(&state, "resume-from-tasks");
    let (task, tx) = build(&state, &row).await;

    video::video_pause(&state, &row.id).unwrap();
    until(&state, &row.id, "the task paused with the video", |v| {
        v.progress
            .as_ref()
            .is_some_and(|p| p.task_state == TaskState::Paused)
    })
    .await;

    // «Continue» in «Tasks».
    state.tasks.resume(&task).unwrap();
    until(&state, &row.id, "the video going again", |v| {
        v.state == VideoState::Working && !v.paused_by_person
    })
    .await;
    // And it stays going: the watcher does not pause it back.
    tx.send((0.5, DetailCode::StageConverting)).unwrap();
    until(&state, &row.id, "the build reporting again", |v| {
        v.progress
            .as_ref()
            .is_some_and(|p| p.progress == 0.5 && p.task_state == TaskState::Running)
    })
    .await;
    assert_eq!(
        state.tasks.get(&task).unwrap().unwrap().state,
        TaskState::Running
    );
    assert_eq!(
        video::video_get(&state, &row.id).unwrap().state,
        VideoState::Working
    );
    state.tasks.cancel(&task).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pause_on_the_card_carried_on_from_the_card_and_paused_again_from_tasks() {
    let state = state();
    let row = going(&state, "pause-both-ways");
    let (task, _tx) = build(&state, &row).await;

    video::video_pause(&state, &row.id).unwrap();
    until(&state, &row.id, "the task paused", |v| {
        v.progress
            .as_ref()
            .is_some_and(|p| p.task_state == TaskState::Paused)
    })
    .await;
    video::video_resume(&state, &row.id).unwrap();
    until(&state, &row.id, "going again", |v| {
        v.state == VideoState::Working
            && v.progress
                .as_ref()
                .is_some_and(|p| p.task_state == TaskState::Running)
    })
    .await;
    state.tasks.pause(&task).unwrap();
    until(&state, &row.id, "paused from «Tasks»", |v| {
        v.state == VideoState::Paused && v.paused_by_person
    })
    .await;
    state.tasks.cancel(&task).unwrap();
}

// ---------- T683: taking a video off the list, removing its server ----------

/// Nothing of the video's work is alive, and nothing of it is left looking unfinished.
fn nothing_left(state: &AppState, video_id: &str, task: &str) {
    assert!(!state.tasks.is_alive(task), "the task is still alive");
    let unfinished =
        vrcast_studio_lib::tasks::store::unfinished_in_batch(&state.db, video_id).unwrap();
    assert!(
        unfinished.is_empty(),
        "rows of the video's tasks look unfinished: {unfinished:?}"
    );
    assert!(rows::get(&state.db, video_id).unwrap().is_none());
}

async fn gone(state: &AppState, id: &str, task: &str) {
    let waited = tokio::time::timeout(Duration::from_secs(10), async {
        while rows::get(&state.db, id).unwrap().is_some() || state.tasks.is_alive(task) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(
        waited.is_ok(),
        "never gone: video {:?}, task alive {}",
        rows::get(&state.db, id).unwrap().map(|r| r.state),
        state.tasks.is_alive(task)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn removing_a_paused_video_stops_its_work_then_takes_it_off() {
    let state = state();
    let row = going(&state, "remove-paused");
    let (task, _tx) = build(&state, &row).await;
    video::video_pause(&state, &row.id).unwrap();
    let mut events = state.subscribe();

    let answer = video::video_remove(&state, &row.id).unwrap();
    // Not gone at once: its work is being stopped, as after «Cancel».
    if let Some(view) = answer {
        assert_eq!(view.state, VideoState::Cancelling);
    }
    gone(&state, &row.id, &task).await;
    nothing_left(&state, &row.id, &task);
    assert_eq!(
        state.tasks.get(&task).unwrap().unwrap().state,
        TaskState::Cancelled
    );
    // The screen is told it is gone.
    let told = std::iter::from_fn(|| events.try_recv().ok()).any(|e| {
        matches!(e, vrcast_studio_lib::commands::AppEvent::VideoRemoved { ref id } if id == &row.id)
    });
    assert!(told, "video:removed never went out");
    // Nothing left to resume from «Tasks».
    assert!(state.tasks.resume(&task).is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn removing_a_working_video_stops_its_work_then_takes_it_off() {
    let state = state();
    let row = going(&state, "remove-working");
    let (task, _tx) = build(&state, &row).await;
    video::video_remove(&state, &row.id).unwrap();
    gone(&state, &row.id, &task).await;
    nothing_left(&state, &row.id, &task);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_video_being_removed_across_a_restart_is_gone_after_it() {
    let state = state();
    let mut row = going(&state, "remove-restart");
    row.state = VideoState::Cancelling;
    row.remove_requested = true;
    rows::save(&state.db, &row).unwrap();
    let next = restarted(&state);
    video::restore_videos(&next).unwrap();
    let waited = tokio::time::timeout(Duration::from_secs(10), async {
        while rows::get(&next.db, &row.id).unwrap().is_some() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(waited.is_ok(), "the video asked to be removed came back");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn removing_a_server_with_work_on_it_asks_then_stops_it_all_first() {
    use vrcast_studio_lib::commands::error::{DetailCode as Code, ErrorCode};

    let state = state();
    let row = going(&state, "remove-server");
    let (task, _tx) = build(&state, &row).await;

    // Asked first; nothing changes.
    let asked = servers::server_remove(&state, &row.server_id, false)
        .await
        .unwrap_err();
    assert_eq!(asked.code, ErrorCode::ConfirmationRequired);
    assert_eq!(asked.details[0].key, Code::ConfirmStopServerWork);
    assert_eq!(asked.details[0].params["count"], serde_json::json!(1));
    assert!(state.tasks.is_alive(&task));
    assert!(rows::get(&state.db, &row.id).unwrap().is_some());
    assert_eq!(servers::servers_list(&state).unwrap().len(), 1);

    // Confirmed: everything there stops, then the profile goes — never the other way round.
    let mut events = state.subscribe();
    servers::server_remove(&state, &row.server_id, true)
        .await
        .unwrap();
    assert!(!state.tasks.is_alive(&task), "the task outlived its server");
    assert!(servers::servers_list(&state).unwrap().is_empty());
    assert!(rows::get(&state.db, &row.id).unwrap().is_none());
    assert!(vrcast_studio_lib::tasks::store::get(&state.db, &task)
        .unwrap()
        .is_none());
    let told = std::iter::from_fn(|| events.try_recv().ok()).any(|e| {
        matches!(e, vrcast_studio_lib::commands::AppEvent::VideoRemoved { ref id } if id == &row.id)
    });
    assert!(told, "the server's video never left the screen");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn removing_a_server_with_nothing_running_asks_nothing() {
    let state = state();
    let row = going(&state, "remove-idle-server");
    let mut idle = rows::get(&state.db, &row.id).unwrap().unwrap();
    idle.state = VideoState::Ready;
    rows::save(&state.db, &idle).unwrap();
    servers::server_remove(&state, &row.server_id, false)
        .await
        .unwrap();
    assert!(rows::get(&state.db, &row.id).unwrap().is_none());
}
