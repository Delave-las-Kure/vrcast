//! T650 — carrying on goes through the lane, like starting does (QA-24A №1, FR-031, FR-081,
//! FR-083).
//!
//! Both of the QA round's scenarios, on the real `TaskEngine`: uploads raised after a restart
//! and carried on while the lane is busy used to count as running while they waited, see each
//! other, and wait for ever; and a task paused mid-work and carried on used to take no place at
//! all, so two transfers ran under a limit of one.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::tasks::engine::TaskEngine;
use vrcast_studio_lib::tasks::state::{Lane, LaneLimits, TaskKind, TaskState};
use vrcast_studio_lib::tasks::store::{self, TaskRecord};

fn one_per_lane(db: Arc<Db>) -> TaskEngine {
    TaskEngine::new(db).with_limits(LaneLimits {
        compute: 1,
        network: 1,
        light: 1,
    })
}

fn state(e: &TaskEngine, id: &str) -> TaskState {
    e.get(id).unwrap().unwrap().state
}

async fn until(e: &TaskEngine, id: &str, want: TaskState) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while state(e, id) != want {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{id} never became {want:?}; it is {:?}", state(e, id)));
}

/// A body that counts how many tasks are inside it at once and holds the lane briefly.
fn counted(
    inside: Arc<AtomicUsize>,
    most: Arc<AtomicUsize>,
    entered: Arc<AtomicUsize>,
) -> impl FnOnce(
    vrcast_studio_lib::tasks::engine::TaskContext,
) -> std::pin::Pin<
    Box<
        dyn std::future::Future<Output = Result<(), vrcast_studio_lib::commands::error::AppError>>
            + Send,
    >,
> {
    move |_ctx| {
        Box::pin(async move {
            entered.fetch_add(1, Ordering::SeqCst);
            let now = inside.fetch_add(1, Ordering::SeqCst) + 1;
            most.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(80)).await;
            inside.fetch_sub(1, Ordering::SeqCst);
            Ok(())
        })
    }
}

/// QA-24A №1, the first half: two uploads raised after a restart, carried on one after the
/// other while a transfer holds the lane. They wait as `Queued`, take no place while waiting,
/// and then run one at a time once the lane is free.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn raised_uploads_carried_on_while_the_lane_is_busy_wait_queued_then_run_in_turn() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    let e = one_per_lane(db.clone());

    let (finish_current, current_done) = tokio::sync::oneshot::channel::<()>();
    let current = e
        .submit(TaskKind::Upload, None, move |_| async move {
            let _ = current_done.await;
            Ok(())
        })
        .await
        .unwrap();
    until(&e, &current, TaskState::Running).await;

    let inside = Arc::new(AtomicUsize::new(0));
    let most = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(AtomicUsize::new(0));
    for id in ["upload-a", "upload-b"] {
        store::upsert(&db, &TaskRecord::new(id, TaskKind::Upload, None)).unwrap();
        e.resubmit_paused(id, counted(inside.clone(), most.clone(), entered.clone()))
            .unwrap();
        assert_eq!(state(&e, id), TaskState::Paused);
    }

    e.resume("upload-a").unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    e.resume("upload-b").unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Waiting for a place is not running.
    assert_eq!(state(&e, "upload-a"), TaskState::Queued);
    assert_eq!(state(&e, "upload-b"), TaskState::Queued);
    assert_eq!(
        e.running_in_lane(Lane::Network),
        1,
        "only the transfer in hand"
    );
    assert_eq!(
        entered.load(Ordering::SeqCst),
        0,
        "a waiting upload began its work"
    );
    assert!(
        !e.get("upload-a").unwrap().unwrap().can_resume,
        "a task already waiting for its turn is offered \"carry on\" again"
    );

    finish_current.send(()).unwrap();
    until(&e, &current, TaskState::Completed).await;
    until(&e, "upload-a", TaskState::Completed).await;
    until(&e, "upload-b", TaskState::Completed).await;

    assert_eq!(
        entered.load(Ordering::SeqCst),
        2,
        "both carried-on uploads ran"
    );
    assert_eq!(
        most.load(Ordering::SeqCst),
        1,
        "two carried-on uploads ran at once under a limit of one"
    );
}

/// QA-24A №1, the second half: A paused mid-work, B started, A carried on. A waits `Queued`
/// and does not move until B has finished.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_paused_upload_carried_on_beside_a_running_one_waits_for_the_lane() {
    let e = one_per_lane(Arc::new(Db::open_in_memory().unwrap()));

    let ticks_a = Arc::new(AtomicUsize::new(0));
    let ta = ticks_a.clone();
    let a = e
        .submit(TaskKind::Upload, None, move |ctx| async move {
            for _ in 0..40 {
                ctx.wait_while_paused().await;
                if ctx.is_cancelled() {
                    break;
                }
                ta.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Ok(())
        })
        .await
        .unwrap();
    until(&e, &a, TaskState::Running).await;
    e.pause(&a).unwrap();
    tokio::time::sleep(Duration::from_millis(60)).await;

    let (finish_b, b_done) = tokio::sync::oneshot::channel::<()>();
    let b = e
        .submit(TaskKind::Upload, None, move |_| async move {
            let _ = b_done.await;
            Ok(())
        })
        .await
        .unwrap();
    until(&e, &b, TaskState::Running).await;

    let frozen = ticks_a.load(Ordering::SeqCst);
    e.resume(&a).unwrap();
    tokio::time::sleep(Duration::from_millis(250)).await;

    assert_eq!(state(&e, &a), TaskState::Queued);
    assert_eq!(e.running_in_lane(Lane::Network), 1);
    assert_eq!(
        ticks_a.load(Ordering::SeqCst),
        frozen,
        "the carried-on upload moved while another held the only place"
    );

    finish_b.send(()).unwrap();
    until(&e, &b, TaskState::Completed).await;
    until(&e, &a, TaskState::Completed).await;
    assert!(ticks_a.load(Ordering::SeqCst) > frozen);
}

/// Every kind goes the same road, not only uploads: a preparation paused and carried on
/// beside another stays frozen (`is_paused`, which is what keeps the encoder suspended) until
/// its turn, instead of being thawed into a second encode in a lane for one.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_paused_preparation_carried_on_stays_frozen_until_its_turn() {
    let e = one_per_lane(Arc::new(Db::open_in_memory().unwrap()));

    let (release_a, a_release) = tokio::sync::oneshot::channel::<()>();
    let (seen_tx, seen_rx) = tokio::sync::oneshot::channel::<bool>();
    let a = e
        .submit(TaskKind::Convert, None, move |ctx| async move {
            // Stands for the encoder: frozen while paused, thawed when not.
            let _ = a_release.await;
            let _ = seen_tx.send(ctx.is_paused());
            ctx.wait_while_paused().await;
            Ok(())
        })
        .await
        .unwrap();
    until(&e, &a, TaskState::Running).await;
    e.pause(&a).unwrap();

    let (finish_b, b_done) = tokio::sync::oneshot::channel::<()>();
    let b = e
        .submit(TaskKind::Convert, None, move |_| async move {
            let _ = b_done.await;
            Ok(())
        })
        .await
        .unwrap();
    until(&e, &b, TaskState::Running).await;

    e.resume(&a).unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(state(&e, &a), TaskState::Queued);
    release_a.send(()).unwrap();
    assert!(
        seen_rx.await.unwrap(),
        "the encoder was thawed while another preparation held the lane"
    );

    finish_b.send(()).unwrap();
    until(&e, &b, TaskState::Completed).await;
    until(&e, &a, TaskState::Completed).await;
}

/// Pressing "carry on" twice is not an error and does not give two places; a carried-on task
/// waiting for its turn can still be dropped.
#[tokio::test]
async fn a_carried_on_task_waiting_its_turn_can_be_pressed_again_and_dropped() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    let e = one_per_lane(db.clone());

    let (finish, done) = tokio::sync::oneshot::channel::<()>();
    let holder = e
        .submit(TaskKind::Upload, None, move |_| async move {
            let _ = done.await;
            Ok(())
        })
        .await
        .unwrap();
    until(&e, &holder, TaskState::Running).await;

    let entered = Arc::new(AtomicUsize::new(0));
    let n = entered.clone();
    store::upsert(&db, &TaskRecord::new("waiting", TaskKind::Upload, None)).unwrap();
    e.resubmit_paused("waiting", move |_| async move {
        n.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })
    .unwrap();

    e.resume("waiting").unwrap();
    e.resume("waiting")
        .expect("a second press counted as an error");
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(state(&e, "waiting"), TaskState::Queued);
    assert_eq!(e.queue_order(), vec![String::from("waiting")]);

    e.cancel("waiting").unwrap();
    until(&e, "waiting", TaskState::Cancelled).await;
    assert_eq!(entered.load(Ordering::SeqCst), 0);

    finish.send(()).unwrap();
    until(&e, &holder, TaskState::Completed).await;
}

/// With room in the lane, "carry on" answers with the task already running — the reply a
/// person's screen reads straight after pressing the button shows the truth.
#[tokio::test]
async fn with_room_in_the_lane_carrying_on_is_running_at_once() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    let e = one_per_lane(db.clone());
    store::upsert(&db, &TaskRecord::new("alone", TaskKind::Upload, None)).unwrap();
    let (finish, done) = tokio::sync::oneshot::channel::<()>();
    e.resubmit_paused("alone", move |_| async move {
        let _ = done.await;
        Ok(())
    })
    .unwrap();

    e.resume("alone").unwrap();
    assert_eq!(state(&e, "alone"), TaskState::Running);
    assert_eq!(e.running_in_lane(Lane::Network), 1);

    finish.send(()).unwrap();
    until(&e, "alone", TaskState::Completed).await;
}
