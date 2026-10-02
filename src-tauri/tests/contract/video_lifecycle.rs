//! QA-25 — a video's life from the controls' side: pause and carry on from either place, take
//! it off the list, remove its server, stop it across a restart, replace its set (T682, T683,
//! T685, T686, T687).
//!
//! The work is a task this test controls, filed under the video's batch exactly as the
//! measurement and the build are: the real engine, the real store and the real watcher of the
//! video commands, without a film or a server.

use std::time::Duration;

use tokio::sync::mpsc;
use vrcast_studio_lib::commands::servers::api as servers;
use vrcast_studio_lib::commands::video::{api as video, VideoView};
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::domain::video::{after_restart, AfterRestart, VideoStage, VideoState};
use vrcast_studio_lib::domain::wording::DetailCode;
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
    // The same store of secrets: it is the operating system's, and outlives the run.
    AppState::with_db(state.db.clone(), state.secrets.clone()).unwrap()
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
    tx.send((0.5, DetailCode::StageSendingVariant)).unwrap();
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

// ---------- T682: a stop on the server that outlived the application ----------

/// A video left `cancelling` at the cutting by a run that is over, with its build's row as a
/// killed run leaves it (`recover_after_start` makes it `paused`) and — when `cut` — the note
/// of the cutting it had started on the server.
fn left_cancelling(state: &AppState, id: &str, server: &str, cut: bool) -> String {
    use vrcast_studio_lib::store::remote_runs::{self, RemoteRun};
    use vrcast_studio_lib::tasks::store::{self as tasks, TaskRecord};

    let mut row = VideoRow::new(id, server, "C:/nowhere/film.mp4", id, id);
    row.state = VideoState::Cancelling;
    row.stage = VideoStage::Cutting;
    let task = format!("old-cutting-{id}");
    row.task_id = Some(task.clone());
    rows::save(&state.db, &row).unwrap();
    let mut t = TaskRecord::new(&task, TaskKind::BuildLadder, Some(server.to_owned()));
    t.state = TaskState::Paused;
    t.stage = Some(DetailCode::StageStopUnconfirmed);
    t.batch = Some(Batch {
        id: id.to_owned(),
        label: id.to_owned(),
    });
    t.resume_token = Some(id.to_owned());
    tasks::upsert(&state.db, &t).unwrap();
    if cut {
        remote_runs::save(
            &state.db,
            &task,
            &RemoteRun {
                var: String::from("VRCAST_HLS_JOB"),
                mark: format!("{id}:0123456789abcdef"),
                // Nothing listens there: the stop cannot be confirmed.
                host: String::from("127.0.0.1"),
                port: 1,
                user: String::from("root"),
            },
        )
        .unwrap();
    }
    task
}

/// A server nobody answers on, with a confirmed fingerprint so a stop is at least tried.
fn unreachable_server(state: &AppState) -> String {
    let mut input = valid_input("Nowhere");
    input.host = String::from("127.0.0.1");
    input.port = 1;
    let id = servers::server_add(state, input, "in-memory-only").unwrap();
    vrcast_studio_lib::store::profiles::set_fingerprint(&state.db, &id, "SHA256:test").ok();
    id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stop_the_server_never_confirmed_stays_cancelling_after_a_restart() {
    let state = state();
    let server = unreachable_server(&state);
    let task = left_cancelling(&state, "stop-unconfirmed", &server, true);

    let next = restarted(&state);
    video::restore_videos(&next).unwrap();
    // The server cannot be reached, so the stop is not confirmed: still stopping, and the
    // build's row still unfinished — its set stays busy for the library and «Replace».
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let v = video::video_get(&next, "stop-unconfirmed").unwrap();
    assert_eq!(v.state, VideoState::Cancelling);
    let t = next.tasks.get(&task).unwrap().unwrap();
    assert!(!t.state.is_final(), "the build was closed without a stop");
    // Not «Retry», not «Remove»-at-once, not a new build: it is still being stopped.
    assert!(video::video_retry(&next, "stop-unconfirmed", false).is_err());
    assert!(vrcast_studio_lib::store::remote_runs::get(&next.db, &task)
        .unwrap()
        .is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stop_with_nothing_on_the_server_is_cancelled_after_a_restart() {
    let state = state();
    let server = unreachable_server(&state);
    // Stopped while measuring or encoding — the work died with the application.
    let task = left_cancelling(&state, "stop-local", &server, false);
    let next = restarted(&state);
    video::restore_videos(&next).unwrap();
    assert_eq!(
        video::video_get(&next, "stop-local").unwrap().state,
        VideoState::Cancelled
    );
    assert_eq!(
        next.tasks.get(&task).unwrap().unwrap().state,
        TaskState::Cancelled
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn work_another_copy_is_running_is_left_alone_after_a_restart() {
    use vrcast_studio_lib::tasks::store as tasks;
    let state = state();
    let server = unreachable_server(&state);
    let task = left_cancelling(&state, "stop-foreign", &server, true);
    // Still `running` after start-up: its owner — another copy — is alive (stamped with
    // this very process, which is alive, as `save_state` does for a running task).
    tasks::save_state(&state.db, &task, TaskState::Running, None).unwrap();
    let next = restarted(&state);
    assert_eq!(
        tasks::get(&next.db, &task).unwrap().unwrap().state,
        TaskState::Running
    );
    video::restore_videos(&next).unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        video::video_get(&next, "stop-foreign").unwrap().state,
        VideoState::Cancelling
    );
    assert_eq!(
        tasks::get(&next.db, &task).unwrap().unwrap().state,
        TaskState::Running,
        "another copy's work was closed"
    );
}

// ---------- T686: a confirmed «Replace» kept across a restart ----------

/// A video stopped on a taken name, as `next_task` leaves it, with a plan, and — when
/// `phase` — a «Replace» confirmed and carried as far as `phase` by a run that was killed.
fn replacing(state: &AppState, id: &str, server: &str, phase: Option<&str>) {
    use vrcast_studio_lib::commands::error::{AppError, ErrorCode};
    let mut row = VideoRow::new(id, server, "C:/nowhere/film.mp4", "Film", id);
    row.stage = VideoStage::Planned;
    row.state = VideoState::Problem;
    row.problem_json = Some(
        serde_json::json!({
            "error": AppError::new(ErrorCode::SlugTaken),
            "actions": ["replace", "rename"],
        })
        .to_string(),
    );
    if let Some(phase) = phase {
        row.replacing_json = Some(
            serde_json::json!({
                "phase": phase,
                "into_medium": false,
                "confirmed": false,
                "media_id": "m_taken",
            })
            .to_string(),
        );
    }
    rows::save(&state.db, &row).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_replace_killed_after_the_old_set_went_carries_on_building_after_a_restart() {
    let state = state();
    let server = unreachable_server(&state);
    replacing(&state, "replace-built", &server, Some("building"));

    let next = restarted(&state);
    video::restore_videos(&next).unwrap();
    // Not left on its old problem: the replace goes on — the medium it was building into is
    // the video's now, and the build is started (it then stops on the server not answering,
    // past the plan, never back on «name taken»).
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        let v = video::video_get(&next, "replace-built").unwrap();
        let row = rows::get(&next.db, "replace-built").unwrap().unwrap();
        if row.replacing_json.is_none() && v.media_id.as_deref() == Some("m_taken") {
            assert!(v.start_requested);
            assert!(!v.problem.as_ref().is_some_and(
                |p| p.error.code == vrcast_studio_lib::commands::error::ErrorCode::SlugTaken
            ));
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the replace was not carried on: {:?} {:?}",
            v.state,
            row.replacing_json
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_replace_killed_while_removing_is_kept_and_the_library_says_building() {
    use vrcast_studio_lib::commands::library::{api as library, LibraryView, MediaView};
    use vrcast_studio_lib::domain::video::SetWorkState;
    use vrcast_studio_lib::store::library_cache;

    let state = state();
    let server = unreachable_server(&state);
    replacing(&state, "replace-removing", &server, Some("deleting"));
    let medium = MediaView {
        id: String::from("m_taken"),
        title: String::from("Film"),
        slug: String::from("replace-removing"),
        files: Vec::new(),
        ladders: Vec::new(),
        set_files: Vec::new(),
        total_bytes: 0,
        created_at: String::from("2026-10-02T00:00:00Z"),
        set_work: None,
    };
    library_cache::save(
        &state.db,
        &server,
        &LibraryView {
            server_id: server.clone(),
            media: vec![medium],
            unrecognized: Vec::new(),
            disk: None,
            stale: false,
        },
    )
    .unwrap();

    let next = restarted(&state);
    video::restore_videos(&next).unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    // The server does not answer: the confirmed replace is kept — asked again later, never
    // dropped — and the medium whose set it is removing reads as building, not missing.
    let row = rows::get(&next.db, "replace-removing").unwrap().unwrap();
    assert!(
        row.replacing_json.is_some(),
        "the confirmed replace was forgotten"
    );
    let view = library::library_list_known(&next, &server).await.unwrap();
    let work = view.media[0]
        .set_work
        .clone()
        .expect("the set reads as missing");
    assert_eq!(work.state, SetWorkState::Building);
    assert_eq!(work.video_id, "replace-removing");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_replace_the_server_refuses_leaves_the_video_on_its_problem_with_replace() {
    let state = state();
    // No fingerprint confirmed: the gate refuses at once, not «unreachable».
    let mut input = valid_input("Refusing");
    input.host = String::from("127.0.0.1");
    input.port = 1;
    let server = servers::server_add(&state, input, "in-memory-only").unwrap();
    replacing(&state, "replace-refused", &server, None);

    let err = video::video_replace(&state, "replace-refused", false)
        .await
        .expect_err("replaced without the server");
    assert_ne!(
        err.code,
        vrcast_studio_lib::commands::error::ErrorCode::VideoNotNow
    );
    let row = rows::get(&state.db, "replace-refused").unwrap().unwrap();
    assert!(row.replacing_json.is_none(), "a refused replace was kept");
    let v = video::video_get(&state, "replace-refused").unwrap();
    assert_eq!(v.state, VideoState::Problem);
    assert!(v
        .problem
        .unwrap()
        .actions
        .contains(&vrcast_studio_lib::domain::video::VideoAction::Replace));
}

// ---------- T687: a version that grows with the progress too ----------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_video_s_rev_grows_with_its_progress_and_a_later_view_never_has_a_lower_one() {
    let state = state();
    let row = going(&state, "rev");
    let (task, tx) = build(&state, &row).await;
    let at_10 = video::video_get(&state, &row.id).unwrap();
    // Same stage as the 10% before it, so the row is not rewritten for the stage either; the bar
    // of an encode is scaled (`bar_of`, the encode and its check share one bar).
    tx.send((0.7, DetailCode::StageConverting)).unwrap();
    let at_70_bar = vrcast_studio_lib::domain::video::bar_of(DetailCode::StageConverting, 0.7);
    until(&state, &row.id, "70%", |v| {
        v.progress.as_ref().is_some_and(|p| p.progress == at_70_bar)
    })
    .await;
    let at_70 = video::video_get(&state, &row.id).unwrap();
    // The row was not written for a percentage — and the version still moved on.
    assert_eq!(at_10.updated_at, at_70.updated_at);
    assert!(at_70.rev > at_10.rev, "{} then {}", at_10.rev, at_70.rev);
    // The list is a view like any other: taken later, never lower.
    let listed = video::video_list(&state).unwrap();
    assert!(listed[0].rev >= at_70.rev);
    // And it keeps growing across a restart: the next run starts above this one.
    let next = AppState::with_db(state.db.clone(), state.secrets.clone()).unwrap();
    assert!(video::video_get(&next, &row.id).unwrap().rev > listed[0].rev);
    state.tasks.cancel(&task).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_change_goes_out_with_a_higher_rev_than_the_one_before() {
    use vrcast_studio_lib::commands::AppEvent;
    let state = state();
    let row = going(&state, "rev-events");
    let mut events = state.subscribe();
    let (task, tx) = build(&state, &row).await;
    for p in [0.2, 0.4, 0.6] {
        tx.send((p, DetailCode::StageSendingVariant)).unwrap();
        until(&state, &row.id, "progress", |v| {
            v.progress.as_ref().is_some_and(|x| x.progress == p)
        })
        .await;
    }
    let revs: Vec<u64> = std::iter::from_fn(|| events.try_recv().ok())
        .filter_map(|e| match e {
            AppEvent::VideoUpdate(v) if v.id == row.id => Some(v.rev),
            _ => None,
        })
        .collect();
    assert!(revs.len() >= 3, "{revs:?}");
    assert!(revs.windows(2).all(|w| w[1] > w[0]), "{revs:?}");
    state.tasks.cancel(&task).unwrap();
}
