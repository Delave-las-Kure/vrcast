//! T024 — the server fingerprint: remembering, comparing, catching impersonation
//! (FR-092).
//!
//! A decision stricter than the specification demands was taken here, and it is worth
//! explaining.
//!
//! An ordinary SSH client shows the fingerprint on first connection and asks "do we
//! trust this?" — but by then the connection is established, and people often press
//! yes without looking. We do it differently: **credentials are not sent to a server
//! whose fingerprint is unconfirmed**. The fingerprint can be learnt by a separate
//! action ([`probe`]) that connects, takes the key, and breaks the connection having
//! presented nothing.
//!
//! The difference matters: with an impersonating server, an ordinary client has
//! already given up the password and we have not. This application is handed to people
//! and holds the keys to their servers, so the cost of a mistake here is not ours but
//! theirs (constitution, principle IV).

use super::{Result, ServerAddress, SshError};
use crate::store::db::{now_rfc3339, Db};
use russh::client;
use russh::keys::HashAlg;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The fingerprint of the server's public key in the same form OpenSSH shows it:
/// `SHA256:...`. Matching the form matters — a person has to be able to compare it by
/// eye against what their hosting provider showed them.
pub type HostKey = String;

/// What to do with the key the server presented.
#[derive(Debug, Clone)]
pub enum HostKeyPolicy {
    /// Learn the fingerprint and nothing more. The connection is accepted and goes
    /// no further.
    Probe,
    /// Accept the connection only if the fingerprint matches the known one.
    Require(HostKey),
}

/// What the handler saw during the handshake.
#[derive(Debug, Default)]
pub(crate) struct HostKeySlot {
    pub seen: Option<HostKey>,
    pub mismatch: Option<(HostKey, HostKey)>,
    pub was_certificate: bool,
}

/// The connection's event handler. Its one job is to decide the fate of the server's key.
pub(crate) struct ClientHandler {
    policy: HostKeyPolicy,
    slot: Arc<Mutex<HostKeySlot>>,
}

impl ClientHandler {
    pub(crate) fn new(policy: HostKeyPolicy, slot: Arc<Mutex<HostKeySlot>>) -> Self {
        Self { policy, slot }
    }
}

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &russh::keys::PublicKeyOrCertificate,
    ) -> std::result::Result<bool, Self::Error> {
        let actual = match key {
            russh::keys::PublicKeyOrCertificate::PublicKey { key, .. } => {
                key.fingerprint(HashAlg::Sha256).to_string()
            }
            russh::keys::PublicKeyOrCertificate::Certificate(_) => {
                if let Ok(mut slot) = self.slot.lock() {
                    slot.was_certificate = true;
                }
                return Ok(false);
            }
        };

        if let Ok(mut slot) = self.slot.lock() {
            slot.seen = Some(actual.clone());
        }

        match &self.policy {
            HostKeyPolicy::Probe => Ok(true),
            HostKeyPolicy::Require(expected) => {
                if expected == &actual {
                    Ok(true)
                } else {
                    if let Ok(mut slot) = self.slot.lock() {
                        slot.mismatch = Some((expected.clone(), actual));
                    }
                    // Refused at the handshake: it will never get as far as sending
                    // credentials.
                    Ok(false)
                }
            }
        }
    }
}

pub(crate) fn client_config() -> Arc<client::Config> {
    Arc::new(client::Config {
        // A connection lives a long time (watching a log, a transfer running for
        // hours), so inactivity must not break it — keepalives below watch for life.
        inactivity_timeout: None,
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        ..Default::default()
    })
}

/// Learn a server's fingerprint while presenting it nothing.
///
/// Connects, takes the key out of the handshake, and breaks the connection at once.
/// Neither user name nor password nor key is sent to the server.
pub async fn probe(addr: &ServerAddress) -> Result<HostKey> {
    let slot = Arc::new(Mutex::new(HostKeySlot::default()));
    let handler = ClientHandler::new(HostKeyPolicy::Probe, slot.clone());

    let connected =
        client::connect(client_config(), (addr.host.as_str(), addr.port), handler).await;

    let taken = slot.lock().ok().and_then(|s| s.seen.clone());

    match connected {
        Ok(handle) => {
            // A polite goodbye; a failed goodbye changes nothing — the fingerprint is
            // already in hand.
            let _ = handle
                .disconnect(russh::Disconnect::ByApplication, "", "en")
                .await;
        }
        Err(e) => {
            if taken.is_none() {
                return Err(SshError::Unreachable {
                    addr: addr.clone(),
                    reason: crate::store::redact::safe_display(&e),
                });
            }
        }
    }

    taken.ok_or_else(|| SshError::Unreachable {
        addr: addr.clone(),
        reason: String::from("the server presented no key"),
    })
}

/// Read a server's stored fingerprint.
pub fn stored(
    db: &Db,
    addr: &ServerAddress,
) -> std::result::Result<Option<HostKey>, crate::store::db::DbError> {
    db.with_conn(|c| {
        let mut stmt =
            c.prepare("SELECT fingerprint FROM host_fingerprints WHERE host = ?1 AND port = ?2")?;
        let mut rows = stmt.query(rusqlite::params![addr.host, addr.port])?;
        Ok(match rows.next()? {
            Some(row) => Some(row.get::<_, String>(0)?),
            None => None,
        })
    })
}

/// Remember a fingerprint as confirmed by the person.
///
/// Overwriting an existing one is a deliberate act: this is reached only after someone
/// has seen the new fingerprint and agreed to it.
pub fn remember(
    db: &Db,
    addr: &ServerAddress,
    key: &str,
) -> std::result::Result<(), crate::store::db::DbError> {
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO host_fingerprints (host, port, fingerprint, first_seen)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (host, port) DO UPDATE SET fingerprint = excluded.fingerprint",
            rusqlite::params![addr.host, addr.port, key, now_rfc3339()],
        )?;
        Ok(())
    })
}

/// Forget a fingerprint — when a server has been rebuilt and that is expected, say.
pub fn forget(db: &Db, addr: &ServerAddress) -> std::result::Result<(), crate::store::db::DbError> {
    db.with_conn(|c| {
        c.execute(
            "DELETE FROM host_fingerprints WHERE host = ?1 AND port = ?2",
            rusqlite::params![addr.host, addr.port],
        )?;
        Ok(())
    })
}
