//! T564 — the retention purge for finished tasks (`store::purge_finished_before`).
//!
//! **What was found.** `purge_finished_before` was written, worked, and was never called
//! from anywhere: not `lib.rs`, not `TaskEngine`, not a periodic job. `store::list()` reads
//! every row the table has ever held, `SELECT * FROM tasks ORDER BY created_at DESC` with no
//! `LIMIT` and no age filter, so a person who keeps the application running for months
//! accumulates one row per task forever. This is the third time this project has caught a
//! written-but-unwired function (T504, T479, T366 for other modules) — the pattern is real
//! enough to be worth a name.
//!
//! **Why `updated_at`, not `created_at`, despite the task text.** The task's own wording
//! says "добавить индекс на `tasks(created_at)`" — but the function actually filters
//! `WHERE ... AND updated_at < ?1`, not `created_at`. An index on the wrong column helps
//! nothing: SQLite would still scan every row to evaluate the real predicate. Re-reading the
//! function's own source before trusting the task's restatement of it is exactly the
//! discipline the coordinating brief asked for, and it mattered here.

use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::tasks::state::{TaskKind, TaskState};
use vrcast_studio_lib::tasks::store::{self, TaskRecord};

fn db() -> Db {
    Db::open_in_memory().expect("the throwaway database would not open")
}

/// A finished task, `age_days` old by its `updated_at` — the column the purge actually
/// reads, not `created_at`, which purge never looks at.
fn finished_task_aged(db: &Db, id: &str, state: TaskState, age_days: i64) {
    let mut task = TaskRecord::new(id, TaskKind::Convert, None);
    task.state = state;
    let stamp = (time::OffsetDateTime::now_utc() - time::Duration::days(age_days))
        .format(&time::format_description::well_known::Rfc3339)
        .expect("the timestamp would not format");
    task.created_at = stamp.clone();
    task.updated_at = stamp;
    store::upsert(db, &task).expect("the fixture task would not write");
}

fn threshold(days_ago: i64) -> String {
    (time::OffsetDateTime::now_utc() - time::Duration::days(days_ago))
        .format(&time::format_description::well_known::Rfc3339)
        .expect("the threshold would not format")
}

fn ids_in(db: &Db) -> Vec<String> {
    store::list(db)
        .expect("the tasks would not read back")
        .into_iter()
        .map(|t| t.id)
        .collect()
}

#[test]
fn a_task_finished_long_ago_is_removed() {
    let db = db();
    finished_task_aged(&db, "old-completed", TaskState::Completed, 120);

    let removed =
        store::purge_finished_before(&db, &threshold(90)).expect("the purge itself failed");

    assert_eq!(removed, 1, "the old finished task was not counted as removed");
    assert!(
        !ids_in(&db).contains(&String::from("old-completed")),
        "the old finished task is still in the table after the purge"
    );
}

#[test]
fn a_task_finished_recently_is_kept() {
    // The line a retention policy must not cross by accident: a task from yesterday is not
    // "long ago" under any reasonable threshold, and purging it would look, to a person
    // checking whether last night's upload went through, exactly like data loss.
    let db = db();
    finished_task_aged(&db, "recent-completed", TaskState::Completed, 1);

    let removed =
        store::purge_finished_before(&db, &threshold(90)).expect("the purge itself failed");

    assert_eq!(removed, 0, "a recently finished task was purged");
    assert!(
        ids_in(&db).contains(&String::from("recent-completed")),
        "a recently finished task vanished from the table"
    );
}

#[test]
fn every_finished_state_is_eligible_but_nothing_else_is() {
    // The three states the function's own WHERE clause names, plus the two it must NOT
    // touch: a task that is still queued or running or paused has not finished at all, and
    // purging it would not be retention — it would be losing someone's actual work.
    let db = db();
    finished_task_aged(&db, "old-completed", TaskState::Completed, 120);
    finished_task_aged(&db, "old-failed", TaskState::Failed, 120);
    finished_task_aged(&db, "old-cancelled", TaskState::Cancelled, 120);
    finished_task_aged(&db, "old-queued", TaskState::Queued, 120);
    finished_task_aged(&db, "old-running", TaskState::Running, 120);
    finished_task_aged(&db, "old-paused", TaskState::Paused, 120);

    let removed =
        store::purge_finished_before(&db, &threshold(90)).expect("the purge itself failed");

    assert_eq!(
        removed, 3,
        "exactly the three finished states should have been purged"
    );
    let remaining = ids_in(&db);
    for still_here in ["old-queued", "old-running", "old-paused"] {
        assert!(
            remaining.contains(&String::from(still_here)),
            "a task that has not finished was purged: {still_here}"
        );
    }
    for gone in ["old-completed", "old-failed", "old-cancelled"] {
        assert!(
            !remaining.contains(&String::from(gone)),
            "a long-finished task survived the purge: {gone}"
        );
    }
}

#[test]
fn calling_it_twice_is_harmless() {
    // Constitution, principle V: repeating must be safe. A start-up call is not the only
    // call this will ever get — a person may open the application twice in a row, or the
    // call may run again after a crash mid-sweep.
    let db = db();
    finished_task_aged(&db, "old-completed", TaskState::Completed, 120);

    let first =
        store::purge_finished_before(&db, &threshold(90)).expect("the first purge failed");
    let second =
        store::purge_finished_before(&db, &threshold(90)).expect("the second purge failed");

    assert_eq!(first, 1);
    assert_eq!(second, 0, "the second purge found something the first one should have removed");
}

/// T564 — the purge has to actually run, not merely exist and pass its own tests.
///
/// A source check, in the same spirit as `registry.rs`'s
/// `the_account_is_installed_beside_the_sweep_that_reads_it`: raising a whole `AppState` in a
/// test to prove one line got called is a heavier and more roundabout way of asking the same
/// question a text search answers directly, and this project already has the precedent for
/// preferring the direct one.
#[test]
fn purge_finished_before_is_called_at_startup() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs");
    let text = std::fs::read_to_string(&path).expect("could not read lib.rs");
    assert!(
        text.contains("purge_finished_before"),
        "purge_finished_before is written and tested but nothing calls it (T564) — \
         the tasks table grows without bound"
    );
}
