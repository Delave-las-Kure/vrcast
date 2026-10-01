//! T652 — a report of progress does not change a task's state (QA-24A №3, FR-034, FR-082).
//!
//! The state an event carries comes from the engine, not from the fact that bytes moved: a
//! window already being written when "pause" was pressed finishes and reports afterwards, and
//! that report used to say `Running` about a task the database already held as `Paused` —
//! which took "Carry on" away from the person looking at it.

use std::sync::Arc;
use std::time::Duration;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::tasks::engine::{TaskEngine, TaskEvent};
use vrcast_studio_lib::tasks::state::{TaskKind, TaskState};

async fn until(e: &TaskEngine, id: &str, want: TaskState) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while e.get(id).unwrap().unwrap().state != want {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{id} never became {want:?}"));
}

/// Everything sent about `id` so far, as (state, progress).
fn drain(rx: &mut tokio::sync::broadcast::Receiver<TaskEvent>, id: &str) -> Vec<(TaskState, f64)> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let TaskEvent::Progress {
            id: which,
            state,
            progress,
            ..
        } = ev
        {
            if which == id {
                out.push((state, progress));
            }
        }
    }
    out
}

/// The QA round's probe, turned round: the last window finishes after the pause and its
/// report says `Paused`, matching the database.
#[tokio::test]
async fn a_report_after_the_pause_says_paused() {
    let e = TaskEngine::new(Arc::new(Db::open_in_memory().unwrap()));
    let (window_done, window) = tokio::sync::oneshot::channel::<()>();
    let (reported_tx, reported) = tokio::sync::oneshot::channel::<()>();
    let id = e
        .submit(TaskKind::Upload, None, move |ctx| async move {
            // The window that was in flight when the pause was pressed.
            let _ = window.await;
            ctx.report_transfer(0.5, Some(1000), Some(60));
            let _ = reported_tx.send(());
            ctx.wait_while_paused().await;
            Ok(())
        })
        .await
        .unwrap();
    until(&e, &id, TaskState::Running).await;

    let mut rx = e.subscribe();
    e.pause(&id).unwrap();
    window_done.send(()).unwrap();
    reported.await.unwrap();

    assert_eq!(e.get(&id).unwrap().unwrap().state, TaskState::Paused);
    let seen = drain(&mut rx, &id);
    assert_eq!(
        seen.first().map(|s| s.0),
        Some(TaskState::Paused),
        "the pause was not announced by the engine itself: {seen:?}"
    );
    assert!(
        seen.iter().any(|(_, p)| (*p - 0.5).abs() < 1e-9),
        "the late window's bytes did not reach anybody: {seen:?}"
    );
    assert!(
        seen.iter().all(|(s, _)| *s == TaskState::Paused),
        "a report of bytes said the paused task was running: {seen:?}"
    );

    e.cancel(&id).unwrap();
    until(&e, &id, TaskState::Cancelled).await;
}

/// Every change of state the engine makes short of the end is announced by the engine, in
/// order, with the bar where it stands: paused, waiting for a place, running again.
#[tokio::test]
async fn pausing_and_carrying_on_are_announced_in_order_by_the_engine() {
    let e = TaskEngine::new(Arc::new(Db::open_in_memory().unwrap()));
    let (go, gate) = tokio::sync::oneshot::channel::<()>();
    let id = e
        .submit(TaskKind::Upload, None, move |ctx| async move {
            ctx.report_important(
                0.3,
                vrcast_studio_lib::commands::error::DetailCode::StageChecksum,
            );
            let _ = gate.await;
            ctx.wait_while_paused().await;
            Ok(())
        })
        .await
        .unwrap();
    until(&e, &id, TaskState::Running).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut rx = e.subscribe();
    e.pause(&id).unwrap();
    e.resume(&id).unwrap();
    let seen: Vec<TaskState> = drain(&mut rx, &id).into_iter().map(|s| s.0).collect();
    assert_eq!(
        seen,
        vec![TaskState::Paused, TaskState::Queued, TaskState::Running],
        "the engine's own changes of state did not reach the interface in order"
    );

    go.send(()).unwrap();
    until(&e, &id, TaskState::Completed).await;
}

/// A report that comes after the task has ended is not sent: it would arrive after `Done`
/// and put a finished row back into motion.
#[tokio::test]
async fn nothing_is_reported_about_a_task_that_has_ended() {
    let e = TaskEngine::new(Arc::new(Db::open_in_memory().unwrap()));
    let (keep_tx, keep_rx) = tokio::sync::oneshot::channel();
    let id = e
        .submit(TaskKind::Upload, None, move |ctx| async move {
            let _ = keep_tx.send(ctx);
            Ok(())
        })
        .await
        .unwrap();
    until(&e, &id, TaskState::Completed).await;
    let ctx = keep_rx.await.unwrap();

    let mut rx = e.subscribe();
    ctx.report_important(
        0.9,
        vrcast_studio_lib::commands::error::DetailCode::StageChecksum,
    );
    assert!(drain(&mut rx, &id).is_empty(), "a report outlived its task");
}
