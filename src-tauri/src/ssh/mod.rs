//! Reaching the user's server.
//!
//! Decision R-04: the server is worked with by a protocol library inside the core
//! rather than by running external `ssh`/`scp`. Not for purity: FR-110 and FR-112
//! require self-sufficiency, and on Windows an ordinary person may not have those
//! programs at all.
//!
//! **One connection per server, many channels.** A server limits how many connections
//! may be established at once (`maxstartups 10:30:100`), and that is exactly what once
//! broke the author's ladder build halfway through the third variant. So there is one
//! connection, and watching a log, transferring a file and running short commands go
//! down separate channels inside it.

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

/// A server's address. The port is kept apart from the name deliberately: it takes
/// part in the fingerprint key.
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

/// Failures in reaching the server.
///
/// Divided by cause rather than by layer: each corresponds to a code from
/// `contracts/ipc-commands.md` and to its own answer for the person (FR-105).
#[derive(Debug, thiserror::Error)]
pub enum SshError {
    #[error("server {addr} is unreachable: {reason}")]
    Unreachable { addr: ServerAddress, reason: String },

    /// FR-092. The most dangerous failure here: it means either the server changed or
    /// the connection was intercepted, and it must not be swallowed quietly.
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

    /// A file operation on the server failed.
    ///
    /// `kind` separates the causes that send a person in DIFFERENT directions. There
    /// used to be none, and every file trouble was reported as a permission problem
    /// with the hint "check who owns the directory" — on a full disk a person went to
    /// fix what was not broken while the real cause lay in plain sight in the error
    /// text (debt T071).
    #[error("file operation on the server failed: {reason}")]
    Sftp { kind: SftpFailure, reason: String },

    #[error("SSH protocol error: {0}")]
    Protocol(String),
}

/// Why a file operation failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SftpFailure {
    /// No permission: the wrong owner, the wrong mode on the directory.
    Denied,
    /// The server has run out of room.
    NoSpace,
    /// The file or directory is not there.
    Missing,
    /// The connection broke mid-operation — a reason to retry, not to fix the server.
    Interrupted,
    /// Something else. The error text is kept whole: it makes little sense, but it
    /// can be searched for, and "a file operation failed" cannot.
    Other,
}

impl SftpFailure {
    /// Work out the cause from the library's complaint.
    ///
    /// Parsing text is not a happy choice: the SFTP layer hands back a status code
    /// mixed in with transport messages, and the one thing always present is the text.
    /// The unfamiliar counts as `Other` rather than being guessed at: a wrong guess
    /// here is worse than an honest "unknown", because it sends a person to fix the
    /// wrong thing.
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
    /// Build a file error, working out the cause from the text.
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
    /// The underlying library's error passes through secret redaction: it knows
    /// nothing of our rules and may well put into its text the very thing we hide
    /// (constitution, principle IV).
    pub(crate) fn protocol(e: impl std::fmt::Display) -> Self {
        Self::Protocol(redact::safe_display(&e))
    }
}
