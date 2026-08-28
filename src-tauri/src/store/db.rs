//! T008 — the local store: the connection, the schema, the migrations.
//!
//! What survives a restart of the application lives here: server profiles **without their
//! secrets** (the secrets themselves are in the operating system store, see
//! `super::secrets`), the task journal with its resume position, and the library cache.
//! Requirements: FR-081 (tasks survive a restart), FR-085 (repeating is safe),
//! constitution, principle V.

use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// The schema version this build of the application understands.
pub const SCHEMA_VERSION: u32 = 12;

/// Migrations are applied in order; the number is the `user_version` after applying it.
/// A migration already released must never be changed — only followed by the next one.
const MIGRATIONS: &[(u32, &str)] = &[
    (1, include_str!("migrations/0001_initial.sql")),
    (2, include_str!("migrations/0002_running_processes.sql")),
    (3, include_str!("migrations/0003_library_cache.sql")),
    (4, include_str!("migrations/0004_process_identity.sql")),
    (5, include_str!("migrations/0005_queue_order.sql")),
    (6, include_str!("migrations/0006_settings.sql")),
    (7, include_str!("migrations/0007_quality_measurements.sql")),
    (8, include_str!("migrations/0008_measure_quality_task.sql")),
    (9, include_str!("migrations/0009_managed_key.sql")),
    (10, include_str!("migrations/0010_process_owner.sql")),
    (11, include_str!("migrations/0011_task_notices.sql")),
    (12, include_str!("migrations/0012_task_batch.sql")),
];

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("could not open the local database: {0}")]
    Open(#[source] rusqlite::Error),

    #[error("could not apply migration {version}: {source}")]
    Migration {
        version: u32,
        #[source]
        source: rusqlite::Error,
    },

    /// The same principle as with the server-side version (FR-130): meeting state newer
    /// than we understand, we refuse to work rather than quietly damaging it.
    #[error("the local database was made by a newer version of the application (schema {found}, this build knows up to {known})")]
    TooNew { found: u32, known: u32 },

    /// A migration left a reference pointing at a row that is not there. Said out loud
    /// rather than passed over: the shape it takes otherwise is a queue full of tasks for a
    /// server that no longer exists, and nothing anywhere to say why.
    #[error("after migrating, {table} refers to a row that is not in {parent}")]
    BrokenReferences { table: String, parent: String },

    #[error("could not determine the application data directory")]
    NoDataDir,

    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
}

pub type Result<T> = std::result::Result<T, DbError>;

/// The local database.
///
/// A rusqlite `Connection` is not shared between threads, so access goes through a mutex.
/// The critical sections are short: long work — a transfer, an encode — does not hold the
/// database, it only writes marks when the state moves.
pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    /// Open the database at a path, creating the directory if needed, and migrate it.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| {
                DbError::Open(rusqlite::Error::InvalidPath(
                    format!("{}: {e}", dir.display()).into(),
                ))
            })?;
        }
        let conn = Connection::open(path).map_err(DbError::Open)?;
        Self::from_conn(conn)
    }

    /// A database in memory, for tests. It leaves no trace on disk.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(DbError::Open)?;
        Self::from_conn(conn)
    }

    fn from_conn(conn: Connection) -> Result<Self> {
        // Referential integrity is off by default in SQLite — turned on explicitly, or
        // deleting a profile leaves orphaned tasks behind.
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        // Write-ahead logging: a read is not blocked by a write. An in-memory database
        // does not support it and quietly stays as it was — which is fine.
        let _: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
        conn.busy_timeout(Duration::from_secs(5))?;

        let db = Self {
            conn: Mutex::new(conn),
        };
        db.migrate()?;
        Ok(db)
    }

    /// The default path: the current user's application data directory.
    pub fn default_path() -> Result<PathBuf> {
        let dirs = directories::ProjectDirs::from("ru", "VRCast", "VRCast Studio")
            .ok_or(DbError::NoDataDir)?;
        Ok(dirs.data_dir().join("vrcast-studio.sqlite"))
    }

    /// Apply the missing migrations. Safe to repeat: applied ones are skipped.
    ///
    /// **Referential integrity is off while this runs, and that is not a shortcut.** SQLite
    /// cannot alter a constraint, so a migration that changes one builds the table anew,
    /// copies the rows across and drops the old one — 0008 and 0009 both do exactly that.
    /// With foreign keys on, `DROP TABLE` performs an implicit `DELETE FROM` first, and that
    /// fires `ON DELETE CASCADE` on every child row: dropping `server_profiles` takes the
    /// whole task queue with it. Measured 2026-08-28 on a database seeded at schema 1 — the
    /// profiles arrived, the queue did not. Turning them off around a rebuild is the
    /// procedure SQLite's own documentation gives.
    ///
    /// The pragma cannot live inside the migration file: it is a no-op within a transaction,
    /// and every migration runs in one. So it sits here, around them — and what it suspends
    /// is put back afterwards and then **checked**, because "off for a moment" is one edit
    /// away from "off from now on", and nothing would notice.
    fn migrate(&self) -> Result<()> {
        let mut guard = self.conn.lock().expect("the database mutex is poisoned");
        let conn = &mut *guard;

        let current: u32 =
            conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? as u32;

        if current > SCHEMA_VERSION {
            return Err(DbError::TooNew {
                found: current,
                known: SCHEMA_VERSION,
            });
        }

        // Nothing to apply is the ordinary case — every start after the first. Leave the
        // connection exactly as it was found rather than switching integrity off and on
        // again for no reason, and skip the check below: it has nothing to check.
        if current == SCHEMA_VERSION {
            return Ok(());
        }

        conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
        let applied = Self::apply_from(conn, current);
        // Back on whatever happened on the way. A failed migration leaving the connection
        // without referential integrity would be the worse of the two faults, and the quiet
        // one.
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        applied?;

        let mut check = conn.prepare("PRAGMA foreign_key_check")?;
        let mut rows = check.query([])?;
        if let Some(row) = rows.next()? {
            return Err(DbError::BrokenReferences {
                table: row.get(0)?,
                parent: row.get(2)?,
            });
        }
        Ok(())
    }

    /// The migrations themselves, from `current` onwards. Split out so that whatever they
    /// do, the caller gets to put referential integrity back.
    fn apply_from(conn: &mut Connection, current: u32) -> Result<()> {
        for (version, sql) in MIGRATIONS {
            if *version <= current {
                continue;
            }
            // The migration and the mark of it go in one transaction: an interruption
            // between them would leave the database in a state that does not match the
            // version written down.
            let tx = conn.transaction()?;
            tx.execute_batch(sql).map_err(|source| DbError::Migration {
                version: *version,
                source,
            })?;
            tx.pragma_update(None, "user_version", *version as i64)?;
            tx.commit()?;
            tracing::info!(version = *version, "database migration applied");
        }
        Ok(())
    }

    /// The current schema version.
    pub fn schema_version(&self) -> Result<u32> {
        let conn = self.conn.lock().expect("the database mutex is poisoned");
        Ok(conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? as u32)
    }

    /// Do some work with the connection. Keep the closure short.
    pub fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let conn = self.conn.lock().expect("the database mutex is poisoned");
        f(&conn)
    }

    /// The same, but with mutable access — needed for transactions.
    pub fn with_conn_mut<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        let mut conn = self.conn.lock().expect("the database mutex is poisoned");
        f(&mut conn)
    }
}

/// A timestamp in a form fit for storing and for comparing as strings.
pub fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| String::from("1970-01-01T00:00:00Z"))
}

/// Parse a timestamp back into seconds since the epoch.
///
/// Needed wherever spans are counted: how long a task ran, how long ago the catalogue was
/// refreshed. It returns an error rather than zero: zero here would mean the year 1970 and
/// would give half-century spans where in truth the timestamp simply would not parse.
pub fn parse_rfc3339(s: &str) -> std::result::Result<u64, time::error::Parse> {
    let t = time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339)?;
    Ok(t.unix_timestamp().max(0) as u64)
}
