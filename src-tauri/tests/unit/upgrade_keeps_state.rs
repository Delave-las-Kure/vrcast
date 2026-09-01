//! T353 — a database left by an earlier build opens, migrates, and keeps what was in it.
//!
//! **Why an earlier build and not a fresh one.** Two of the migrations do not add a column,
//! they rebuild the table: 0008 for `tasks`, 0009 for `server_profiles`. Each makes a new
//! table, copies the rows across and drops the old one. A fresh database passes a rebuild no
//! matter what the copy does — there is nothing in it to lose. Only rows written *before* the
//! rebuild can tell whether the copy is real, and losing the server profiles on upgrade is
//! the one failure here a person cannot repair afterwards.
//!
//! **Why the migration files and not a schema written out here.** A schema spelled out in a
//! test is a second copy of the first one, and copies drift apart quietly. These are the very
//! files `store::db` applies.
//!
//! The other half of the promise — that the file is still there at all after an installer has
//! run over the old version — is not a database question, and is checked where it lives: the
//! third case of the uninstaller's truth table (`tests/uninstall-hook/`), where `$UpdateMode`
//! is 1 and the data must survive.

use rusqlite::Connection;
use std::path::{Path, PathBuf};
use vrcast_studio_lib::store::db::{now_rfc3339, Db, SCHEMA_VERSION};

/// The released migrations in order: `RELEASED[i]` takes the schema to version `i + 1`.
const RELEASED: &[&str] = &[
    include_str!("../../src/store/migrations/0001_initial.sql"),
    include_str!("../../src/store/migrations/0002_running_processes.sql"),
    include_str!("../../src/store/migrations/0003_library_cache.sql"),
    include_str!("../../src/store/migrations/0004_process_identity.sql"),
    include_str!("../../src/store/migrations/0005_queue_order.sql"),
    include_str!("../../src/store/migrations/0006_settings.sql"),
    include_str!("../../src/store/migrations/0007_quality_measurements.sql"),
    include_str!("../../src/store/migrations/0008_measure_quality_task.sql"),
    include_str!("../../src/store/migrations/0009_managed_key.sql"),
    include_str!("../../src/store/migrations/0010_process_owner.sql"),
    include_str!("../../src/store/migrations/0011_task_notices.sql"),
    include_str!("../../src/store/migrations/0012_task_batch.sql"),
    include_str!("../../src/store/migrations/0013_donor_anchor.sql"),
    include_str!("../../src/store/migrations/0014_material.sql"),
    include_str!("../../src/store/migrations/0015_shape.sql"),
];

/// A directory that removes itself, so a failing test does not leave databases behind.
struct Scratch(PathBuf);

impl Scratch {
    fn new(what: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "vrcast-upgrade-{what}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).expect("the scratch directory would not be made");
        Self(dir)
    }

    fn db_path(&self) -> PathBuf {
        self.0.join("vrcast-studio.sqlite")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Build a database file standing exactly at `version` — the way the build that released that
/// version would have left it, and no further.
fn database_at(path: &Path, version: u32) -> Connection {
    assert!(
        (1..=SCHEMA_VERSION).contains(&version),
        "there was never a version {version}"
    );
    let conn = Connection::open(path).expect("the database would not open");
    for sql in &RELEASED[..version as usize] {
        conn.execute_batch(sql)
            .expect("a migration would not apply");
    }
    conn.pragma_update(None, "user_version", version as i64)
        .expect("the version would not be written");
    conn
}

#[test]
fn the_migrations_named_here_are_all_of_them() {
    // Without this, adding migration 0010 leaves the seeding below silently short: the tests
    // would go on passing while checking a database nobody will ever have.
    assert_eq!(
        RELEASED.len() as u32,
        SCHEMA_VERSION,
        "the schema is at {SCHEMA_VERSION} but {} migrations are listed here — add the new one to this list too",
        RELEASED.len()
    );
}

#[test]
fn a_database_from_the_very_first_version_keeps_its_rows() {
    let scratch = Scratch::new("first");
    let path = scratch.db_path();

    {
        let old = database_at(&path, 1);
        // The columns are the ones version 1 had, not the ones there are now: this is meant to
        // be what an old build actually wrote.
        old.execute(
            "INSERT INTO server_profiles
             (id, name, host, port, username, auth_kind, secret_ref, key_path,
              domain, video_dir, is_active, created_at)
             VALUES ('p1', 'первый', 'a.example.test', 22, 'root', 'key', 'server/p1',
                     'C:\\ключи\\id_ed25519', 'v1.example.test', '/var/lib/vrcast/videos', 1, ?1)",
            [now_rfc3339()],
        )
        .expect("the first profile would not go in");
        old.execute(
            "INSERT INTO server_profiles
             (id, name, host, port, username, auth_kind, secret_ref,
              domain, video_dir, is_active, created_at)
             VALUES ('p2', 'второй', 'b.example.test', 2222, 'admin', 'password', 'server/p2',
                     'v2.example.test', '/srv/video', 0, ?1)",
            [now_rfc3339()],
        )
        .expect("the second profile would not go in");
        old.execute(
            "INSERT INTO tasks (id, kind, server_id, state, progress, resume_token,
                                created_at, updated_at)
             VALUES ('t1', 'upload', 'p1', 'paused', 0.42, '12345 sent', ?1, ?1)",
            [now_rfc3339()],
        )
        .expect("the paused task would not go in");
        old.execute(
            "INSERT INTO tasks (id, kind, server_id, state, created_at, updated_at)
             VALUES ('t2', 'convert', NULL, 'queued', ?1, ?1)",
            [now_rfc3339()],
        )
        .expect("the queued task would not go in");
        old.execute(
            "INSERT INTO host_fingerprints (host, port, fingerprint, first_seen)
             VALUES ('a.example.test', 22, 'SHA256:whatever', ?1)",
            [now_rfc3339()],
        )
        .expect("the fingerprint would not go in");
    }

    // The new build opens it. This is the moment the upgrade happens.
    let db = Db::open(&path).expect("the old database would not open");
    assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION);

    db.with_conn(|c| {
        // The profiles came through the rebuild of 0009 — names, addresses, and the path to
        // the key file, which the rebuilt CHECK still has to allow.
        let (name, host, port, key_path): (String, String, i64, Option<String>) = c.query_row(
            "SELECT name, host, port, key_path FROM server_profiles WHERE id = 'p1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
        assert_eq!(name, "первый");
        assert_eq!(host, "a.example.test");
        assert_eq!(port, 22);
        assert_eq!(key_path.as_deref(), Some("C:\\ключи\\id_ed25519"));

        let active: i64 = c.query_row(
            "SELECT count(*) FROM server_profiles WHERE is_active = 1",
            [],
            |r| r.get(0),
        )?;
        assert_eq!(active, 1, "the active server was not carried over");

        let profiles: i64 =
            c.query_row("SELECT count(*) FROM server_profiles", [], |r| r.get(0))?;
        assert_eq!(profiles, 2);

        // The queue came through the rebuild of 0008, resume position and all: a transfer that
        // forgets where it stopped starts again from nothing.
        let (state, progress, token): (String, f64, Option<String>) = c.query_row(
            "SELECT state, progress, resume_token FROM tasks WHERE id = 't1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        assert_eq!(state, "paused");
        assert!((progress - 0.42).abs() < 1e-9);
        assert_eq!(token.as_deref(), Some("12345 sent"));

        // 0005 gave the already-queued tasks an order by the time they appeared. Two tasks,
        // two different numbers — otherwise "move this one up" has nothing to move.
        let orders: i64 =
            c.query_row("SELECT count(DISTINCT queue_order) FROM tasks", [], |r| {
                r.get(0)
            })?;
        assert_eq!(
            orders, 2,
            "the queue order did not survive, or is the same for both"
        );

        let prints: i64 =
            c.query_row("SELECT count(*) FROM host_fingerprints", [], |r| r.get(0))?;
        assert_eq!(prints, 1);
        Ok(())
    })
    .expect("reading the upgraded database failed");
}

#[test]
fn referential_integrity_is_back_on_after_an_upgrade() {
    // Migrating switches foreign keys off on purpose — see `store::db::migrate`. If it ever
    // stopped switching them back on, every test above would still pass, the application
    // would still start, and the first sign of it would be a queue holding tasks for a
    // server that had been deleted months earlier.
    //
    // So the pragma is asked *and* the rule is used: an answer of 1 from a connection that
    // does not act on it would be the same lie in a friendlier voice.
    let scratch = Scratch::new("integrity");
    let path = scratch.db_path();

    {
        let old = database_at(&path, 1);
        old.execute(
            "INSERT INTO server_profiles
             (id, name, host, username, auth_kind, secret_ref, domain, video_dir, created_at)
             VALUES ('p1', 'сервер', 'a.example.test', 'root', 'password', 'server/p1',
                     'v.example.test', '/var/lib/vrcast/videos', ?1)",
            [now_rfc3339()],
        )
        .expect("the profile would not go in");
        old.execute(
            "INSERT INTO tasks (id, kind, server_id, state, created_at, updated_at)
             VALUES ('t1', 'upload', 'p1', 'queued', ?1, ?1)",
            [now_rfc3339()],
        )
        .expect("the task would not go in");
    }

    let db = Db::open(&path).expect("the old database would not open");

    let answer: i64 = db
        .with_conn(|c| Ok(c.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?))
        .expect("the pragma would not read");
    assert_eq!(answer, 1, "referential integrity was left switched off");

    db.with_conn(|c| {
        c.execute("DELETE FROM server_profiles WHERE id = 'p1'", [])?;
        Ok(())
    })
    .expect("the profile would not delete");

    let orphans: i64 = db
        .with_conn(|c| Ok(c.query_row("SELECT count(*) FROM tasks", [], |r| r.get(0))?))
        .expect("counting the tasks failed");
    assert_eq!(
        orphans, 0,
        "the task outlived its server: the cascade is not being enforced"
    );
}

#[test]
fn settings_written_by_an_earlier_version_survive() {
    // Settings only exist from schema 6 onwards, so that is where they are put: seeding them
    // earlier would be seeding a table that build could not have had.
    let scratch = Scratch::new("settings");
    let path = scratch.db_path();

    {
        let old = database_at(&path, 6);
        old.execute(
            "INSERT INTO settings (name, value) VALUES ('theme', 'dark'), ('mascot', 'false')",
            [],
        )
        .expect("the settings would not go in");
    }

    let db = Db::open(&path).expect("the old database would not open");
    let theme: String = db
        .with_conn(|c| {
            Ok(
                c.query_row("SELECT value FROM settings WHERE name = 'theme'", [], |r| {
                    r.get(0)
                })?,
            )
        })
        .expect("the setting was lost on upgrade");
    assert_eq!(theme, "dark");
}

#[test]
fn no_column_of_the_schema_is_meant_to_hold_a_secret() {
    // Principle IV: the credential store is the operating system's, and this file is an
    // ordinary one in a person's profile. The rule is worth checking rather than remembering,
    // because the way it breaks is somebody adding one convenient column in a migration.
    //
    // "token" is deliberately not on the list: `resume_token` is a position in a transfer, and
    // a check that fires on it would be switched off within the week.
    const MEANT_FOR_SECRETS: &[&str] = &[
        "password",
        "passphrase",
        "private_key",
        "privkey",
        "credential",
        "secret",
    ];

    let db = Db::open_in_memory().expect("the database would not open");
    db.with_conn(|c| {
        let mut tables = c.prepare("SELECT name FROM sqlite_master WHERE type = 'table'")?;
        let names: Vec<String> = tables
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        assert!(
            !names.is_empty(),
            "no tables at all — the check would be passing on nothing"
        );

        for table in names {
            let mut cols = c.prepare(&format!("PRAGMA table_info(\"{table}\")"))?;
            let columns: Vec<String> = cols
                .query_map([], |r| r.get(1))?
                .collect::<rusqlite::Result<_>>()?;
            for column in columns {
                // `secret_ref` is the exception, and it is one on purpose: it holds the name of
                // an entry in the operating system's store, never the entry's contents.
                if column == "secret_ref" {
                    continue;
                }
                let lowered = column.to_lowercase();
                for word in MEANT_FOR_SECRETS {
                    assert!(
                        !lowered.contains(word),
                        "{table}.{column} reads like a place to keep a secret; secrets belong in the operating system's store (principle IV)"
                    );
                }
            }
        }
        Ok(())
    })
    .expect("reading the schema failed");
}
