//! T040 — хранение профилей серверов в локальной базе.
//!
//! Здесь нет ни одного поля под секрет: в таблице лежит только ссылка на запись
//! в хранилище ОС (конституция, принцип IV). Само правило «активен ровно один»
//! (FR-002) держится частичным уникальным индексом в схеме, а не аккуратностью
//! этого кода — но переключение всё равно делается транзакцией, иначе индекс
//! просто не даст снять старый и поставить новый двумя отдельными запросами.

use crate::domain::server_profile::{AuthKind, Ipv6Mode, ServerProfile};
use crate::store::db::{now_rfc3339, Db, DbError};
use rusqlite::OptionalExtension;

fn row_to_profile(row: &rusqlite::Row<'_>) -> rusqlite::Result<ServerProfile> {
    let auth_kind: String = row.get("auth_kind")?;
    let ipv6: Option<String> = row.get("ipv6_mode")?;
    Ok(ServerProfile {
        id: row.get("id")?,
        name: row.get("name")?,
        host: row.get("host")?,
        port: row.get::<_, i64>("port")? as u16,
        user: row.get("username")?,
        // Значения в базе ограничены проверкой схемы, но разбор всё равно должен
        // иметь ответ на неожиданное: доступ по паролю безопаснее подставить, чем
        // уронить чтение всего списка из-за одной испорченной строки.
        auth_kind: AuthKind::parse(&auth_kind).unwrap_or(AuthKind::Password),
        secret_ref: row.get("secret_ref")?,
        key_path: row.get("key_path")?,
        domain: row.get("domain")?,
        video_dir: row.get("video_dir")?,
        cdn_base: row.get("cdn_base")?,
        host_fingerprint: row.get("host_fingerprint")?,
        ipv6_mode: ipv6.as_deref().and_then(Ipv6Mode::parse),
        is_active: row.get::<_, i64>("is_active")? != 0,
    })
}

/// Все профили. Порядок — по имени: список видит человек, и он должен быть устойчив.
pub fn list(db: &Db) -> Result<Vec<ServerProfile>, DbError> {
    db.with_conn(|c| {
        let mut stmt = c.prepare("SELECT * FROM server_profiles ORDER BY name")?;
        let rows = stmt.query_map([], row_to_profile)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    })
}

pub fn get(db: &Db, id: &str) -> Result<Option<ServerProfile>, DbError> {
    db.with_conn(|c| {
        Ok(c.query_row(
            "SELECT * FROM server_profiles WHERE id = ?1",
            [id],
            row_to_profile,
        )
        .optional()?)
    })
}

/// Активный профиль, если он выбран.
pub fn active(db: &Db) -> Result<Option<ServerProfile>, DbError> {
    db.with_conn(|c| {
        Ok(c.query_row(
            "SELECT * FROM server_profiles WHERE is_active = 1",
            [],
            row_to_profile,
        )
        .optional()?)
    })
}

/// Занято ли имя другим профилем.
pub fn name_taken(db: &Db, name: &str, except_id: Option<&str>) -> Result<bool, DbError> {
    db.with_conn(|c| {
        let count: i64 = c.query_row(
            "SELECT COUNT(*) FROM server_profiles WHERE name = ?1 AND id <> ?2",
            rusqlite::params![name, except_id.unwrap_or("")],
            |r| r.get(0),
        )?;
        Ok(count > 0)
    })
}

/// Создать профиль.
pub fn insert(db: &Db, p: &ServerProfile) -> Result<(), DbError> {
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO server_profiles
                (id, name, host, port, username, auth_kind, secret_ref, key_path,
                 domain, video_dir, cdn_base, host_fingerprint, ipv6_mode, is_active,
                 last_seen_state, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, NULL, ?15)",
            rusqlite::params![
                p.id,
                p.name,
                p.host,
                p.port as i64,
                p.user,
                p.auth_kind.as_str(),
                p.secret_ref,
                p.key_path,
                p.domain,
                p.video_dir,
                p.cdn_base,
                p.host_fingerprint,
                p.ipv6_mode.map(|m| m.as_str()),
                i64::from(p.is_active),
                now_rfc3339(),
            ],
        )?;
        Ok(())
    })
}

/// Изменить профиль.
///
/// `is_active` и `secret_ref` намеренно не трогаются: активность переключается
/// отдельной командой, а ссылка на секрет принадлежит ядру и не должна меняться
/// при обычной правке полей.
pub fn update(db: &Db, p: &ServerProfile) -> Result<(), DbError> {
    db.with_conn(|c| {
        c.execute(
            "UPDATE server_profiles SET
                name = ?2, host = ?3, port = ?4, username = ?5, auth_kind = ?6,
                key_path = ?7, domain = ?8, video_dir = ?9, cdn_base = ?10,
                host_fingerprint = ?11, ipv6_mode = ?12
             WHERE id = ?1",
            rusqlite::params![
                p.id,
                p.name,
                p.host,
                p.port as i64,
                p.user,
                p.auth_kind.as_str(),
                p.key_path,
                p.domain,
                p.video_dir,
                p.cdn_base,
                p.host_fingerprint,
                p.ipv6_mode.map(|m| m.as_str()),
            ],
        )?;
        Ok(())
    })
}

/// Удалить профиль. Отсутствие профиля не ошибка: повтор обязан быть безопасным.
pub fn remove(db: &Db, id: &str) -> Result<(), DbError> {
    db.with_conn(|c| {
        c.execute("DELETE FROM server_profiles WHERE id = ?1", [id])?;
        Ok(())
    })
}

/// Сделать профиль активным, сняв отметку с прежнего.
///
/// Обязательно транзакцией и в этом порядке: частичный уникальный индекс не даёт
/// существовать двум активным даже на миг, поэтому «поставить новый, потом снять
/// старый» просто не выполнится.
pub fn set_active(db: &Db, id: &str) -> Result<bool, DbError> {
    db.with_conn_mut(|c| {
        let tx = c.transaction()?;
        tx.execute(
            "UPDATE server_profiles SET is_active = 0 WHERE is_active = 1",
            [],
        )?;
        let changed = tx.execute(
            "UPDATE server_profiles SET is_active = 1 WHERE id = ?1",
            [id],
        )?;
        tx.commit()?;
        Ok(changed > 0)
    })
}

/// Запомнить подтверждённый отпечаток (FR-092).
pub fn set_fingerprint(db: &Db, id: &str, fingerprint: &str) -> Result<bool, DbError> {
    db.with_conn(|c| {
        let changed = c.execute(
            "UPDATE server_profiles SET host_fingerprint = ?2 WHERE id = ?1",
            rusqlite::params![id, fingerprint],
        )?;
        Ok(changed > 0)
    })
}
