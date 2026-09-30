//! T643 (QA-22 №3) — "forget everything" is not done while a task is at work, and no task starts
//! while it runs (the owner's decision of 2026-09-30).
//!
//! The finding: a deployment's `key_keeper`, already past its look, wrote the made key into the
//! store after the removal had reported "removed 1, left 0". A lock around the removal alone
//! would not help — the task writes once it is let go. So the removal is refused while any task
//! is alive (`FORGET_TASKS_RUNNING`), and while it runs the engine is closed: a claim, a new task
//! and a second removal are refused (`FORGET_IN_PROGRESS`). The look and the closing are one step
//! (`TaskEngine::close_for_forgetting`).

use std::sync::Arc;
use std::time::Duration;

use vrcast_studio_lib::commands::forget::api;
use vrcast_studio_lib::commands::servers::api as servers;
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::error::{AppError, ErrorCode};
use vrcast_studio_lib::store::secrets::{InMemorySecretStore, SecretRef, SecretStore};
use vrcast_studio_lib::tasks::engine::ClaimRefused;
use vrcast_studio_lib::tasks::state::{LaneLimits, TaskKind, TaskState};

use super::support::{state, valid_input};

const MADE_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nmade-by-the-run\n";

fn secret_of(state: &AppState, id: &str) -> Option<String> {
    let profile = vrcast_studio_lib::store::profiles::get(&state.db, id)
        .unwrap()
        .expect("the profile is gone");
    state
        .secrets
        .get(&SecretRef::from_stored(profile.secret_ref))
        .ok()
}

async fn wait_for(state: &AppState, id: &str, want: TaskState) {
    for _ in 0..250 {
        if state.tasks.get(id).unwrap().map(|t| t.state) == Some(want) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the task never became {want:?}");
}

/// A task that runs until it is let go.
async fn hold(state: &AppState, kind: TaskKind) -> (String, Arc<tokio::sync::Notify>) {
    let go = Arc::new(tokio::sync::Notify::new());
    let wait = go.clone();
    let id = state
        .tasks
        .submit(kind, None, move |_ctx| async move {
            wait.notified().await;
            Ok(())
        })
        .await
        .expect("the task would not start");
    (id, go)
}

#[tokio::test]
async fn a_running_task_refuses_and_nothing_is_removed() {
    let s = state();
    let id = servers::server_add(&s, valid_input("идёт задача"), "секрет")
        .expect("the profile would not be created");

    for kind in [
        TaskKind::Deploy,
        TaskKind::UpgradeServer,
        TaskKind::Upload,
        TaskKind::Convert,
        TaskKind::BuildLadder,
        TaskKind::MeasureQuality,
    ] {
        let (task, go) = hold(&s, kind).await;
        wait_for(&s, &task, TaskState::Running).await;

        let err = api::forget_everything(&s, true).expect_err("removed under a running task");
        assert_eq!(err.code, ErrorCode::ForgetTasksRunning, "{kind:?}: {err:?}");
        assert!(
            err.cause
                .as_deref()
                .is_some_and(|c| c.contains(kind.as_str())),
            "{kind:?}: the cause does not name the task: {err:?}"
        );
        assert_eq!(
            secret_of(&s, &id).as_deref(),
            Some("секрет"),
            "{kind:?}: the secret went although the removal was refused"
        );
        assert_eq!(
            vrcast_studio_lib::store::profiles::list(&s.db)
                .unwrap()
                .len(),
            1,
            "{kind:?}: the profile went although the removal was refused"
        );
        assert!(
            !s.tasks.is_closed_for_forgetting(),
            "a refused removal left the engine closed"
        );

        go.notify_one();
        wait_for(&s, &task, TaskState::Completed).await;
    }

    // Everything over: the removal goes through.
    let went = api::forget_everything(&s, true).expect("removal after the tasks ended");
    assert_eq!(went.secrets_removed, 1);
}

#[tokio::test]
async fn a_queued_task_and_a_pending_claim_refuse_too() {
    let s = state();
    servers::server_add(&s, valid_input("очередь"), "секрет").unwrap();

    // A deployment still connecting holds its claim and has no task yet (T621).
    let claim = s.tasks.claim("deploy:x").expect("the claim was not given");
    let err = api::forget_everything(&s, true).expect_err("removed under a pending claim");
    assert_eq!(err.code, ErrorCode::ForgetTasksRunning);
    drop(claim);

    // Queued: the lane holds one, the second waits.
    s.tasks.set_limits(LaneLimits {
        compute: 1,
        network: 1,
        light: 1,
    });
    let (first, go_first) = hold(&s, TaskKind::Convert).await;
    wait_for(&s, &first, TaskState::Running).await;
    let (second, go_second) = hold(&s, TaskKind::Convert).await;
    assert_eq!(
        s.tasks.get(&second).unwrap().map(|t| t.state),
        Some(TaskState::Queued)
    );
    let err = api::forget_everything(&s, true).expect_err("removed under a queued task");
    assert_eq!(err.code, ErrorCode::ForgetTasksRunning);

    go_first.notify_one();
    wait_for(&s, &first, TaskState::Completed).await;
    wait_for(&s, &second, TaskState::Running).await;
    go_second.notify_one();
    wait_for(&s, &second, TaskState::Completed).await;

    let went = api::forget_everything(&s, true).expect("removal after the tasks ended");
    assert_eq!(went.secrets_removed, 1);
}

#[tokio::test]
async fn the_qa_path_a_deployment_keeping_its_key_cannot_write_it_back() {
    // QA-22 №3: the run's `key_keeper` wrote the key into the store after the removal had
    // reported it gone. Now the removal is refused while the run is alive; once the run is over
    // and the removal done, nothing is left that could write it back.
    let s = state();
    let id = servers::server_add(&s, valid_input("развёртывание"), "пароль").unwrap();
    let profile = vrcast_studio_lib::store::profiles::get(&s.db, &id)
        .unwrap()
        .unwrap();

    let go = Arc::new(tokio::sync::Notify::new());
    let wait = go.clone();
    let keep = vrcast_studio_lib::commands::deploy::key_keeper(&s, &profile, MADE_KEY.into());
    let task = s
        .tasks
        .submit(TaskKind::Deploy, Some(id.clone()), move |_ctx| async move {
            wait.notified().await;
            keep().map_err(|e| AppError::new(ErrorCode::Internal).with_cause(e))
        })
        .await
        .unwrap();
    wait_for(&s, &task, TaskState::Running).await;

    let err = api::forget_everything(&s, true).expect_err("removed under a deployment");
    assert_eq!(err.code, ErrorCode::ForgetTasksRunning);
    assert_eq!(secret_of(&s, &id).as_deref(), Some("пароль"));

    go.notify_one();
    wait_for(&s, &task, TaskState::Completed).await;
    assert_eq!(secret_of(&s, &id).as_deref(), Some(MADE_KEY));

    let went = api::forget_everything(&s, true).expect("removal after the run");
    assert_eq!(went.secrets_removed, 1);
    assert!(went.secrets_left.is_empty());
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        s.secrets
            .get(&SecretRef::from_stored(profile.secret_ref.clone()))
            .is_err(),
        "the removed secret came back"
    );
}

/// A store whose first `delete` stops and waits: the removal is held in its middle.
struct SlowDelete {
    inner: InMemorySecretStore,
    entered: std::sync::Mutex<Option<std::sync::mpsc::Sender<()>>>,
    release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}

impl SecretStore for SlowDelete {
    fn set(&self, r: &SecretRef, v: &str) -> vrcast_studio_lib::store::secrets::Result<()> {
        self.inner.set(r, v)
    }
    fn get(&self, r: &SecretRef) -> vrcast_studio_lib::store::secrets::Result<String> {
        self.inner.get(r)
    }
    fn delete(&self, r: &SecretRef) -> vrcast_studio_lib::store::secrets::Result<()> {
        let entered = self.entered.lock().unwrap().take();
        if let Some(tx) = entered {
            let _ = tx.send(());
            let _ = self.release.lock().unwrap().recv();
        }
        self.inner.delete(r)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn while_the_removal_runs_nothing_starts_and_afterwards_it_does() {
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let store = Arc::new(SlowDelete {
        inner: InMemorySecretStore::new(),
        entered: std::sync::Mutex::new(Some(entered_tx)),
        release: std::sync::Mutex::new(release_rx),
    });
    let s = AppState::with_db(
        Arc::new(vrcast_studio_lib::store::db::Db::open_in_memory().unwrap()),
        store,
    )
    .unwrap();
    servers::server_add(&s, valid_input("посреди удаления"), "секрет").unwrap();

    let removing = {
        let s = s.clone();
        std::thread::spawn(move || api::forget_everything(&s, true))
    };
    entered_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the removal never reached the store");

    // In its middle: a claim, a new task and a second removal are all refused.
    assert!(s.tasks.is_closed_for_forgetting());
    assert_eq!(
        s.tasks.try_claim("deploy:x").err(),
        Some(ClaimRefused::Forgetting)
    );
    assert!(s.tasks.claim("deploy:x").is_none());
    let err = s
        .tasks
        .submit(TaskKind::Upload, None, |_ctx| async { Ok(()) })
        .await
        .map_err(AppError::from)
        .expect_err("a task started during the removal");
    assert_eq!(err.code, ErrorCode::ForgetInProgress);
    let err = api::forget_everything(&s, true).expect_err("a second removal started");
    assert_eq!(err.code, ErrorCode::ForgetInProgress);
    assert!(
        s.tasks.list().unwrap().is_empty(),
        "a refused task was written down"
    );

    release_tx.send(()).unwrap();
    let went = removing.join().unwrap().expect("the removal failed");
    assert_eq!(went.secrets_removed, 1);

    // Over: the engine is open again, and a task starts and runs.
    assert!(!s.tasks.is_closed_for_forgetting());
    assert!(s.tasks.claim("deploy:x").is_some());
    let id = s
        .tasks
        .submit(TaskKind::Upload, None, |_ctx| async { Ok(()) })
        .await
        .expect("a task would not start after the removal");
    wait_for(&s, &id, TaskState::Completed).await;
}

#[test]
fn a_refused_removal_and_a_finished_one_both_leave_the_engine_open() {
    let s = state();
    let claim = s.tasks.claim("deploy:y").unwrap();
    assert_eq!(
        api::forget_everything(&s, true).unwrap_err().code,
        ErrorCode::ForgetTasksRunning
    );
    assert!(!s.tasks.is_closed_for_forgetting());
    drop(claim);

    api::forget_everything(&s, true).expect("removal failed");
    assert!(!s.tasks.is_closed_for_forgetting());
    assert!(s.tasks.claim("deploy:y").is_some());
}
