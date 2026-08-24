//! Тесты локального хранилища (T008).
//!
//! Проверяется не «SQLite работает», а те свойства схемы, на которые опираются требования:
//! безопасность миграций при повторе (принцип V), отказ работать с более новой базой
//! и правило «активен ровно один сервер» (FR-002), вынесенное в саму базу.

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
        other => panic!("неожиданная ошибка: {other}"),
    })
}

#[test]
fn миграции_применяются_и_дают_известную_версию() {
    let db = Db::open_in_memory().expect("база не открылась");
    assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION);

    // Таблицы на месте.
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
        assert_eq!(found, 1, "нет таблицы {table}");
    }
}

#[test]
fn повторное_открытие_базы_не_ломает_её() {
    // Конституция, принцип V: повтор обязан быть безопасным.
    let dir = std::env::temp_dir().join(format!("vrcast-test-{}", std::process::id()));
    let path = dir.join("repeat.sqlite");
    let _ = std::fs::remove_file(&path);

    {
        let db = Db::open(&path).unwrap();
        insert_profile(&db, "srv1", "Первый", true).unwrap();
    }
    {
        let db = Db::open(&path).unwrap();
        assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION);
        let count: i64 = db
            .with_conn(|c| {
                Ok(c.query_row("SELECT count(*) FROM server_profiles", [], |r| r.get(0))?)
            })
            .unwrap();
        assert_eq!(count, 1, "данные потерялись при повторном открытии");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn база_новее_известной_отвергается() {
    // Тот же принцип, что и с версией серверной части (FR-130): встретив состояние
    // новее, чем понимаем, отказываемся работать, а не портим его молча.
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
        Ok(_) => panic!("база новее известной была открыта — это молчаливая порча состояния"),
        Err(other) => panic!("не та ошибка: {other}"),
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn активным_может_быть_только_один_сервер() {
    // FR-002 обеспечен правилом базы, а не аккуратностью кода: даже ошибка в коде
    // не сможет сделать активными двоих.
    let db = Db::open_in_memory().unwrap();
    insert_profile(&db, "srv1", "Первый", true).unwrap();

    let err = insert_profile(&db, "srv2", "Второй", true)
        .expect_err("база позволила сделать активными два сервера");
    assert!(
        err.to_string().contains("UNIQUE"),
        "ожидалось нарушение уникальности, получено: {err}"
    );

    // Неактивных сколько угодно.
    insert_profile(&db, "srv3", "Третий", false).unwrap();
    insert_profile(&db, "srv4", "Четвёртый", false).unwrap();
}

#[test]
fn удаление_профиля_уносит_его_задачи() {
    // Ссылочная целостность включается явно: без неё удаление профиля оставит
    // осиротевшие задачи, которые приложение будет пытаться возобновить.
    let db = Db::open_in_memory().unwrap();
    insert_profile(&db, "srv1", "Первый", true).unwrap();

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
    assert_eq!(left, 0, "остались осиротевшие задачи");
}

#[test]
fn задача_не_может_иметь_недопустимое_состояние() {
    // Переходы состояний задачи описаны в data-model.md §7. Набор значений закреплён
    // в схеме, чтобы опечатка не создала задачу в состоянии, которого не существует.
    let db = Db::open_in_memory().unwrap();
    let err = db
        .with_conn(|c| {
            Ok(c.execute(
                "INSERT INTO tasks (id, kind, state, created_at, updated_at)
                 VALUES ('t1', 'upload', 'выполняется-как-то-так', ?1, ?1)",
                [now_rfc3339()],
            )?)
        })
        .expect_err("база приняла несуществующее состояние задачи");
    assert!(err.to_string().contains("CHECK"), "получено: {err}");
}
