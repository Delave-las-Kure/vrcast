//! T619 — a command of a deployment whose end was not heard (its answer lost, or given up on at
//! `EXEC_CEILING` while the connection lived) is not a step saying no: the run confirms the
//! stop of what carries its mark before it sends anything else, and the task is not written
//! `Completed` until that stop is confirmed (QA-19 №4).
//!
//! By the pattern of `deploy_stop.rs`: what is checked is the rule, with a stand-in for the
//! server — `server::deploy::settle_unheard` (what `Context::send` does before every command)
//! and `tasks::deploy::settle` on the real task engine. The real stop against a container is
//! `deploy_cancel.rs`'s and `deploy_network_cut.rs`'s job.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use vrcast_studio_lib::commands::api as core;
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::domain::marked::Stopped;
use vrcast_studio_lib::domain::server_profile::{AuthKind, ServerProfile};
use vrcast_studio_lib::domain::wording::DetailCode;
use vrcast_studio_lib::server::deploy::{settle_unheard, DeployError, RunMark};
use vrcast_studio_lib::server::marked::{Patience, StopProblem};
use vrcast_studio_lib::ssh::exec::EXEC_CEILING;
use vrcast_studio_lib::ssh::SshError;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;
use vrcast_studio_lib::tasks::state::{TaskKind, TaskState};

const SERVER: &str = "t619-server";

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
            name: String::from("T619"),
            host: String::from("192.0.2.1"),
            port: 22,
            user: String::from("root"),
            auth_kind: AuthKind::Password,
            secret_ref: String::from("t619"),
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

/// A command sent a minute ago whose end was not heard: nine minutes of its `EXEC_CEILING`
/// left, so a stop is to wait for it, not signal it.
fn unheard_a_minute_ago() -> RunMark {
    RunMark::fresh().with_unheard_command(Instant::now() - Duration::from_secs(60))
}

#[tokio::test]
async fn nothing_unheard_asks_no_stop() {
    let run = RunMark::fresh();
    let asked = AtomicUsize::new(0);
    let r = settle_unheard(&run, |_| {
        asked.fetch_add(1, Ordering::SeqCst);
        async { Err(StopProblem::Unreadable(String::from("must not be asked"))) }
    })
    .await;
    assert!(r.is_ok(), "{r:?}");
    assert_eq!(asked.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn an_unheard_command_is_waited_for_and_confirmed_before_the_next_goes() {
    let run = unheard_a_minute_ago();
    let answers = Mutex::new(vec![
        Err(StopProblem::StillRunning(String::from("apt-get 4242"))),
        Err(StopProblem::StillRunning(String::from("apt-get 4242"))),
        Ok(Stopped::EndedOnItsOwn),
    ]);
    let patiences: Mutex<Vec<Patience>> = Mutex::default();
    let r = settle_unheard(&run, |p| {
        patiences.lock().unwrap().push(p);
        let answer = answers.lock().unwrap().remove(0);
        async move { answer }
    })
    .await;
    assert!(r.is_ok(), "{r:?}");
    let patiences = patiences.lock().unwrap();
    assert_eq!(
        patiences.len(),
        3,
        "\"still running\" was taken as an answer rather than asked again"
    );
    assert!(
        patiences.iter().all(|p| *p != Patience::NONE),
        "a command still within its EXEC_CEILING was signalled rather than waited for: \
         {patiences:?}"
    );
    assert_eq!(
        run.may_run_until(),
        None,
        "a confirmed stop left the command on record as possibly running"
    );
}

#[tokio::test]
async fn an_unconfirmed_stop_sends_nothing_further_and_keeps_the_record() {
    // The server could not be asked; the command may still be running. The run must not go on
    // to the next step, and the one record of when that command was sent must survive for the
    // task's own stop (`tasks::deploy::settle`), which is patient with it by that record.
    let sent = Instant::now() - Duration::from_secs(60);
    let run = RunMark::fresh().with_unheard_command(sent);
    let r = settle_unheard(&run, |_| async {
        Err(StopProblem::Ssh(SshError::Exec(String::from(
            "the channel would not open",
        ))))
    })
    .await;
    assert!(matches!(r, Err(DeployError::Ssh(_))), "{r:?}");
    assert_eq!(
        run.may_run_until(),
        Some(sent + EXEC_CEILING),
        "the record of the unheard command was lost or moved"
    );

    // Something survived KILL — not a confirmation either.
    let r = settle_unheard(&run, |_| async {
        Err(StopProblem::StillAlive(String::from("dpkg 77")))
    })
    .await;
    assert!(r.is_err(), "{r:?}");
    assert_eq!(run.may_run_until(), Some(sent + EXEC_CEILING));

    // A stop asked meanwhile: `Cancelled`, and the task's stop takes over with the same record.
    run.ask_to_stop();
    let asked = AtomicBool::new(false);
    let r = settle_unheard(&run, |_| {
        asked.store(true, Ordering::SeqCst);
        async { Ok(Stopped::AlreadyGone) }
    })
    .await;
    assert!(matches!(r, Err(DeployError::Cancelled)), "{r:?}");
    assert!(!asked.load(Ordering::SeqCst));
    assert_eq!(run.may_run_until(), Some(sent + EXEC_CEILING));
}

#[tokio::test]
async fn a_command_past_its_ceiling_is_signalled_at_once() {
    let run = RunMark::fresh().with_unheard_command(Instant::now() - EXEC_CEILING);
    let patience: Mutex<Option<Patience>> = Mutex::default();
    let r = settle_unheard(&run, |p| {
        *patience.lock().unwrap() = Some(p);
        async { Ok(Stopped::AfterTerm) }
    })
    .await;
    assert!(r.is_ok(), "{r:?}");
    assert_eq!(*patience.lock().unwrap(), Some(Patience::NONE));
}

/// The case from QA-19 №4: a non-blocking step (`UnattendedUpgrades`) timed out on the last
/// command the run sent, the run itself came back `Ok` — and the task used to be written
/// `Completed` over an apt still running. Now it stays `Running`, holding the server, until
/// the stop is confirmed, and only then `Completed`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_run_that_ended_well_with_a_command_unheard_is_not_completed_until_its_stop_is_confirmed()
{
    let state = app_state();
    let server_back = Arc::new(AtomicBool::new(false));
    let attempts = Arc::new(AtomicUsize::new(0));
    let first_asked = Arc::new(AtomicBool::new(false));
    let patiences: Arc<Mutex<Vec<Patience>>> = Arc::default();

    let id = {
        let server_back = server_back.clone();
        let attempts = attempts.clone();
        let first_asked = first_asked.clone();
        let patiences = patiences.clone();
        state
            .tasks
            .submit(TaskKind::Deploy, Some(SERVER.to_owned()), move |task| async move {
                let run = unheard_a_minute_ago();
                let stop_again = move |_mark: String, patience: Patience| -> BoxFuture<'static, Result<Stopped, String>> {
                    let server_back = server_back.clone();
                    let attempts = attempts.clone();
                    patiences.lock().unwrap().push(patience);
                    Box::pin(async move {
                        attempts.fetch_add(1, Ordering::SeqCst);
                        if server_back.load(Ordering::SeqCst) {
                            Ok(Stopped::EndedOnItsOwn)
                        } else {
                            Err(String::from("could not reach the server"))
                        }
                    })
                };
                vrcast_studio_lib::tasks::deploy::settle(
                    &task,
                    1.0,
                    run.as_str(),
                    Ok(()),
                    false,
                    async {
                        first_asked.store(true, Ordering::SeqCst);
                        Err(String::from("the run's own connection is gone"))
                    },
                    &|| run.may_run_until(),
                    &stop_again,
                )
                .await
                .map_err(|e| vrcast_studio_lib::tasks::deploy::failed(e, &[]))
            })
            .await
            .expect("the task was not submitted")
    };

    tokio::time::sleep(Duration::from_millis(3_000)).await;
    assert!(
        first_asked.load(Ordering::SeqCst),
        "a run that came back Ok with a command unheard was not stopped at all"
    );
    let task = core::task_get(&state, &id).unwrap();
    assert_eq!(
        task.state,
        TaskState::Running,
        "the deployment was written Completed while a command of it may still be running"
    );
    assert_eq!(task.stage, Some(DetailCode::StageStopUnconfirmed));
    assert!(attempts.load(Ordering::SeqCst) >= 1);
    assert!(
        patiences
            .lock()
            .unwrap()
            .iter()
            .all(|p| *p != Patience::NONE),
        "the unheard command was signalled inside its EXEC_CEILING"
    );

    server_back.store(true, Ordering::SeqCst);
    let deadline = Instant::now() + Duration::from_secs(15);
    let ended = loop {
        let t = core::task_get(&state, &id).unwrap();
        if t.state.is_final() {
            break t.state;
        }
        assert!(Instant::now() < deadline, "never ended ({:?})", t.state);
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(
        ended,
        TaskState::Completed,
        "a run whose steps all went through is a completed one, once its stop is confirmed"
    );
}

/// And a run that came back `Ok` with every command heard is still handed back at once, with
/// no stop sent — the ordinary deployment costs nothing more.
#[tokio::test]
async fn a_run_that_ended_well_with_everything_heard_is_not_stopped() {
    let task = vrcast_studio_lib::tasks::engine::TaskContext::detached(Arc::new(
        Db::open_in_memory().unwrap(),
    ));
    let run = RunMark::fresh();
    let first_asked = AtomicBool::new(false);
    let r = vrcast_studio_lib::tasks::deploy::settle(
        &task,
        1.0,
        run.as_str(),
        Ok(3),
        false,
        async {
            first_asked.store(true, Ordering::SeqCst);
            Err(String::from("must not be asked"))
        },
        &|| run.may_run_until(),
        &|_: String, _: Patience| -> BoxFuture<'static, Result<Stopped, String>> {
            Box::pin(async { Err(String::from("must not be asked")) })
        },
    )
    .await;
    assert_eq!(r.ok(), Some(3));
    assert!(!first_asked.load(Ordering::SeqCst));
}
