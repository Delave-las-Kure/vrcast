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
pub const SCHEMA_VERSION: u32 = 27;

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
    (13, include_str!("migrations/0013_donor_anchor.sql")),
    (14, include_str!("migrations/0014_material.sql")),
    (15, include_str!("migrations/0015_shape.sql")),
    (
        16,
        include_str!("migrations/0016_measured_the_broken_way.sql"),
    ),
    (17, include_str!("migrations/0017_check_pending.sql")),
    (18, include_str!("migrations/0018_task_owner.sql")),
    (19, include_str!("migrations/0019_task_result.sql")),
    (
        20,
        include_str!("migrations/0020_tasks_updated_at_index.sql"),
    ),
    (21, include_str!("migrations/0021_videos.sql")),
    (22, include_str!("migrations/0022_videos_seq.sql")),
    (23, include_str!("migrations/0023_video_lifecycle.sql")),
    (24, include_str!("migrations/0024_remote_runs.sql")),
    (25, include_str!("migrations/0025_video_replacing.sql")),
    (26, include_str!("migrations/0026_video_made_medium.sql")),
    (27, include_str!("migrations/0027_video_subtitles.sql")),
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
    /// ⚠ **T636 (QA-21 №3) — a change of a profile's way of signing in is one step, not three.**
    /// See [`Db::sign_in_lock`].
    sign_in: Mutex<()>,
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
            sign_in: Mutex::new(()),
        };
        db.migrate()?;
        Ok(db)
    }

    /// The default path: the current user's application data directory
    /// ([`super::data_dir::root`] — in the e2e build, the one it was given).
    pub fn default_path() -> Result<PathBuf> {
        let dir = super::data_dir::root().ok_or(DbError::NoDataDir)?;
        Ok(dir.join("vrcast-studio.sqlite"))
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

    /// Hold every change of a profile's way of signing in apart from every other (T636).
    ///
    /// Such a change is three things — read the profile, write the profile, write the secret in
    /// the operating system's store — and the store is not in this database, so no SQLite
    /// transaction can hold all three. Two writers exist: `server_update` (a person's form) and
    /// the deployment keeping the key it made (`commands::deploy::switch_to_managed_key`, T616).
    /// Interleaved, a form read as `password` could write `password` back over a profile the
    /// deployment had just moved to `managed_key`, or write its new password into the store
    /// after the deployment had put the key there — a profile and a store that disagree, with
    /// every step having succeeded.
    ///
    /// Held around the whole change by both writers. A lock of its own rather than the
    /// connection's: the store may take its time (a system keyring), and every other reader of
    /// the database must not wait for it. Order: this one first, then the connection — never
    /// the other way round. A poisoned lock is taken over: it guards no data of its own.
    pub fn sign_in_lock(&self) -> std::sync::MutexGuard<'_, ()> {
        self.sign_in
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// A timestamp in a form fit for storing and for comparing as strings.
///
/// **Always nine digits of the second's fraction.** The well-known RFC 3339 format drops the
/// fraction's trailing zeros, and a fraction of varying width does not compare as text the
/// way it does as time: `…:05.1234Z` is greater than `…:05.12345678Z` because `Z` is greater
/// than `5`. Everything that orders or compares these strings — the video list, «newer than
/// what is shown» on the screen, the latest timings — got the same second's events in either
/// order (the flaky `video::the_list_keeps_the_order_videos_were_added_in`). Fixed width is
/// still RFC 3339, and `parse_rfc3339` reads it.
pub fn now_rfc3339() -> String {
    rfc3339_fixed(time::OffsetDateTime::now_utc())
}

/// `at` as [`now_rfc3339`] writes it: UTC, nine digits of fraction.
pub fn rfc3339_fixed(at: time::OffsetDateTime) -> String {
    let t = at.to_offset(time::UtcOffset::UTC);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:09}Z",
        t.year(),
        u8::from(t.month()),
        t.day(),
        t.hour(),
        t.minute(),
        t.second(),
        t.nanosecond()
    )
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
