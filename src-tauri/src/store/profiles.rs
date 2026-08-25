//! T040 — keeping server profiles in the local database.
//!
//! There is not one field for a secret here: the table holds only a reference to an
//! entry in the OS store (constitution, principle IV). The rule "exactly one is
//! active" (FR-002) is held by a partial unique index in the schema rather than by the
//! carefulness of this code — but switching is still done in a transaction, because
//! otherwise the index simply will not allow clearing the old one and setting the new
//! one as two separate statements.

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
        // The values in the database are constrained by a schema check, but parsing
        // still needs an answer for the unexpected: password access is safer to
        // assume than to fail reading the whole list over one corrupt row.
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

/// Every profile, ordered by name: a person sees this list, and it must be stable.
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

/// The active profile, if one is chosen.
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

/// Whether the name is taken by another profile.
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

/// Create a profile.
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

/// Change a profile.
///
/// `is_active` and `secret_ref` are deliberately left alone: being active is switched
/// by its own command, and the reference to the secret belongs to the core and must
/// not move when ordinary fields are edited.
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

/// Delete a profile. A missing profile is not an error: repeating must be safe.
pub fn remove(db: &Db, id: &str) -> Result<(), DbError> {
    db.with_conn(|c| {
        c.execute("DELETE FROM server_profiles WHERE id = ?1", [id])?;
        Ok(())
    })
}

/// Make a profile active, clearing the mark from the previous one.
///
/// In a transaction and in this order, necessarily: the partial unique index does not
/// let two active profiles exist even for an instant, so "set the new one, then clear
/// the old" simply will not run.
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

/// Remember a confirmed fingerprint (FR-092).
pub fn set_fingerprint(db: &Db, id: &str, fingerprint: &str) -> Result<bool, DbError> {
    db.with_conn(|c| {
        let changed = c.execute(
            "UPDATE server_profiles SET host_fingerprint = ?2 WHERE id = ?1",
            rusqlite::params![id, fingerprint],
        )?;
        Ok(changed > 0)
    })
}
