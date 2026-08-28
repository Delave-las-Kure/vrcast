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
use vrcast_studio_lib::domain::ladder::{may_build_unasked, Objection};
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::tasks::engine::TaskEngine;
use vrcast_studio_lib::tasks::state::{TaskKind, TaskState};

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

    let at = source
        .find("submit(TaskKind::MeasureQuality")
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
