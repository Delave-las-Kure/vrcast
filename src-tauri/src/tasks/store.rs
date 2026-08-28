//! T017 — storing tasks: they survive a restart of the application (FR-081).
//!
//! Not every movement is written to the database, only the ones that matter: a change
//! of state, the resume position, an error. Progress, which changes hundreds of times
//! a second, does not go to the database — otherwise the disk would become the
//! bottleneck of a task that ought to be limited by the network.

use super::state::{TaskKind, TaskState};
use crate::domain::wording::{Detail, DetailCode};
use crate::error::AppError;
use crate::store::db::{now_rfc3339, Db, DbError};
use serde::{Deserialize, Serialize};

/// A task in the form it is stored and shown in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRecord {
    pub id: String,
    pub kind: TaskKind,
    pub server_id: Option<String>,
    pub state: TaskState,
    pub progress: f64,
    /// Which stage it is at. A code, not a phrase: the wording belongs to the
    /// interface, and a task outlives the language it was started in.
    pub stage: Option<DetailCode>,
    pub speed_bps: Option<i64>,
    pub eta_s: Option<i64>,
    /// The resume position: bytes sent, rungs finished, steps completed.
    pub resume_token: Option<String>,
    /// Why it failed, if it did. Stored as an object rather than a sentence: a task
    /// finished a week ago must still explain itself in whatever language is chosen
    /// today. Secrets are already redacted.
    pub error: Option<AppError>,
    /// What the task had to say that is not a failure (T415).
    ///
    /// Variants taken from a previous run, a measurement that stopped short, the graphics
    /// card that refused. Codes and their numbers, like `error` and for the same reason: a
    /// task that finished a week ago still explains itself in whatever language is chosen
    /// today.
    pub notices: Vec<Detail>,
    /// Place in the queue: lower runs sooner.
    ///
    /// Kept apart from the creation time, because reordering (FR-083) has to change
    /// the order without falsifying when a task appeared.
    pub queue_order: i64,
    pub created_at: String,
    pub updated_at: String,
}

impl TaskRecord {
    pub fn new(id: impl Into<String>, kind: TaskKind, server_id: Option<String>) -> Self {
        let now = now_rfc3339();
        Self {
            id: id.into(),
            kind,
            server_id,
            state: TaskState::Queued,
            progress: 0.0,
            stage: None,
            speed_bps: None,
            eta_s: None,
            resume_token: None,
            error: None,
            notices: Vec::new(),
            queue_order: 0,
            created_at: now.clone(),
            updated_at: now,
        }
    }
}

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskRecord> {
    let kind: String = row.get("kind")?;
    let state: String = row.get("state")?;
    Ok(TaskRecord {
        id: row.get("id")?,
        kind: TaskKind::parse(&kind).unwrap_or(TaskKind::Probe),
        server_id: row.get("server_id")?,
        state: TaskState::parse(&state).unwrap_or(TaskState::Failed),
        progress: row.get("progress")?,
        // An unknown code means a task stored by a newer version of the application.
        // Showing nothing beats showing a key nobody can read.
        stage: row
            .get::<_, Option<String>>("stage")?
            .and_then(|s| DetailCode::parse(&s)),
        speed_bps: row.get("speed_bps")?,
        eta_s: row.get("eta_s")?,
        resume_token: row.get("resume_token")?,
        error: row
            .get::<_, Option<String>>("error")?
            .and_then(|s| serde_json::from_str(&s).ok()),
        // Unreadable notices are dropped rather than fatal: a task's own outcome must not be
        // lost because an aside about it was written by a newer version.
        notices: row
            .get::<_, Option<String>>("notices")?
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default(),
        queue_order: row.get("queue_order")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

/// Write a task: create or update.
pub fn upsert(db: &Db, task: &TaskRecord) -> Result<(), DbError> {
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO tasks
                (id, kind, server_id, state, progress, stage, speed_bps, eta_s,
                 resume_token, error, notices, queue_order, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
             ON CONFLICT (id) DO UPDATE SET
                state = excluded.state,
                progress = excluded.progress,
                stage = excluded.stage,
                speed_bps = excluded.speed_bps,
                eta_s = excluded.eta_s,
                resume_token = excluded.resume_token,
                error = excluded.error,
                notices = excluded.notices,
                updated_at = excluded.updated_at",
            rusqlite::params![
                task.id,
                task.kind.as_str(),
                task.server_id,
                task.state.as_str(),
                task.progress,
                task.stage.map(|s| s.as_str()),
                task.speed_bps,
                task.eta_s,
                task.resume_token,
                task.error
                    .as_ref()
                    .and_then(|e| serde_json::to_string(e).ok()),
                notices_json(&task.notices),
                task.queue_order,
                task.created_at,
                task.updated_at,
            ],
        )?;
        Ok(())
    })
}

/// Write only the resume position, leaving the rest alone.
///
/// A pointed update rather than read-modify-write of the whole record: the token is
/// written by the running task, while the state is written by a pause or a cancel from
/// another thread, and a full rewrite from either side would overwrite the other's
/// fresh field.
pub fn save_resume_token(db: &Db, id: &str, token: &str) -> Result<(), DbError> {
    db.with_conn(|c| {
        c.execute(
            "UPDATE tasks SET resume_token = ?2, updated_at = ?3 WHERE id = ?1",
            rusqlite::params![id, token, now_rfc3339()],
        )?;
        Ok(())
    })
}

/// Write only the progress, leaving the rest alone.
///
/// Needed for restarts: progress spreads through the interface as events, but it lives
/// only in memory. Without this write, a task interrupted halfway through a
/// thirty-gigabyte upload shows zero after the application starts — and a person has
/// nothing to decide by whether to resume it or drop it.
///
/// Records of finished tasks are left alone: a progress message can arrive late and
/// knock a completed task's 1.0 back down to 0.98.
pub fn save_progress(db: &Db, id: &str, progress: f64) -> Result<(), DbError> {
    db.with_conn(|c| {
        c.execute(
            "UPDATE tasks SET progress = ?2, updated_at = ?3
             WHERE id = ?1 AND state NOT IN ('completed', 'failed', 'cancelled')",
            rusqlite::params![id, progress.clamp(0.0, 1.0), now_rfc3339()],
        )?;
        Ok(())
    })
}

/// Write only the stage, leaving the rest alone.
///
/// Called on a change of stage and on nothing else — see `TaskContext::note_stage`. Pointed
/// for the same reason as [`save_resume_token`]: the stage comes from the running task while
/// the state comes from a pause or a cancel on another thread.
///
/// Finished records are left alone. A stage message can arrive a moment late and put "cutting
/// segments" on a task that is already done, which reads as a task that stopped halfway.
pub fn save_stage(db: &Db, id: &str, stage: DetailCode) -> Result<(), DbError> {
    db.with_conn(|c| {
        c.execute(
            "UPDATE tasks SET stage = ?2, updated_at = ?3
             WHERE id = ?1 AND state NOT IN ('completed', 'failed', 'cancelled')",
            rusqlite::params![id, stage.as_str(), now_rfc3339()],
        )?;
        Ok(())
    })
}

/// The notices as they are stored, or nothing at all when there are none.
///
/// `None` rather than `"[]"`: an empty list and never having said anything are the same
/// thing, and writing two spellings of it means reading two one day.
fn notices_json(notices: &[Detail]) -> Option<String> {
    if notices.is_empty() {
        return None;
    }
    serde_json::to_string(notices).ok()
}

/// Write only what the task had to say, leaving the rest alone.
///
/// Written as the task ends, and unlike [`save_progress`] **not** withheld from finished
/// records: this is the one write that has to land on a task that is already over, because
/// that is when there is anything to write.
pub fn save_notices(db: &Db, id: &str, notices: &[Detail]) -> Result<(), DbError> {
    let Some(json) = notices_json(notices) else {
        return Ok(());
    };
    db.with_conn(|c| {
        c.execute(
            "UPDATE tasks SET notices = ?2, updated_at = ?3 WHERE id = ?1",
            rusqlite::params![id, json, now_rfc3339()],
        )?;
        Ok(())
    })
}

/// Write only the state and, if there is one, the error — leaving the rest alone.
///
/// Returns `false` when there is no record of the task. The reason for being pointed
/// is the same as in [`save_resume_token`].
pub fn save_state(
    db: &Db,
    id: &str,
    state: TaskState,
    error: Option<&AppError>,
) -> Result<bool, DbError> {
    db.with_conn(|c| {
        let changed = c.execute(
            "UPDATE tasks SET
                state = ?2,
                progress = CASE WHEN ?2 = 'completed' THEN 1.0 ELSE progress END,
                error = COALESCE(?3, error),
                updated_at = ?4
             WHERE id = ?1",
            rusqlite::params![
                id,
                state.as_str(),
                error.and_then(|e| serde_json::to_string(e).ok()),
                now_rfc3339()
            ],
        )?;
        Ok(changed > 0)
    })
}

/// Read one task.
pub fn get(db: &Db, id: &str) -> Result<Option<TaskRecord>, DbError> {
    db.with_conn(|c| {
        let mut stmt = c.prepare("SELECT * FROM tasks WHERE id = ?1")?;
        let mut rows = stmt.query([id])?;
        Ok(match rows.next()? {
            Some(row) => Some(row_to_record(row)?),
            None => None,
        })
    })
}

/// The highest place taken in the queue.
///
/// Needed at start-up: the next task has to go **after** everything already in the
/// database, or it quietly cuts into the middle of the previous run's queue.
pub fn max_queue_order(db: &Db) -> Result<i64, DbError> {
    db.with_conn(|c| {
        Ok(
            c.query_row("SELECT COALESCE(MAX(queue_order), 0) FROM tasks", [], |r| {
                r.get(0)
            })?,
        )
    })
}

/// Write only the place in the queue.
pub fn save_queue_order(db: &Db, id: &str, order: i64) -> Result<(), DbError> {
    db.with_conn(|c| {
        c.execute(
            "UPDATE tasks SET queue_order = ?2, updated_at = ?3 WHERE id = ?1",
            rusqlite::params![id, order, now_rfc3339()],
        )?;
        Ok(())
    })
}

/// Every task, newest first.
pub fn list(db: &Db) -> Result<Vec<TaskRecord>, DbError> {
    db.with_conn(|c| {
        let mut stmt = c.prepare("SELECT * FROM tasks ORDER BY created_at DESC")?;
        let rows = stmt.query_map([], row_to_record)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    })
}

/// What restoring after start-up found.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RecoveryReport {
    /// Tasks caught running by the previous run. Moved to paused.
    pub interrupted: Vec<String>,
}

/// Sort out the state after the application starts.
///
/// Tasks left in the running state belong to the previous run: their processes are
/// gone. They are moved to **paused** — and never to completed (constitution,
/// principle III; SC-010). The difference is not cosmetic: "completed" would mean the
/// result is ready, and it was cut off halfway.
pub fn recover_after_start(db: &Db) -> Result<RecoveryReport, DbError> {
    let interrupted: Vec<String> = db.with_conn(|c| {
        let mut stmt = c.prepare("SELECT id FROM tasks WHERE state = 'running'")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    })?;

    if interrupted.is_empty() {
        return Ok(RecoveryReport::default());
    }

    db.with_conn(|c| {
        c.execute(
            "UPDATE tasks SET state = 'paused', updated_at = ?1 WHERE state = 'running'",
            [now_rfc3339()],
        )?;
        Ok(())
    })?;

    tracing::warn!(
        count = interrupted.len(),
        "tasks from the previous run were moved to paused"
    );

    Ok(RecoveryReport { interrupted })
}

/// Remove tasks that finished long ago.
pub fn purge_finished_before(db: &Db, before_rfc3339: &str) -> Result<usize, DbError> {
    db.with_conn(|c| {
        Ok(c.execute(
            "DELETE FROM tasks
             WHERE state IN ('completed', 'failed', 'cancelled') AND updated_at < ?1",
            [before_rfc3339],
        )?)
    })
}
