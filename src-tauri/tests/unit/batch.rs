//! T439–T442 — the chain from a measurement to a build, and what stops it.
//!
//! **What a batch is for.** A season goes in, and a season comes out on the server. The
//! decision between "these are the rungs" and "send them" is taken in the core, because by
//! then the window may be shut or in the tray — and a decision taken by a closed window is
//! taken by nobody.
//!
//! **What is checked here and what is not.** The gate and the lanes are checked here: both
//! are answerable without a film, an encoder or a server. Whether a real season gets through
//! — the build appearing only after the measurement ends, one failure not stopping the rest —
//! is the integration suite's, against the throwaway stand (T447). Standing in for ffmpeg and
//! a server here would check the stand-ins.

use std::sync::Arc;
use std::time::Duration;
use vrcast_studio_lib::commands::error::{AppError, ErrorCode};
use vrcast_studio_lib::domain::ladder::{may_build_unasked, Objection};
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::tasks::engine::TaskEngine;
use vrcast_studio_lib::tasks::state::{TaskKind, TaskState};
use vrcast_studio_lib::tasks::store::{self, Batch};

// ---------- the gate (T439, T442) ----------

#[test]
fn a_ladder_with_nothing_wrong_with_it_is_built_without_asking() {
    // **The negative control, and the one that makes the test below mean anything.** Without
    // it, "stop on an objection" is satisfied by stopping always — and a batch that never
    // builds anything would pass a check that only ever looked for refusals.
    assert!(may_build_unasked(&[]));
}

#[test]
fn every_kind_of_objection_stops_the_chain() {
    // Every kind, not one: a gate that lets four of the five through is a gate that will one
    // day send a ladder nobody looked at to a server, and the reason will be that somebody
    // matched on the objections they happened to think of.
    let each: Vec<Objection> = vec![
        Objection::RungAboveSource {
            index: 0,
            source_bps: 9_000_000,
        },
        Objection::BufsizeTooLarge {
            index: 1,
            maxrate_bps: 45_000_000,
        },
        Objection::LevelExceeded {
            index: 2,
            level: String::from("4.1"),
            limits: Vec::new(),
        },
        Objection::OutOfOrder { index: 3 },
        Objection::BadStep {
            index: 4,
            times: 2.75,
        },
    ];
    for objection in each {
        assert!(
            !may_build_unasked(std::slice::from_ref(&objection)),
            "this objection did not stop the chain: {objection:?}"
        );
    }
}

#[test]
fn every_objection_can_say_what_it_is_in_words_a_person_reads() {
    // A batch that stops and cannot say why is a batch nobody can act on. The wording lives
    // under the code — one set, shared with the ladder screen (T444) — and the check that both
    // catalogues have it is `every_code_has_a_wording_in_both_languages`.
    let objection = Objection::BadStep {
        index: 4,
        times: 2.75,
    };
    let detail = objection.detail();
    assert_eq!(
        detail.params.get("index"),
        Some(&serde_json::json!(5)),
        "the rung was named by its place in the array; a person counting rungs starts at one"
    );
    assert_eq!(detail.params.get("times"), Some(&serde_json::json!("2.8")));
}

// ---------- the lanes (T440) ----------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_measurement_and_a_build_run_at_the_same_time() {
    // **The negative control for "do not merge the two kinds into one".** Both lanes hold one
    // task at a time. Were `MeasureQuality` and `BuildLadder` one kind they would share a
    // lane, and the second film's measurement would wait for the first film's build — hours,
    // and a batch becomes a queue of one.
    //
    // **Each task waits for the other before finishing**, and that is the whole design of this
    // check. The first shape of it only asked whether both had started by the end, which is
    // true of two tasks run one after the other — it passed with the kinds deliberately
    // merged, and so checked nothing at all. Made to run at the same time or not at all: if
    // they are serialised, the first waits for a partner that cannot start until it has
    // finished, and the wait runs out.
    let db = Arc::new(Db::open_in_memory().unwrap());
    let engine = TaskEngine::new(db);
    let both_here = Arc::new(tokio::sync::Barrier::new(2));

    let mut ids = Vec::new();
    for kind in [TaskKind::BuildLadder, TaskKind::MeasureQuality] {
        let gate = both_here.clone();
        ids.push(
            engine
                .submit(kind, None, move |_ctx| async move {
                    match tokio::time::timeout(Duration::from_secs(3), gate.wait()).await {
                        Ok(_) => Ok(()),
                        Err(_) => Err(vrcast_studio_lib::commands::error::AppError::new(
                            vrcast_studio_lib::commands::error::ErrorCode::Internal,
                        )
                        .with_cause("the other kind never began: they are sharing a lane")),
                    }
                })
                .await
                .unwrap(),
        );
    }

    for id in ids {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if engine
                .get(&id)
                .ok()
                .flatten()
                .is_some_and(|t| t.state.is_final())
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        let task = engine.get(&id).unwrap().unwrap();
        assert_eq!(
            task.state,
            TaskState::Completed,
            "a measurement and a build could not run at the same time, so a season would go \
             through one film at a time: {:?}",
            task.error
        );
    }
}

// ---------- where the chain lives (T441) ----------

#[test]
fn the_chain_is_in_the_core_and_not_on_a_screen() {
    // **The check that would fail on the tempting implementation.** Driving the chain from a
    // screen — listen for the measurement to end, then call `ladder_build` — works perfectly
    // while somebody is looking at it, and does nothing at all once the window is shut or in
    // the tray. That is precisely when a batch is running.
    //
    // So the build has to be submitted from inside the measurement task itself. This reads the
    // source and says so: crude, and it bites on the one change that matters.
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/quality.rs"),
    )
    .expect("quality.rs would not read");

    // Either way in — `submit` or `submit_in_batch`. Named by the kind rather than by the
    // function, because the function is the part that has already changed once.
    let at = source
        .find("TaskKind::MeasureQuality,")
        .expect("the measurement no longer submits a task of its own kind");
    let closure = &source[at..];
    let ends = closure
        .find("\n            .await?;")
        .expect("the submitted closure could not be found");
    assert!(
        closure[..ends].contains("then_build("),
        "the measurement task does not put the build on the queue itself. If a screen does it \
         instead, a batch stops the moment the window is closed — which is exactly when a \
         batch is left to run."
    );
}

// ---------- stopping a whole batch (T445) ----------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stopping_a_batch_reaches_the_ones_that_have_not_started() {
    // **The fault this is written against.** A batch of ten films has one task running and
    // the rest waiting. A cancel that only reached what was running would stop the film in
    // hand and let the next nine begin — which is the opposite of what the button says, and
    // the person watching would see the list carry on and conclude nothing had happened.
    let db = Arc::new(Db::open_in_memory().unwrap());
    let engine = TaskEngine::new(db.clone());
    let ours = Batch {
        id: String::from("season-1"),
        label: String::from("Blue Eye Samurai"),
    };

    let mut mine = Vec::new();
    for _ in 0..3 {
        mine.push(
            engine
                .submit_in_batch(
                    TaskKind::Convert,
                    None,
                    Some(ours.clone()),
                    |ctx| async move {
                        // Long enough that they are still waiting when the batch is stopped.
                        for _ in 0..100 {
                            ctx.bail_if_cancelled()
                                .map_err(|_| AppError::new(ErrorCode::Internal))?;
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                        Ok(())
                    },
                )
                .await
                .unwrap(),
        );
    }
    // Somebody else's task, of the same kind, which must be left entirely alone.
    let stranger = engine
        .submit(TaskKind::Convert, None, |ctx| async move {
            for _ in 0..100 {
                ctx.bail_if_cancelled()
                    .map_err(|_| AppError::new(ErrorCode::Internal))?;
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Ok(())
        })
        .await
        .unwrap();

    let stopped = engine
        .cancel_batch(&ours.id)
        .expect("the batch would not stop");
    assert_eq!(stopped, 3, "the waiting ones were not reached");

    for id in &mine {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if engine
                .get(id)
                .ok()
                .flatten()
                .is_some_and(|t| t.state.is_final())
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(
            engine.get(id).unwrap().unwrap().state,
            TaskState::Cancelled,
            "a task of the batch went on after the batch was stopped"
        );
    }
    assert_ne!(
        engine.get(&stranger).unwrap().unwrap().state,
        TaskState::Cancelled,
        "stopping one batch stopped somebody else's work"
    );
    let _ = engine.cancel(&stranger);
}

#[tokio::test]
async fn a_task_says_which_film_it_belongs_to() {
    // Thirty rows saying "measuring quality" are a wall. The label sits on the task itself,
    // so it is still there after a restart and after the file has been renamed in the library.
    let db = Arc::new(Db::open_in_memory().unwrap());
    let engine = TaskEngine::new(db.clone());
    let id = engine
        .submit_in_batch(
            TaskKind::Probe,
            None,
            Some(Batch {
                id: String::from("season-1"),
                label: String::from("Blue Eye Samurai S01E04"),
            }),
            |_ctx| async move { Ok(()) },
        )
        .await
        .unwrap();

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if engine
            .get(&id)
            .ok()
            .flatten()
            .is_some_and(|t| t.state.is_final())
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Read back out of the store, not out of memory: this has to survive the application.
    let rec = vrcast_studio_lib::tasks::store::get(&db, &id)
        .unwrap()
        .expect("the record is gone");
    let batch = rec
        .batch
        .expect("the task forgot which batch it was part of");
    assert_eq!(batch.id, "season-1");
    assert_eq!(batch.label, "Blue Eye Samurai S01E04");
}

#[tokio::test]
async fn a_task_nobody_batched_belongs_to_no_batch() {
    // Otherwise every single-file job would draw a batch heading of its own, and a heading
    // that is always there is a heading nobody reads.
    let db = Arc::new(Db::open_in_memory().unwrap());
    let engine = TaskEngine::new(db.clone());
    let id = engine
        .submit(TaskKind::Probe, None, |_ctx| async move { Ok(()) })
        .await
        .unwrap();
    assert!(vrcast_studio_lib::tasks::store::get(&db, &id)
        .unwrap()
        .unwrap()
        .batch
        .is_none());
}

// ---------- when the build appears, and when it must not (T440, T441) ----------
//
// **What is checked here and what is not.** These drive the engine directly with stand-in
// work rather than measuring a real film: what is being checked is *when* the second task
// appears and whether anything can stop it, and a real measurement would take half an hour to
// answer a question about ordering. That a real season survives one film failing is T447, and
// it needs a real season — a stand-in that fails on command proves the engine carries on, not
// that a measurement does.

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn no_build_exists_while_the_measurement_is_still_running() {
    // **The whole shape of the chain in one check.** The build is submitted by the
    // measurement's own closure, at its very end. If it were submitted alongside — or by the
    // screen that started the measurement — a build would be queued against rungs nobody had
    // measured yet, and it would either wait in a lane or run on the guess.
    let db = std::sync::Arc::new(Db::open_in_memory().unwrap());
    let engine = TaskEngine::new(db.clone());
    let gate = std::sync::Arc::new(tokio::sync::Notify::new());

    let after = gate.clone();
    let onward = engine.clone();
    let measuring = engine
        .submit(TaskKind::MeasureQuality, None, move |_ctx| async move {
            after.notified().await;
            // The chain: the build goes in here, at the end, and nowhere else.
            onward
                .submit(TaskKind::BuildLadder, None, |_| async { Ok(()) })
                .await
                .map_err(|e| AppError::new(ErrorCode::Internal).with_cause(e))?;
            Ok(())
        })
        .await
        .unwrap();

    // While it runs there is no build anywhere — not queued, not paused, not anything.
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(
        !any_build(&db),
        "a build existed while the measurement was still running"
    );

    gate.notify_one();
    assert!(
        wait_for_state(
            &engine,
            &measuring,
            TaskState::Completed,
            Duration::from_secs(5)
        )
        .await
    );

    // And after it, there is.
    let appeared = wait_until(Duration::from_secs(5), || any_build(&db)).await;
    assert!(appeared, "the measurement finished and no build followed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_measurement_cancelled_partway_leaves_no_build_behind() {
    // T441(b). Cancelling is the person saying stop, and a chain that goes on to queue hours
    // of encoding after that has not stopped — it has changed what it is doing.
    let db = std::sync::Arc::new(Db::open_in_memory().unwrap());
    let engine = TaskEngine::new(db.clone());

    let onward = engine.clone();
    let measuring = engine
        .submit(TaskKind::MeasureQuality, None, move |ctx| async move {
            // Long enough to be cancelled in the middle of, and it checks — the chain runs
            // only where the work was not stopped.
            for _ in 0..100 {
                ctx.bail_if_cancelled()
                    .map_err(|_| AppError::new(ErrorCode::Internal))?;
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            onward
                .submit(TaskKind::BuildLadder, None, |_| async { Ok(()) })
                .await
                .map_err(|e| AppError::new(ErrorCode::Internal).with_cause(e))?;
            Ok(())
        })
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;
    engine.cancel(&measuring).expect("the cancel would not go");
    assert!(
        wait_for_state(
            &engine,
            &measuring,
            TaskState::Cancelled,
            Duration::from_secs(5)
        )
        .await
    );

    // Given time to do the wrong thing, and it does not.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !any_build(&db),
        "the measurement was cancelled and a build was queued anyway"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_chain_runs_with_nobody_listening() {
    // T441(c). **This is the one that would fail on any implementation where a screen drives
    // the chain**, and it is the reason the chain is in the core at all: the window may be
    // shut, or in the tray, when the measurement ends. Nothing here subscribes to the event
    // stream, and the build must appear regardless.
    let db = std::sync::Arc::new(Db::open_in_memory().unwrap());
    let engine = TaskEngine::new(db.clone());

    let onward = engine.clone();
    engine
        .submit(TaskKind::MeasureQuality, None, move |_ctx| async move {
            onward
                .submit(TaskKind::BuildLadder, None, |_| async { Ok(()) })
                .await
                .map_err(|e| AppError::new(ErrorCode::Internal).with_cause(e))?;
            Ok(())
        })
        .await
        .unwrap();

    assert!(
        wait_until(Duration::from_secs(5), || any_build(&db)).await,
        "the chain did not run with nobody subscribed to its events"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_film_failing_does_not_stop_the_others() {
    // T441(a), as far as an engine can answer it: the second film's chain fails, and its build
    // **never** appears, while the first and third both reach theirs. What a real season adds
    // is whether a real measurement fails the way this stand-in does, and that is T447.
    let db = std::sync::Arc::new(Db::open_in_memory().unwrap());
    let engine = TaskEngine::new(db.clone());

    for (film, ok) in [("a", true), ("b", false), ("c", true)] {
        let onward = engine.clone();
        // Named by the batch label, which is what it is for: `server_id` refers to a real
        // profile and cannot stand in for a film's name.
        let mark = Some(Batch {
            id: String::from("one-press"),
            label: String::from(film),
        });
        engine
            .submit_in_batch(
                TaskKind::MeasureQuality,
                None,
                mark.clone(),
                move |_ctx| async move {
                    if !ok {
                        return Err(AppError::new(ErrorCode::Internal));
                    }
                    onward
                        .submit_in_batch(TaskKind::BuildLadder, None, mark, |_| async { Ok(()) })
                        .await
                        .map_err(|e| AppError::new(ErrorCode::Internal).with_cause(e))?;
                    Ok(())
                },
            )
            .await
            .unwrap();
    }

    // Two builds, and never a third.
    assert!(
        wait_until(Duration::from_secs(5), || builds_for(&db, "a")
            && builds_for(&db, "c"))
        .await,
        "the films either side of the failure did not reach their builds"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !builds_for(&db, "b"),
        "the film that failed got a build anyway"
    );
}

/// Whether any build task exists at all, in any state.
fn any_build(db: &Db) -> bool {
    store::list(db)
        .unwrap_or_default()
        .iter()
        .any(|t| t.kind == TaskKind::BuildLadder)
}

/// Whether a build exists for one film, by the label its batch gave it.
fn builds_for(db: &Db, film: &str) -> bool {
    store::list(db).unwrap_or_default().iter().any(|t| {
        t.kind == TaskKind::BuildLadder && t.batch.as_ref().is_some_and(|b| b.label == film)
    })
}

/// Wait for something to become true, or give up.
async fn wait_until(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = std::time::Instant::now() + limit;
    while std::time::Instant::now() < deadline {
        if done() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    false
}

/// The same helper the engine's own tests use, kept here so this file stands alone.
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
