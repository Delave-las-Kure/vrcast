//! T024 — отпечаток сервера: запоминание, сверка, обнаружение подмены (FR-092).
//!
//! Здесь принято решение строже, чем требует спецификация, и оно стоит объяснения.
//!
//! Обычный клиент SSH при первом подключении показывает отпечаток и спрашивает «доверяем?» —
//! но соединение к этому моменту уже установлено, а дальше пользователь нередко жмёт «да»
//! не глядя. Мы делаем иначе: **учётные данные не отправляются серверу, отпечаток которого
//! не подтверждён**. Узнать отпечаток можно отдельным действием ([`probe`]), которое
//! соединяется, забирает ключ и разрывает связь, ничего не предъявив.
//!
//! Разница существенная: при подмене сервера обычный клиент уже отдал пароль, а мы — нет.
//! Приложение раздаётся людям и держит доступы к их серверам, поэтому цена ошибки здесь
//! не своя, а чужая (конституция, принцип IV).

use super::{Result, ServerAddress, SshError};
use crate::store::db::{now_rfc3339, Db};
use russh::client;
use russh::keys::HashAlg;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Отпечаток открытого ключа сервера в том же виде, в каком его показывает OpenSSH:
/// `SHA256:...`. Совпадение вида важно — пользователь должен иметь возможность сверить
/// его глазами с тем, что показал ему хостер.
pub type HostKey = String;

/// Что делать с ключом, который предъявил сервер.
#[derive(Debug, Clone)]
pub enum HostKeyPolicy {
    /// Только узнать отпечаток. Соединение принимается, но дальше него дело не идёт.
    Probe,
    /// Принимать соединение, только если отпечаток совпал с известным.
    Require(HostKey),
}

/// Что увидел обработчик во время рукопожатия.
#[derive(Debug, Default)]
pub(crate) struct HostKeySlot {
    pub seen: Option<HostKey>,
    pub mismatch: Option<(HostKey, HostKey)>,
    pub was_certificate: bool,
}

/// Обработчик событий соединения. Единственная его задача — решить судьбу ключа сервера.
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
                    // Отказ на уровне рукопожатия: до отправки учётных данных дело не дойдёт.
                    Ok(false)
                }
            }
        }
    }
}

pub(crate) fn client_config() -> Arc<client::Config> {
    Arc::new(client::Config {
        // Соединение живёт долго (слежение за журналом, многочасовая передача),
        // поэтому бездействие не должно его рвать — за живостью следят проверки ниже.
        inactivity_timeout: None,
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        ..Default::default()
    })
}

/// Узнать отпечаток сервера, не предъявляя ему ничего.
///
/// Соединяется, забирает ключ из рукопожатия и сразу разрывает связь. Ни имя пользователя,
/// ни пароль, ни ключ серверу не отправляются.
pub async fn probe(addr: &ServerAddress) -> Result<HostKey> {
    let slot = Arc::new(Mutex::new(HostKeySlot::default()));
    let handler = ClientHandler::new(HostKeyPolicy::Probe, slot.clone());

    let connected =
        client::connect(client_config(), (addr.host.as_str(), addr.port), handler).await;

    let taken = slot.lock().ok().and_then(|s| s.seen.clone());

    match connected {
        Ok(handle) => {
            // Вежливо прощаемся; неудача прощания ничего не меняет — отпечаток уже получен.
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
        reason: String::from("сервер не предъявил ключ"),
    })
}

/// Прочитать сохранённый отпечаток сервера.
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

/// Запомнить отпечаток как подтверждённый пользователем.
///
/// Перезапись существующего — осознанное действие: сюда попадают только после того,
/// как пользователь увидел новый отпечаток и согласился с ним.
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

/// Забыть отпечаток — например, когда сервер пересоздан и это ожидаемо.
pub fn forget(db: &Db, addr: &ServerAddress) -> std::result::Result<(), crate::store::db::DbError> {
    db.with_conn(|c| {
        c.execute(
            "DELETE FROM host_fingerprints WHERE host = ?1 AND port = ?2",
            rusqlite::params![addr.host, addr.port],
        )?;
        Ok(())
    })
}
