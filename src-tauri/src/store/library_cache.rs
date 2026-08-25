//! T046 — the library cache: the last known state of a server.
//!
//! Stored as the finished answer of a command rather than spread across tables. It is
//! a snapshot for showing: nothing queries it, and spreading it out would mean keeping
//! two schemas of the same thing in agreement — and one day failing to.

use crate::commands::library::LibraryView;
use crate::store::db::{now_rfc3339, Db, DbError};
use rusqlite::OptionalExtension;

/// Remember the last library state that was read successfully.
pub fn save(db: &Db, server_id: &str, view: &LibraryView) -> Result<(), DbError> {
    let json = serde_json::to_string(view).unwrap_or_else(|_| String::from("{}"));
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO library_cache (server_id, view_json, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT (server_id) DO UPDATE SET
                view_json = excluded.view_json,
                updated_at = excluded.updated_at",
            rusqlite::params![server_id, json, now_rfc3339()],
        )?;
        Ok(())
    })
}

/// Read the last known state.
///
/// A corrupt cache is an absent cache, not an error: it makes showing faster and
/// decides nothing, and failing to open the library over it would be out of all
/// proportion.
pub fn load(db: &Db, server_id: &str) -> Result<Option<LibraryView>, DbError> {
    let json: Option<String> = db.with_conn(|c| {
        Ok(c.query_row(
            "SELECT view_json FROM library_cache WHERE server_id = ?1",
            [server_id],
            |r| r.get(0),
        )
        .optional()?)
    })?;

    Ok(json.and_then(|j| match serde_json::from_str(&j) {
        Ok(view) => Some(view),
        Err(e) => {
            tracing::warn!(server = server_id, error = %e, "library cache unreadable, reading from the server");
            None
        }
    }))
}

/// Forget a server's cache.
pub fn forget(db: &Db, server_id: &str) -> Result<(), DbError> {
    db.with_conn(|c| {
        c.execute(
            "DELETE FROM library_cache WHERE server_id = ?1",
            [server_id],
        )?;
        Ok(())
    })
}
