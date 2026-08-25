//! Tests for the task machinery (T016, T017, T019, T020).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use vrcast_studio_lib::commands::error::{AppError, DetailCode, ErrorCode};
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::tasks::engine::{TaskEngine, TaskError, TaskEvent};
use vrcast_studio_lib::tasks::progress::ProgressThrottle;
use vrcast_studio_lib::tasks::state::{Lane, LaneLimits, PauseKind, TaskKind, TaskState};
use vrcast_studio_lib::tasks::store;

fn engine() -> TaskEngine {
    TaskEngine::new(Arc::new(Db::open_in_memory().unwrap()))
}

async fn wait_for_state(e: &TaskEngine, id: &str, want: TaskState, limit: Duration) -> bool {
    let deadline = std::time::Instant::now() + limit;
    while std::time::Instant::now() < deadline {
        if let Ok(Some(rec)) = e.get(id) {
            if rec.state == want {
                return true;
            }
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    false
}

// ---------- state transitions: the pure logic ----------

#[test]
fn the_state_transitions_obey_the_table() {
    use TaskState::*;
    assert!(Queued.can_transition_to(Running));
    assert!(Queued.can_transition_to(Cancelled));
    assert!(Running.can_transition_to(Paused));
    assert!(Paused.can_transition_to(Running));
    assert!(Running.can_transition_to(Completed));

    // Straight from the queue to finished will not do: the task never ran.
    assert!(!Queued.can_transition_to(Completed));
    // Out of the finished states there are no transitions at all.
    assert!(!Completed.can_transition_to(Running));
    assert!(!Cancelled.can_transition_to(Running));
    assert!(!Failed.can_transition_to(Running));

    // A transition into itself is allowed: cancelling twice is not an error (principle V).
    assert!(Cancelled.can_transition_to(Cancelled));

    // Out of paused, finishing is possible too. A pause takes effect at the nearest
    // stopping point, and the work manages to run to its end while the task is already
    // marked paused: a transfer finishes writing its last window. The table used to forbid
    // this while the engine did it anyway — and the disagreement said nothing (debt T072).
    assert!(Paused.can_transition_to(Completed));
    assert!(Paused.can_transition_to(Failed));
}

#[tokio::test]
async fn a_task_that_finished_while_paused_is_recorded_as_finished() {
    // What is checked is the engine rather than the table: it wrote "completed" here before
    // as well, but the table forbade it, and nobody knew which of them was right.
    let e = engine();
    let reached = Arc::new(AtomicUsize::new(0));
    let d = reached.clone();

    let id = e
        .submit(TaskKind::Upload, None, move |ctx| async move {
            // The work manages to end although a pause has already been asked for: the
            // stopping point itself lies ahead, and it never gets there.
            tokio::time::sleep(Duration::from_millis(150)).await;
            d.fetch_add(1, Ordering::SeqCst);
            let _ = ctx;
            Ok(())
        })
        .await
        .unwrap();

    assert!(wait_for_state(&e, &id, TaskState::Running, Duration::from_secs(3)).await);
    e.pause(&id).expect("the task would not pause");

    assert!(
        wait_for_state(&e, &id, TaskState::Completed, Duration::from_secs(5)).await,
        "the work ran to its end but was not recorded as finished: {:?}",
        e.get(&id).unwrap().unwrap().state
    );
    assert_eq!(reached.load(Ordering::SeqCst), 1);
}

#[test]
fn the_lanes_separate_tasks_by_resource() {
    // A preparation is bound by computation and a transfer by the network: there is no
    // reason for them to get in each other's way, while two preparations at once are each
    // twice as slow.
    assert_eq!(TaskKind::Convert.lane(), Lane::Compute);
    assert_eq!(TaskKind::Upload.lane(), Lane::Network);
    assert_eq!(TaskKind::Probe.lane(), Lane::Light);
    assert_ne!(TaskKind::Convert.lane(), TaskKind::Upload.lane());

    let l = LaneLimits::default();
    assert_eq!(l.for_lane(Lane::Compute), 1);
    assert_eq!(l.for_lane(Lane::Network), 1);
    assert!(l.for_lane(Lane::Light) > 1);
}

#[test]
fn the_kinds_of_task_take_pausing_differently() {
    // The difference is not cosmetic: what to tell a person when the application closes
    // depends on it (FR-086).
    assert_eq!(
        TaskKind::Upload.pause_kind(),
        PauseKind::ResumableAcrossRestart
    );
    assert_eq!(TaskKind::Convert.pause_kind(), PauseKind::SuspendedProcess);
    assert_eq!(TaskKind::Probe.pause_kind(), PauseKind::NotPausable);
}

// ---------- capping the rate of events ----------

#[test]
fn frequent_progress_events_are_filtered_out() {
    let t = ProgressThrottle::new(Duration::from_millis(250));
    let start = std::time::Instant::now();

    assert!(t.allow_at(start, false), "the first event must get through");
    assert!(!t.allow_at(start + Duration::from_millis(50), false));
    assert!(!t.allow_at(start + Duration::from_millis(200), false));
    assert!(t.allow_at(start + Duration::from_millis(300), false));
}

#[test]
fn an_important_event_always_gets_through() {
    // Without this exception the figure sticks at 87 % on a task that has already ended.
    let t = ProgressThrottle::new(Duration::from_millis(250));
    let start = std::time::Instant::now();

    assert!(t.allow_at(start, false));
    assert!(!t.allow_at(start + Duration::from_millis(10), false));
    assert!(
        t.allow_at(start + Duration::from_millis(11), true),
        "an important event was filtered out by the cap"
    );
}

// ---------- the queue ----------

#[tokio::test]
async fn a_task_runs_and_finishes() {
    let e = engine();
    let id = e
        .submit(TaskKind::Probe, None, |ctx| async move {
            ctx.report_important(0.5, DetailCode::StageConverting);
            Ok(())
        })
        .await
        .unwrap();

    assert!(
        wait_for_state(&e, &id, TaskState::Completed, Duration::from_secs(5)).await,
        "the task did not finish"
    );
    let rec = e.get(&id).unwrap().unwrap();
    assert_eq!(rec.progress, 1.0, "a finished task's progress must be full");
    assert!(rec.error.is_none());
}

#[tokio::test]
async fn a_failure_is_recorded_as_a_failure_rather_than_a_success() {
    let e = engine();
    let id = e
        .submit(TaskKind::Probe, None, |_ctx| async move {
            Err(AppError::new(ErrorCode::InvalidInput).detail(DetailCode::ProbeUnreadable))
        })
        .await
        .unwrap();

    assert!(wait_for_state(&e, &id, TaskState::Failed, Duration::from_secs(5)).await);
    let rec = e.get(&id).unwrap().unwrap();
    // The error is stored as an object rather than a sentence: a task that failed a week
    // ago explains itself in whichever language is chosen today.
    let error = rec.error.expect("the failure was recorded with no cause");
    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert!(error.says(DetailCode::ProbeUnreadable));
    assert!(
        rec.progress < 1.0,
        "a failed task's progress must not be full"
    );
}

#[tokio::test]
async fn a_second_task_in_the_same_lane_waits_for_the_first() {
    let e = engine();
    let running = Arc::new(AtomicUsize::new(0));
    let max_seen = Arc::new(AtomicUsize::new(0));

    let mut ids = Vec::new();
    for _ in 0..3 {
        let r = running.clone();
        let m = max_seen.clone();
        let id = e
            .submit(TaskKind::Convert, None, move |_ctx| async move {
                let now = r.fetch_add(1, Ordering::SeqCst) + 1;
                m.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(250)).await;
                r.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            })
            .await
            .unwrap();
        ids.push(id);
    }

    for id in &ids {
        assert!(
            wait_for_state(&e, id, TaskState::Completed, Duration::from_secs(10)).await,
            "the task {id} did not finish"
        );
    }

    assert_eq!(
        max_seen.load(Ordering::SeqCst),
        1,
        "more than one task ran in the compute lane at once"
    );
}

#[tokio::test]
async fn tasks_in_different_lanes_run_at_the_same_time() {
    // The counterpart of the previous one: a single limit over all tasks would be wrong.
    let e = engine();
    let together = Arc::new(AtomicUsize::new(0));
    let max_seen = Arc::new(AtomicUsize::new(0));

    let mut ids = Vec::new();
    for kind in [TaskKind::Convert, TaskKind::Upload] {
        let t = together.clone();
        let m = max_seen.clone();
        let id = e
            .submit(kind, None, move |_ctx| async move {
                let now = t.fetch_add(1, Ordering::SeqCst) + 1;
                m.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(400)).await;
                t.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            })
            .await
            .unwrap();
        ids.push(id);
    }

    for id in &ids {
        assert!(wait_for_state(&e, id, TaskState::Completed, Duration::from_secs(10)).await);
    }

    assert_eq!(
        max_seen.load(Ordering::SeqCst),
        2,
        "the preparation and the transfer did not run together although they take different resources"
    );
}

// ---------- cancelling ----------

#[tokio::test]
async fn cancelling_breaks_off_a_running_task() {
    let e = engine();
    let finished_work = Arc::new(AtomicUsize::new(0));
    let fw = finished_work.clone();

    let id = e
        .submit(TaskKind::Convert, None, move |ctx| async move {
            for _ in 0..100 {
                ctx.bail_if_cancelled()?;
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            fw.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .await
        .unwrap();

    assert!(wait_for_state(&e, &id, TaskState::Running, Duration::from_secs(5)).await);
    e.cancel(&id).unwrap();

    assert!(
        wait_for_state(&e, &id, TaskState::Cancelled, Duration::from_secs(5)).await,
        "the task did not move to cancelled"
    );
    assert_eq!(
        finished_work.load(Ordering::SeqCst),
        0,
        "the work ran to its end although the task was cancelled"
    );
}

#[tokio::test]
async fn a_cancelled_task_does_not_count_as_failed() {
    // The difference shows to a person: a task they dropped must not look like an error.
    // The work is long with cancellation points rather than "sleep 200 ms": a short one
    // would manage to finish before the cancel on a loaded machine, and the unwrap would
    // fail the test for nothing.
    let e = engine();
    let id = e
        .submit(TaskKind::Convert, None, |ctx| async move {
            for _ in 0..600 {
                ctx.bail_if_cancelled()?;
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Ok(())
        })
        .await
        .unwrap();

    assert!(wait_for_state(&e, &id, TaskState::Running, Duration::from_secs(5)).await);
    e.cancel(&id).unwrap();
    assert!(wait_for_state(&e, &id, TaskState::Cancelled, Duration::from_secs(5)).await);

    let rec = e.get(&id).unwrap().unwrap();
    assert_eq!(rec.state, TaskState::Cancelled);
    assert!(rec.error.is_none(), "a cancelled task must hold no error");
}

#[tokio::test]
async fn a_task_can_be_dropped_straight_out_of_the_queue() {
    // A task standing in the queue has not started — it must be droppable without waiting
    // for it to.
    let e = TaskEngine::new(Arc::new(Db::open_in_memory().unwrap())).with_limits(LaneLimits {
        compute: 1,
        network: 1,
        light: 1,
    });
    let started = Arc::new(AtomicUsize::new(0));

    let s1 = started.clone();
    let blocker = e
        .submit(TaskKind::Convert, None, move |_c| async move {
            s1.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(900)).await;
            Ok(())
        })
        .await
        .unwrap();

    let s2 = started.clone();
    let queued = e
        .submit(TaskKind::Convert, None, move |_c| async move {
            s2.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .await
        .unwrap();

    assert!(wait_for_state(&e, &blocker, TaskState::Running, Duration::from_secs(5)).await);
    e.cancel(&queued).unwrap();

    assert!(
        wait_for_state(&e, &queued, TaskState::Cancelled, Duration::from_secs(5)).await,
        "the task in the queue was not dropped"
    );
    assert_eq!(
        started.load(Ordering::SeqCst),
        1,
        "a task dropped from the queue started after all"
    );

    e.cancel(&blocker).unwrap();
}

// ---------- pausing ----------

#[tokio::test]
async fn pausing_stops_the_work_and_carrying_on_resumes_it() {
    let e = engine();
    let steps = Arc::new(AtomicUsize::new(0));
    let s = steps.clone();

    let id = e
        .submit(TaskKind::Upload, None, move |ctx| async move {
            for _ in 0..200 {
                ctx.wait_while_paused().await;
                if ctx.is_cancelled() {
                    return Ok(());
                }
                s.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Ok(())
        })
        .await
        .unwrap();

    assert!(wait_for_state(&e, &id, TaskState::Running, Duration::from_secs(5)).await);
    tokio::time::sleep(Duration::from_millis(150)).await;

    e.pause(&id).unwrap();
    // The iteration already begun is given room to reach the stopping point: a short window
    // here turned into a false "the work carried on" under preemption on a loaded machine.
    tokio::time::sleep(Duration::from_millis(400)).await;
    let frozen = steps.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        steps.load(Ordering::SeqCst),
        frozen,
        "the work carried on after the pause"
    );

    e.resume(&id).unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        steps.load(Ordering::SeqCst) > frozen,
        "the work did not resume after carrying on"
    );

    e.cancel(&id).unwrap();
}

#[tokio::test]
async fn cancelling_wakes_and_drops_a_task_standing_paused() {
    // The defect this test exists for: cancelling does not clear the pause flag, and a task
    // asleep in wait_while_paused woke on the notify, saw "still paused" and fell asleep
    // again — forever. No cancellation, no event, no record in the database.
    let e = engine();
    let id = e
        .submit(TaskKind::Upload, None, move |ctx| async move {
            for _ in 0..600 {
                ctx.wait_while_paused().await;
                if ctx.is_cancelled() {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Ok(())
        })
        .await
        .unwrap();

    assert!(wait_for_state(&e, &id, TaskState::Running, Duration::from_secs(5)).await);
    e.pause(&id).unwrap();
    // The task is given time to reach the stopping point and really fall asleep in it.
    tokio::time::sleep(Duration::from_millis(200)).await;

    e.cancel(&id).unwrap();
    assert!(
        wait_for_state(&e, &id, TaskState::Cancelled, Duration::from_secs(5)).await,
        "a task cancelled while paused hung forever"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_simultaneous_start_does_not_slip_past_the_lane_limit() {
    // The defect this test exists for: checking for room and changing the state went under
    // separate takes of the lock, and two tasks waking at the same moment on different
    // threads both saw one free place — two preparations in a lane meant for one. A
    // single-threaded runtime hides that race, so this one is multi-threaded.
    let e = engine();
    let running = Arc::new(AtomicUsize::new(0));
    let max_seen = Arc::new(AtomicUsize::new(0));

    let mut ids = Vec::new();
    for _ in 0..6 {
        let r = running.clone();
        let m = max_seen.clone();
        let id = e
            .submit(TaskKind::Convert, None, move |_ctx| async move {
                let now = r.fetch_add(1, Ordering::SeqCst) + 1;
                m.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(60)).await;
                r.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            })
            .await
            .unwrap();
        ids.push(id);
    }

    for id in &ids {
        assert!(
            wait_for_state(&e, id, TaskState::Completed, Duration::from_secs(15)).await,
            "the task {id} did not finish"
        );
    }

    assert_eq!(
        max_seen.load(Ordering::SeqCst),
        1,
        "two tasks took the compute lane at once"
    );
}

#[tokio::test]
async fn a_short_task_cannot_be_paused() {
    let e = engine();
    let id = e
        .submit(TaskKind::Probe, None, |_c| async move {
            tokio::time::sleep(Duration::from_millis(600)).await;
            Ok(())
        })
        .await
        .unwrap();

    assert!(wait_for_state(&e, &id, TaskState::Running, Duration::from_secs(5)).await);
    let err = e
        .pause(&id)
        .expect_err("examining a source must not be pausable");
    // The kind of refusal is checked rather than its text: a lower layer's error text is a
    // detail for the log, while the interface tells a person in its own words.
    assert!(
        matches!(err, TaskError::NotPausable),
        "the pause was refused with the wrong kind of refusal: {err}"
    );
}

// ---------- surviving a restart ----------

#[tokio::test]
async fn an_interrupted_task_becomes_paused_rather_than_finished() {
    // Constitution, principle III and SC-010. "Completed" would mean the result is ready
    // while it was cut off halfway — the most dangerous substitution of all.
    let db = Arc::new(Db::open_in_memory().unwrap());

    let mut rec = store::TaskRecord::new("t-interrupted", TaskKind::Upload, None);
    rec.state = TaskState::Running;
    rec.progress = 0.42;
    rec.resume_token = Some(String::from("12400000000"));
    store::upsert(&db, &rec).unwrap();

    let e = TaskEngine::new(db.clone());
    let report = e.recover_after_start().unwrap();

    assert_eq!(report.interrupted, vec!["t-interrupted".to_string()]);
    let after = store::get(&db, "t-interrupted").unwrap().unwrap();
    assert_eq!(after.state, TaskState::Paused, "the state is not paused");
    assert_ne!(after.state, TaskState::Completed);
    assert_eq!(
        after.resume_token.as_deref(),
        Some("12400000000"),
        "the resume position was lost"
    );
}

// ---------- the queue's order (T096, FR-083) ----------

/// An engine with a lane for one task, plus a shared record of the order they ran in.
///
/// The order can only be checked this way: from inside a task, at the moment it got going.
/// The database records do not show it — only the outcome stays there.
fn queue_of_one() -> (TaskEngine, Arc<Mutex<Vec<String>>>) {
    let e = TaskEngine::new(Arc::new(Db::open_in_memory().unwrap())).with_limits(LaneLimits {
        compute: 1,
        network: 1,
        light: 1,
    });
    (e, Arc::new(Mutex::new(Vec::new())))
}

/// Submit a task that notes itself in the record and waits for the go-ahead.
///
/// The wait is needed so the queue has time to form: without it the first task finishes
/// before the third is submitted, and there is nothing to reorder. The go-ahead is a polled
/// flag rather than a signal: a signal wakes only those waiting for it right then, while the
/// tasks here reach the wait one after another, freeing the lane as they go.
async fn submit_named(
    e: &TaskEngine,
    name: &str,
    order: Arc<Mutex<Vec<String>>>,
    release: Arc<std::sync::atomic::AtomicBool>,
) -> String {
    let name = name.to_owned();
    e.submit(TaskKind::Upload, None, move |_ctx| async move {
        order.lock().unwrap().push(name);
        while !release.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok(())
    })
    .await
    .expect("the task would not submit")
}

async fn all_finished(e: &TaskEngine, ids: &[&String]) {
    for id in ids {
        assert!(
            wait_for_state(e, id, TaskState::Completed, Duration::from_secs(10)).await,
            "the task {id} did not finish"
        );
    }
}

#[tokio::test]
async fn without_reordering_the_tasks_run_in_the_order_they_were_submitted() {
    let (e, order) = queue_of_one();
    let release = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let first = submit_named(&e, "first", order.clone(), release.clone()).await;
    let second = submit_named(&e, "second", order.clone(), release.clone()).await;
    let third = submit_named(&e, "third", order.clone(), release.clone()).await;

    assert!(wait_for_state(&e, &first, TaskState::Running, Duration::from_secs(3)).await);
    assert_eq!(
        e.queue_order(),
        vec![second.clone(), third.clone()],
        "the queue is shown in an order other than the one the tasks will run in"
    );

    release.store(true, Ordering::SeqCst);
    all_finished(&e, &[&first, &second, &third]).await;

    assert_eq!(
        *order.lock().unwrap(),
        vec!["first", "second", "third"],
        "the tasks ran in an order other than the one they were submitted in"
    );
}

#[tokio::test]
async fn reordering_changes_which_task_runs_next() {
    // This is what FR-083 exists for. It is checked by which task actually got going rather
    // than by a field in the database: a field can be reordered with no consequences, and
    // then the button in the interface would be a lie.
    let (e, order) = queue_of_one();
    let release = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let first = submit_named(&e, "first", order.clone(), release.clone()).await;
    let second = submit_named(&e, "second", order.clone(), release.clone()).await;
    let third = submit_named(&e, "third", order.clone(), release.clone()).await;

    assert!(wait_for_state(&e, &first, TaskState::Running, Duration::from_secs(3)).await);

    // The person changed their mind: the third is wanted before the second.
    let moved = e
        .reorder_queue(&[third.clone(), second.clone()])
        .expect("the reordering failed");
    assert_eq!(moved, 2);
    assert_eq!(e.queue_order(), vec![third.clone(), second.clone()]);

    release.store(true, Ordering::SeqCst);
    all_finished(&e, &[&first, &second, &third]).await;

    assert_eq!(
        *order.lock().unwrap(),
        vec!["first", "third", "second"],
        "the reordering did not change which task ran next"
    );
}

#[tokio::test]
async fn reordering_does_not_break_off_a_task_already_begun() {
    // Breaking off a running task for the sake of the order would throw away work already
    // done — on an upload that runs for hours, that is hours.
    let (e, order) = queue_of_one();
    let release = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let first = submit_named(&e, "first", order.clone(), release.clone()).await;
    let second = submit_named(&e, "second", order.clone(), release.clone()).await;

    assert!(wait_for_state(&e, &first, TaskState::Running, Duration::from_secs(3)).await);

    // The request includes the task already begun — that is how it arrives from the list on
    // the screen.
    let moved = e
        .reorder_queue(&[second.clone(), first.clone()])
        .expect("the reordering failed");
    assert_eq!(
        moved, 0,
        "there was nothing to reorder: only one task is waiting"
    );
    assert_eq!(
        e.get(&first).unwrap().unwrap().state,
        TaskState::Running,
        "a running task was broken off for the sake of the order"
    );
}

#[tokio::test]
async fn the_order_survives_a_restart_of_the_application() {
    // Otherwise a person lines the queue up for the night, closes the application, and finds
    // the old order in the morning.
    let db = Arc::new(Db::open_in_memory().unwrap());
    let e = TaskEngine::new(db.clone()).with_limits(LaneLimits {
        compute: 1,
        network: 1,
        light: 1,
    });
    let order = Arc::new(Mutex::new(Vec::new()));
    let release = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let first = submit_named(&e, "first", order.clone(), release.clone()).await;
    let second = submit_named(&e, "second", order.clone(), release.clone()).await;
    let third = submit_named(&e, "third", order.clone(), release.clone()).await;

    assert!(wait_for_state(&e, &first, TaskState::Running, Duration::from_secs(3)).await);
    e.reorder_queue(&[third.clone(), second.clone()])
        .expect("the reordering failed");

    // A new run reads the same database.
    let orders: Vec<(String, i64)> = store::list(&db)
        .unwrap()
        .into_iter()
        .map(|t| (t.id, t.queue_order))
        .collect();
    let place = |id: &str| orders.iter().find(|(i, _)| i == id).unwrap().1;
    assert!(
        place(&third) < place(&second),
        "the reordering never reached the database and will not survive a restart"
    );
}

#[tokio::test]
async fn a_new_task_goes_to_the_end_of_the_previous_run_s_queue() {
    // The count of places carries on rather than starting afresh: otherwise a task submitted
    // after a restart would quietly cut into the middle of somebody else's queue.
    let db = Arc::new(Db::open_in_memory().unwrap());
    let mut old = store::TaskRecord::new("t-old", TaskKind::Upload, None);
    old.state = TaskState::Queued;
    old.queue_order = 100;
    store::upsert(&db, &old).unwrap();

    let e = TaskEngine::new(db.clone());
    let id = e
        .submit(TaskKind::Upload, None, |_| async { Ok(()) })
        .await
        .unwrap();

    assert!(
        e.get(&id).unwrap().unwrap().queue_order > 100,
        "the new task went ahead of a task from the previous run"
    );
}

#[tokio::test]
async fn a_task_from_the_previous_run_is_raised_and_carries_on() {
    // FR-031. Without this a task shows in the list as paused after the application restarts
    // while there is nothing to carry it on with: the working part lives only in memory and
    // dies along with the application. To a person that looks like "the task is there but
    // the button does nothing".
    let db = Arc::new(Db::open_in_memory().unwrap());

    // The previous run is portrayed: the task was running, the application died.
    let mut rec = store::TaskRecord::new("t-previous", TaskKind::Upload, None);
    rec.state = TaskState::Running;
    rec.progress = 0.4;
    store::upsert(&db, &rec).unwrap();

    let e = TaskEngine::new(db.clone());
    e.recover_after_start().unwrap();
    assert_eq!(
        e.get("t-previous").unwrap().unwrap().state,
        TaskState::Paused,
        "an interrupted task must become paused rather than finished"
    );

    let ran = Arc::new(AtomicUsize::new(0));
    let r = ran.clone();
    e.resubmit_paused("t-previous", move |ctx| async move {
        ctx.wait_while_paused().await;
        r.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })
    .expect("the task would not be raised");

    // A raised task waits for a person and does not start by itself.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        ran.load(Ordering::SeqCst),
        0,
        "a raised task carried on unbidden — while the application may have been closed \
         precisely to stop it"
    );
    assert_eq!(
        e.get("t-previous").unwrap().unwrap().state,
        TaskState::Paused
    );

    // And carries on at a person's word — under the same identifier it had.
    e.resume("t-previous")
        .expect("the raised task would not carry on");
    assert!(
        wait_for_state(
            &e,
            "t-previous",
            TaskState::Completed,
            Duration::from_secs(5)
        )
        .await,
        "the task that carried on did not run to its end"
    );
    assert_eq!(ran.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_raised_task_does_not_wait_for_its_own_place_in_the_lane() {
    // It already counts as running, and, counting itself, would never see a free place in a
    // lane that holds one.
    let db = Arc::new(Db::open_in_memory().unwrap());
    let mut rec = store::TaskRecord::new("t-alone", TaskKind::Convert, None);
    rec.state = TaskState::Running;
    store::upsert(&db, &rec).unwrap();

    let e = TaskEngine::new(db.clone()).with_limits(LaneLimits {
        compute: 1,
        network: 1,
        light: 1,
    });
    e.recover_after_start().unwrap();

    e.resubmit_paused("t-alone", |ctx| async move {
        ctx.wait_while_paused().await;
        Ok(())
    })
    .unwrap();
    e.resume("t-alone").unwrap();

    assert!(
        wait_for_state(&e, "t-alone", TaskState::Completed, Duration::from_secs(5)).await,
        "the task stuck waiting for a place it takes up itself"
    );
}

#[tokio::test]
async fn a_task_from_the_previous_run_can_be_dropped_without_raising_it() {
    // Otherwise it stays in the list as paused forever, with nothing to drop it with.
    let db = Arc::new(Db::open_in_memory().unwrap());
    let mut rec = store::TaskRecord::new("t-unwanted", TaskKind::Upload, None);
    rec.state = TaskState::Paused;
    store::upsert(&db, &rec).unwrap();

    let e = TaskEngine::new(db.clone());
    e.cancel("t-unwanted").expect("the task would not drop");

    assert_eq!(
        e.get("t-unwanted").unwrap().unwrap().state,
        TaskState::Cancelled
    );
    // Repeating is safe (constitution, principle V).
    e.cancel("t-unwanted")
        .expect("dropping a second time counted as an error");
    // And one that does not exist cannot be dropped — which is not the same as one already
    // dropped.
    assert!(e.cancel("no-such-task").is_err());
}

#[test]
fn a_pointed_write_does_not_clobber_other_fields() {
    // The defect this test exists for: both the token and the state were written through
    // read-modify-write of the whole record, and a pause and a token write running side by
    // side clobbered each other. Pointed updates must leave the other fields alone.
    let db = Db::open_in_memory().unwrap();
    let mut rec = store::TaskRecord::new("t-pointed", TaskKind::Upload, None);
    rec.resume_token = Some(String::from("the-old-token"));
    store::upsert(&db, &rec).unwrap();

    assert!(store::save_state(&db, "t-pointed", TaskState::Paused, None).unwrap());
    store::save_resume_token(&db, "t-pointed", "the-fresh-token").unwrap();

    let after = store::get(&db, "t-pointed").unwrap().unwrap();
    assert_eq!(
        after.state,
        TaskState::Paused,
        "writing the token clobbered the state"
    );
    assert_eq!(
        after.resume_token.as_deref(),
        Some("the-fresh-token"),
        "writing the state clobbered the token"
    );

    // The error is added without wiping the token already written.
    let failure = AppError::new(ErrorCode::SshUnreachable);
    assert!(store::save_state(&db, "t-pointed", TaskState::Failed, Some(&failure)).unwrap());
    let after = store::get(&db, "t-pointed").unwrap().unwrap();
    assert_eq!(after.resume_token.as_deref(), Some("the-fresh-token"));
    assert_eq!(
        after.error,
        Some(failure),
        "the error did not survive writing and reading"
    );

    // There is no record — save_state says so honestly rather than keeping quiet.
    assert!(!store::save_state(&db, "t-no-such-task", TaskState::Failed, None).unwrap());
}

#[tokio::test]
async fn the_resume_position_is_saved_and_read_back() {
    let e = engine();
    let id = e
        .submit(TaskKind::Upload, None, |ctx| async move {
            assert!(
                ctx.resume_token().unwrap().is_none(),
                "the position came out of nowhere"
            );
            ctx.save_resume_token("8388608")?;
            assert_eq!(ctx.resume_token().unwrap().as_deref(), Some("8388608"));
            Ok(())
        })
        .await
        .unwrap();

    assert!(wait_for_state(&e, &id, TaskState::Completed, Duration::from_secs(5)).await);
    assert_eq!(
        e.get(&id).unwrap().unwrap().resume_token.as_deref(),
        Some("8388608")
    );
}

// ---------- events ----------

#[tokio::test]
async fn finishing_is_reported_by_an_event() {
    let e = engine();
    let mut rx = e.subscribe();

    let id = e
        .submit(TaskKind::Probe, None, |ctx| async move {
            ctx.report_important(0.5, DetailCode::StageConverting);
            Ok(())
        })
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut got_done = false;
    while tokio::time::Instant::now() < deadline && !got_done {
        if let Ok(Ok(TaskEvent::Done {
            id: done_id, state, ..
        })) = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await
        {
            if done_id == id {
                assert_eq!(state, TaskState::Completed);
                got_done = true;
            }
        }
    }
    assert!(got_done, "no event about finishing arrived");
}
