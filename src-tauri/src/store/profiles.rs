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

/// Change a profile — **only while it still signs in the way it did when it was read** (T636).
///
/// `is_active` and `secret_ref` are deliberately left alone: being active is switched
/// by its own command, and the reference to the secret belongs to the core and must
/// not move when ordinary fields are edited.
///
/// `read_as` is the `auth_kind` the caller read and checked its change against. The check and
/// the write used to be two separate trips to the database, with nothing holding the profile
/// between them: a deployment moving it to `managed_key` in between (T616) had its switch
/// written over by a form that had checked "password → password, nothing to refuse". The
/// condition sits in the `UPDATE` itself, so no order of events lets a check against one way of
/// signing in write over another. `false` — nothing was written: the profile is gone or signs
/// in differently now; read it again and check again.
pub fn update_if_signs_in(db: &Db, p: &ServerProfile, read_as: AuthKind) -> Result<bool, DbError> {
    db.with_conn(|c| {
        let changed = c.execute(
            "UPDATE server_profiles SET
                name = ?2, host = ?3, port = ?4, username = ?5, auth_kind = ?6,
                key_path = ?7, domain = ?8, video_dir = ?9, cdn_base = ?10,
                host_fingerprint = ?11, ipv6_mode = ?12
             WHERE id = ?1 AND auth_kind = ?13",
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
                read_as.as_str(),
            ],
        )?;
        Ok(changed > 0)
    })
}

/// Point a password profile at the key a deployment made for it (T616, T636): `auth_kind =
/// managed_key`, no key path — **these two columns, and only from `password`**.
///
/// Not [`update_if_signs_in`] with the run's copy of the profile: that copy was taken when the
/// run started, and writing all of it back would undo whatever the person renamed or moved in
/// the meantime. And only from `password`, because that is the one way of signing in the made
/// key replaces: a profile a person has since pointed at a key file of their own is theirs, and
/// the run does not take it back. `false` — nothing was written.
///
/// ⚠ **T642 (QA-22 №2) — and only while the profile still points at the machine the key was
/// made for.** `at_start` is the run's copy of the profile, taken when it started: the key was
/// put on *that* address, port and user, and proved there. A profile a person has since pointed
/// at another server (a new address, port or user — still on a password, the new server's)
/// keeps its password: the key of the old server written over it would lock the person out of
/// the new one. The condition is in the `UPDATE` itself, next to `auth_kind`; a rename or a
/// change of any other field does not stop it.
pub fn switch_to_managed_key(db: &Db, at_start: &ServerProfile) -> Result<bool, DbError> {
    db.with_conn(|c| {
        let changed = c.execute(
            "UPDATE server_profiles SET auth_kind = ?2, key_path = NULL
             WHERE id = ?1 AND auth_kind = ?3
               AND host = ?4 AND port = ?5 AND username = ?6",
            rusqlite::params![
                at_start.id,
                AuthKind::ManagedKey.as_str(),
                AuthKind::Password.as_str(),
                at_start.host,
                at_start.port as i64,
                at_start.user,
            ],
        )?;
        Ok(changed > 0)
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

/// Remember the IPv6 choice made on the deploy screen (FR-135) — **this one column and no
/// other** (T626).
///
/// The screen used to save it through [`update`], sending back the whole profile as it held
/// it. A deployment switches a password profile to `managed_key` in the middle of its run
/// (T616), so the profile the screen held was out of date by the time the choice was saved
/// again, and the stale `auth_kind = password` went back into the database over a store that
/// now held the key.
pub fn set_ipv6_mode(db: &Db, id: &str, mode: Ipv6Mode) -> Result<bool, DbError> {
    db.with_conn(|c| {
        let changed = c.execute(
            "UPDATE server_profiles SET ipv6_mode = ?2 WHERE id = ?1",
            rusqlite::params![id, mode.as_str()],
        )?;
        Ok(changed > 0)
    })
}
