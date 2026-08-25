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
    #[error("server {addr} is unreachable: {reason}")]
    Unreachable { addr: ServerAddress, reason: String },

    /// FR-092. Самая опасная из ошибок здесь: она означает либо смену сервера,
    /// либо перехват соединения, и молча её проглатывать нельзя.
    #[error("fingerprint of server {addr} has changed: expected {expected}, got {actual}")]
    HostKeyChanged {
        addr: ServerAddress,
        expected: String,
        actual: String,
    },

    #[error("fingerprint of server {addr} has not been confirmed yet")]
    HostKeyUnconfirmed { addr: ServerAddress },

    #[error("the server presented a certificate instead of a key, which is not supported")]
    HostKeyIsCertificate,

    #[error("sign-in failed; the server offers: {methods}")]
    AuthFailed { methods: String },

    #[error("key {path} is protected by a passphrase")]
    KeyNeedsPassphrase { path: String },

    #[error("could not read key {path}: {reason}")]
    KeyUnreadable { path: String, reason: String },

    #[error("command on the server failed: {0}")]
    Exec(String),

    /// Файловая операция на сервере не удалась.
    ///
    /// `kind` отделяет причины, которые ведут человека в РАЗНЫЕ стороны. Раньше
    /// их не было, и любая файловая беда объявлялась нехваткой прав с подсказкой
    /// «проверьте владельца каталога» — при полном диске человек шёл чинить то,
    /// что не сломано, а настоящая причина лежала на виду в тексте ошибки
    /// (задолженность T071).
    #[error("file operation on the server failed: {reason}")]
    Sftp { kind: SftpFailure, reason: String },

    #[error("SSH protocol error: {0}")]
    Protocol(String),
}

/// Отчего не удалась файловая операция.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SftpFailure {
    /// Нет прав: не тот владелец, не те права на каталоге.
    Denied,
    /// На сервере кончилось место.
    NoSpace,
    /// Файла или каталога нет.
    Missing,
    /// Связь оборвалась посреди операции — повод повторить, а не чинить сервер.
    Interrupted,
    /// Что-то ещё. Текст ошибки сохраняется целиком: он непонятен, но его можно
    /// найти поиском, а «файловая операция не удалась» — нельзя.
    Other,
}

impl SftpFailure {
    /// Опознать причину по жалобе библиотеки.
    ///
    /// Разбор по тексту — не от хорошей жизни: слой SFTP отдаёт код состояния
    /// вперемешку с сообщениями транспорта, и единственное, что есть всегда, —
    /// это текст. Незнакомое считается `Other`, а не угадывается: неверная догадка
    /// здесь хуже честного «не знаю», потому что уводит человека чинить не то.
    pub fn classify(text: &str) -> Self {
        let t = text.to_ascii_lowercase();
        if t.contains("no space") || t.contains("quota") || t.contains("disk full") {
            Self::NoSpace
        } else if t.contains("permission denied") || t.contains("access denied") {
            Self::Denied
        } else if t.contains("no such file") || t.contains("not found") {
            Self::Missing
        } else if t.contains("connection") || t.contains("eof") || t.contains("broken pipe") {
            Self::Interrupted
        } else {
            Self::Other
        }
    }
}

impl SshError {
    /// Собрать файловую ошибку, опознав причину по тексту.
    pub fn sftp(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self::Sftp {
            kind: SftpFailure::classify(&reason),
            reason,
        }
    }
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
