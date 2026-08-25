//! T023 — ways of signing in to a server, including a key with a passphrase (FR-096).

use super::{Result, SshError};
use crate::store::redact;
use russh::keys::PrivateKey;
use std::path::{Path, PathBuf};

/// What we sign in to the server with.
///
/// A secret lives here exactly as long as the connection does: it never reaches the
/// database, and comes from the OS store by reference (constitution, principle IV).
#[derive(Clone)]
pub enum Credentials {
    /// A private key. The passphrase is not always needed — but if the key is
    /// protected by one, it will not read without it, and that is its own error with
    /// its own answer for the person.
    Key {
        path: PathBuf,
        passphrase: Option<String>,
    },
    Password(String),
}

/// `Debug` deliberately prints neither the password nor the passphrase: the whole
/// structure can end up in debug output, and that is the commonest way of leaking one.
impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Key { path, passphrase } => f
                .debug_struct("Credentials::Key")
                .field("path", path)
                .field(
                    "passphrase",
                    &if passphrase.is_some() { "set" } else { "none" },
                )
                .finish(),
            Self::Password(_) => f.write_str("Credentials::Password(<hidden>)"),
        }
    }
}

/// Read a private key from disk.
///
/// Tells "the key is protected by a passphrase" from "the key will not read" — those
/// are different causes calling for different answers (FR-105). Merging them into one
/// error would leave a person guessing whether they picked the wrong file.
pub fn load_key(path: &Path, passphrase: Option<&str>) -> Result<PrivateKey> {
    // The passphrase is registered before the read: if the read fails, the error
    // message will no longer be able to carry it out.
    if let Some(p) = passphrase {
        redact::register(p);
    }

    russh::keys::load_secret_key(path, passphrase).map_err(|e| {
        let shown = path.display().to_string();
        match e {
            russh::keys::Error::KeyIsEncrypted => SshError::KeyNeedsPassphrase { path: shown },
            other => SshError::KeyUnreadable {
                path: shown,
                reason: redact::safe_display(&other),
            },
        }
    })
}
