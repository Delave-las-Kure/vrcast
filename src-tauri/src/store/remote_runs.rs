//! T682 — work started on a server apart from the application, written down until its end is
//! confirmed (migration 0024).
//!
//! Only reading and writing rows. What is started is `server::hls_package`; what is done with
//! a row after a restart is `commands::video` (a video that was being stopped).

use serde::{Deserialize, Serialize};

use crate::store::db::{now_rfc3339, Db, DbError};

/// One start of work on a server, by the mark every process of it carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteRun {
    /// The environment variable the mark is in (`VRCAST_HLS_JOB`).
    pub var: String,
    /// The mark itself — one per start, never shared with another copy of the application.
    pub mark: String,
    /// The server it runs on and the account it was started as. A stop goes there and
    /// nowhere else.
    pub host: String,
    pub port: u16,
    pub user: String,
}

/// Write down that this task has started `run` (or replace what it had).
pub fn save(db: &Db, task_id: &str, run: &RemoteRun) -> Result<(), DbError> {
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO remote_runs (task_id, var, mark, host, port, user, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (task_id) DO UPDATE SET
                var = excluded.var, mark = excluded.mark, host = excluded.host,
                port = excluded.port, user = excluded.user, created_at = excluded.created_at",
            rusqlite::params![
                task_id,
                run.var,
                run.mark,
                run.host,
                run.port as i64,
                run.user,
                now_rfc3339()
            ],
        )?;
        Ok(())
    })
}

/// What this task started and has not seen end, if anything.
pub fn get(db: &Db, task_id: &str) -> Result<Option<RemoteRun>, DbError> {
    db.with_conn(|c| {
        let mut stmt =
            c.prepare("SELECT var, mark, host, port, user FROM remote_runs WHERE task_id = ?1")?;
        let mut rows = stmt.query([task_id])?;
        Ok(match rows.next()? {
            Some(r) => Some(RemoteRun {
                var: r.get(0)?,
                mark: r.get(1)?,
                host: r.get(2)?,
                port: r.get::<_, i64>(3)?.clamp(0, u16::MAX as i64) as u16,
                user: r.get(4)?,
            }),
            None => None,
        })
    })
}

/// Strike it out: the work ended and its end is confirmed. Repeating is safe.
pub fn clear(db: &Db, task_id: &str) -> Result<(), DbError> {
    db.with_conn(|c| {
        c.execute("DELETE FROM remote_runs WHERE task_id = ?1", [task_id])?;
        Ok(())
    })
}
