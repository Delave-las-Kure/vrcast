//! Доступ к серверу пользователя.
//!
//! Решение R-04: работа с сервером ведётся библиотекой протокола прямо в ядре, а не запуском
//! внешних `ssh`/`scp`. Причина не в чистоте: FR-110 и FR-112 требуют самодостаточности, а на
//! Windows у обычного пользователя этих программ может не быть вовсе.
//!
//! **Одно соединение на сервер, много каналов.** Сервер ограничивает число одновременно
//! устанавливаемых соединений (`maxstartups 10:30:100`), и именно на этом у автора однажды
//! оборвалась сборка лесенки на середине третьего варианта. Поэтому соединение одно, а
//! слежение за журналом, передача файла и короткие команды идут отдельными каналами внутри него.

pub mod auth;
pub mod connection;
pub mod exec;
pub mod fingerprint;
pub mod sftp;

pub use auth::Credentials;
pub use connection::Connection;
pub use exec::CommandOutput;
pub use fingerprint::HostKey;

use crate::store::redact;

/// Адрес сервера. Порт отделён от имени намеренно: он участвует в ключе отпечатка.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServerAddress {
    pub host: String,
    pub port: u16,
}

impl ServerAddress {
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
        }
    }
}

impl std::fmt::Display for ServerAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

/// Ошибки доступа к серверу.
///
/// Разделены по причинам, а не по слоям: каждая соответствует коду из
/// `contracts/ipc-commands.md` и своей подсказке пользователю (FR-105).
#[derive(Debug, thiserror::Error)]
pub enum SshError {
    #[error("сервер {addr} недоступен: {reason}")]
    Unreachable { addr: ServerAddress, reason: String },

    /// FR-092. Самая опасная из ошибок здесь: она означает либо смену сервера,
    /// либо перехват соединения, и молча её проглатывать нельзя.
    #[error("отпечаток сервера {addr} изменился. Ожидался {expected}, получен {actual}")]
    HostKeyChanged {
        addr: ServerAddress,
        expected: String,
        actual: String,
    },

    #[error("отпечаток сервера {addr} ещё не подтверждён")]
    HostKeyUnconfirmed { addr: ServerAddress },

    #[error("сервер предъявил сертификат вместо ключа — такой сервер не поддерживается")]
    HostKeyIsCertificate,

    #[error("вход на сервер не удался. Сервер предлагает способы: {methods}")]
    AuthFailed { methods: String },

    #[error("ключ {path} защищён парольной фразой — укажите её")]
    KeyNeedsPassphrase { path: String },

    #[error("не удалось прочитать ключ {path}: {reason}")]
    KeyUnreadable { path: String, reason: String },

    #[error("команда на сервере не выполнилась: {0}")]
    Exec(String),

    #[error("файловая операция на сервере не удалась: {0}")]
    Sftp(String),

    #[error("ошибка протокола SSH: {0}")]
    Protocol(String),
}

pub type Result<T> = std::result::Result<T, SshError>;

impl SshError {
    /// Ошибка нижележащей библиотеки проходит через вырезание секретов: она о наших
    /// правилах не знает и вполне может вставить в текст то, что мы прячем
    /// (конституция, принцип IV).
    pub(crate) fn protocol(e: impl std::fmt::Display) -> Self {
        Self::Protocol(redact::safe_display(&e))
    }
}
