//! T063 — responsiveness under load (SC-009, FR-080).
//!
//! The requirement: a reaction to an action within 100 ms, even while background tasks are
//! running. What is checked here is its core half — that the reading commands answer quickly
//! while the engine is busy. The other half (the drawing) belongs to the interface and is
//! measured by eye; but the core is exactly where responsiveness is lost: it is enough for a
//! reading command to wait on a lock a running task holds, and the window freezes for as
//! long as that task runs.
//!
//! The threshold is taken with room to spare over the stated one: on a machine busy with a
//! build a single call sometimes lags, and a test failing over that would soon be re-run
//! without a glance. The difference between "tens of milliseconds" and "hundreds" matters
//! more here than the exact figure.

use std::sync::Arc;
use std::time::{Duration, Instant};
use vrcast_studio_lib::commands::error::DetailCode;
use vrcast_studio_lib::commands::servers::api as servers;
use vrcast_studio_lib::commands::{api, AppState};
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;
use vrcast_studio_lib::tasks::state::TaskKind;

/// The limit for one call. 100 ms is stated; twice that is taken so the test catches a
/// broken design rather than the jitter of a loaded machine.
const LIMIT: Duration = Duration::from_millis(200);

fn state() -> AppState {
    AppState::with_db(
        Arc::new(Db::open_in_memory().unwrap()),
        Arc::new(InMemorySecretStore::new()),
    )
    .expect("the application state would not assemble")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reading_commands_answer_quickly_while_tasks_run() {
    let s = state();

    // The queue is filled with work: some running, some waiting for its lane.
    let mut ids = Vec::new();
    for kind in [
        TaskKind::Convert,
        TaskKind::Convert,
        TaskKind::Upload,
        TaskKind::Upload,
        TaskKind::Probe,
        TaskKind::Probe,
    ] {
        let id = s
            .tasks
            .submit(kind, None, |ctx| async move {
                // Work that reports something all the time: progress events are the
                // densest stream the application ever has.
                for i in 0..2_000 {
                    ctx.report(i as f64 / 2_000.0, DetailCode::StageConverting);
                    if ctx.is_cancelled() {
                        return Ok(());
                    }
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
                Ok(())
            })
            .await
            .expect("the task would not submit");
        ids.push(id);
    }

    // The tasks are given time to really start: measuring responsiveness on an idle
    // engine would measure nothing.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut worst = Duration::ZERO;
    let mut worst_name = "";

    for _ in 0..20 {
        for (name, elapsed) in [
            ("tasks_list", measure(|| api::tasks_list(&s).map(|_| ()))),
            (
                "tasks_on_close",
                measure(|| api::tasks_on_close(&s).map(|_| ())),
            ),
            (
                "servers_list",
                measure(|| servers::servers_list(&s).map(|_| ())),
            ),
        ] {
            if elapsed > worst {
                worst = elapsed;
                worst_name = name;
            }
        }

        // Measured apart from the rest because it is async now: asked about no server it
        // touches nothing, and that is the case this promise is about — the About screen must
        // answer at once whether or not a machine somewhere is awake.
        {
            let began = std::time::Instant::now();
            let _ = api::app_versions(&s, None).await;
            let elapsed = began.elapsed();
            if elapsed > worst {
                worst = elapsed;
                worst_name = "app_versions";
            }
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    for id in &ids {
        let _ = s.tasks.cancel(id);
    }

    assert!(
        worst < LIMIT,
        "the command {worst_name} answered in {worst:?} while tasks were running — \
         longer than the limit of {LIMIT:?}. The interface freezes in that time"
    );
    println!("worst call under load: {worst_name} in {worst:?}");
}

fn measure<F>(f: F) -> Duration
where
    F: FnOnce() -> vrcast_studio_lib::commands::error::Result<()>,
{
    let started = Instant::now();
    f().expect("a reading command must not fail");
    started.elapsed()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn submitting_a_task_returns_at_once_rather_than_awaiting_its_end() {
    // FR-080, the command layer's contract, rule 1: anything longer than a second is a
    // task. The command must return the identifier immediately, or the window freezes for
    // as long as the work runs, and no progress events will put that right.
    let s = state();

    let started = Instant::now();
    let id = s
        .tasks
        .submit(TaskKind::Upload, None, |ctx| async move {
            // Certainly longer than any reasonable wait for an answer.
            for _ in 0..600 {
                if ctx.is_cancelled() {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Ok(())
        })
        .await
        .expect("the task would not submit");
    let elapsed = started.elapsed();

    assert!(
        elapsed < LIMIT,
        "submitting the task took {elapsed:?} — the command waited for the work instead \
         of returning the identifier"
    );

    let _ = s.tasks.cancel(&id);
}
