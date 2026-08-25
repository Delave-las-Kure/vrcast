//! Tests for the local store (T008).
//!
//! What is checked is not "SQLite works" but the properties of the schema the requirements
//! lean on: migrations being safe to repeat (principle V), the refusal to work with a newer
//! database, and the rule "exactly one server is active" (FR-002), moved into the database
//! itself.

use vrcast_studio_lib::store::db::{now_rfc3339, Db, DbError, SCHEMA_VERSION};

fn insert_profile(db: &Db, id: &str, name: &str, active: bool) -> rusqlite::Result<usize> {
    db.with_conn(|c| {
        Ok(c.execute(
            "INSERT INTO server_profiles
             (id, name, host, username, auth_kind, secret_ref, domain, video_dir, is_active, created_at)
             VALUES (?1, ?2, 'example.test', 'root', 'key', 'server/x', 'd.example.test', '/var/lib/vrcast/videos', ?3, ?4)",
            rusqlite::params![id, name, active as i32, now_rfc3339()],
        )?)
    })
    .map_err(|e| match e {
        DbError::Sql(e) => e,
        other => panic!("unexpected error: {other}"),
    })
}

#[test]
fn migrations_apply_and_give_a_known_version() {
    let db = Db::open_in_memory().expect("the database would not open");
    assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION);

    // The tables are there.
    for table in ["server_profiles", "tasks", "host_fingerprints"] {
        let found: i64 = db
            .with_conn(|c| {
                Ok(c.query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |r| r.get(0),
                )?)
            })
            .unwrap();
        assert_eq!(found, 1, "there is no table {table}");
    }
}

#[test]
fn opening_the_database_again_does_not_break_it() {
    // Constitution, principle V: repeating must be safe.
    let dir = std::env::temp_dir().join(format!("vrcast-test-{}", std::process::id()));
    let path = dir.join("repeat.sqlite");
    let _ = std::fs::remove_file(&path);

    {
        let db = Db::open(&path).unwrap();
        insert_profile(&db, "srv1", "First", true).unwrap();
    }
    {
        let db = Db::open(&path).unwrap();
        assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION);
        let count: i64 = db
            .with_conn(|c| {
                Ok(c.query_row("SELECT count(*) FROM server_profiles", [], |r| r.get(0))?)
            })
            .unwrap();
        assert_eq!(count, 1, "the data was lost on reopening");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_database_newer_than_we_know_is_refused() {
    // The same principle as with the server-side version (FR-130): meeting state newer
    // than we understand, we refuse to work rather than quietly damaging it.
    let dir = std::env::temp_dir().join(format!("vrcast-test-new-{}", std::process::id()));
    let path = dir.join("future.sqlite");
    let _ = std::fs::remove_file(&path);

    {
        let db = Db::open(&path).unwrap();
        db.with_conn(|c| {
            c.execute_batch(&format!("PRAGMA user_version = {};", SCHEMA_VERSION + 5))?;
            Ok(())
        })
        .unwrap();
    }

    match Db::open(&path) {
        Err(DbError::TooNew { found, known }) => {
            assert_eq!(found, SCHEMA_VERSION + 5);
            assert_eq!(known, SCHEMA_VERSION);
        }
        Ok(_) => panic!("a database newer than we know was opened — that is silent damage"),
        Err(other) => panic!("the wrong error: {other}"),
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn only_one_server_can_be_active() {
    // FR-002 is upheld by a rule in the database rather than by careful code: even a bug
    // in the code cannot make two of them active.
    let db = Db::open_in_memory().unwrap();
    insert_profile(&db, "srv1", "First", true).unwrap();

    let err = insert_profile(&db, "srv2", "Second", true)
        .expect_err("the database allowed two active servers");
    assert!(
        err.to_string().contains("UNIQUE"),
        "a uniqueness violation was expected, got: {err}"
    );

    // Inactive ones may be as many as you like.
    insert_profile(&db, "srv3", "Third", false).unwrap();
    insert_profile(&db, "srv4", "Fourth", false).unwrap();
}

#[test]
fn deleting_a_profile_takes_its_tasks_with_it() {
    // Referential integrity is turned on explicitly: without it, deleting a profile leaves
    // orphaned tasks behind that the application will try to carry on.
    let db = Db::open_in_memory().unwrap();
    insert_profile(&db, "srv1", "First", true).unwrap();

    db.with_conn(|c| {
        c.execute(
            "INSERT INTO tasks (id, kind, server_id, state, created_at, updated_at)
             VALUES ('t1', 'upload', 'srv1', 'queued', ?1, ?1)",
            [now_rfc3339()],
        )?;
        Ok(())
    })
    .unwrap();

    db.with_conn(|c| {
        c.execute("DELETE FROM server_profiles WHERE id = 'srv1'", [])?;
        Ok(())
    })
    .unwrap();

    let left: i64 = db
        .with_conn(|c| Ok(c.query_row("SELECT count(*) FROM tasks", [], |r| r.get(0))?))
        .unwrap();
    assert_eq!(left, 0, "orphaned tasks were left behind");
}

#[test]
fn a_task_cannot_hold_a_state_that_does_not_exist() {
    // A task's state transitions are described in data-model.md section 7. The set of
    // values is pinned in the schema so a typo cannot create a task in a state that does
    // not exist.
    let db = Db::open_in_memory().unwrap();
    let err = db
        .with_conn(|c| {
            Ok(c.execute(
                "INSERT INTO tasks (id, kind, state, created_at, updated_at)
                 VALUES ('t1', 'upload', 'running-ish', ?1, ?1)",
                [now_rfc3339()],
            )?)
        })
        .expect_err("the database accepted a task state that does not exist");
    assert!(err.to_string().contains("CHECK"), "got: {err}");
}
