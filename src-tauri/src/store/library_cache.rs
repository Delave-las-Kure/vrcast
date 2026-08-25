//! T046 — кеш библиотеки: последнее известное состояние сервера.
//!
//! Хранится готовым ответом команды, а не разложенным по таблицам. Это снимок для
//! показа: запросов к нему не делают, а разложить его значило бы держать в согласии
//! две схемы одного и того же — и однажды не удержать.

use crate::commands::library::LibraryView;
use crate::store::db::{now_rfc3339, Db, DbError};
use rusqlite::OptionalExtension;

/// Запомнить последнее удачно прочитанное состояние библиотеки.
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

/// Прочитать последнее известное состояние.
///
/// Испорченный кеш — это отсутствие кеша, а не ошибка: он ускоряет показ и ничего
/// не решает, и ронять из-за него открытие библиотеки было бы несоразмерно.
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
            tracing::warn!(server = server_id, error = %e, "кеш библиотеки не разобрать, читаем с сервера");
            None
        }
    }))
}

/// Забыть кеш сервера.
pub fn forget(db: &Db, server_id: &str) -> Result<(), DbError> {
    db.with_conn(|c| {
        c.execute(
            "DELETE FROM library_cache WHERE server_id = ?1",
            [server_id],
        )?;
        Ok(())
    })
}
