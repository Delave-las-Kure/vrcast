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
    /// A key held **as itself** rather than as a path (T290a).
    ///
    /// The application makes one when a server is reached by password and keeps it in
    /// the operating system's store, like every other secret (principle IV). There is
    /// no file to point at, and that is deliberate: a private key on disk is a secret
    /// outside the store the constitution keeps them in.
    KeyText {
        openssh: String,
        passphrase: Option<String>,
    },
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
            Self::KeyText { .. } => f.write_str("Credentials::KeyText(<hidden>)"),
        }
    }
}

/// Read a private key from disk.
///
/// Tells "the key is protected by a passphrase" from "the key will not read" — those
/// are different causes calling for different answers (FR-105). Merging them into one
/// error would leave a person guessing whether they picked the wrong file.
/// Read a private key held as itself rather than as a file (T290a).
///
/// The same failures are told apart as for a file: a key that needs a passphrase and a
/// key that will not read are different causes with different answers. What cannot
/// happen here is "the file is not there" — which is the point.
pub fn load_key_text(openssh: &str, passphrase: Option<&str>) -> Result<PrivateKey> {
    if let Some(p) = passphrase {
        redact::register(p);
    }
    // The key itself is registered too. It reaches an error message only through a
    // library's own words, and those are exactly the words nobody thought to check.
    redact::register(openssh);

    let key =
        russh::keys::PrivateKey::from_openssh(openssh).map_err(|e| SshError::KeyUnreadable {
            path: String::from("the key kept by the application"),
            reason: redact::safe_display(&e),
        })?;
    if !key.is_encrypted() {
        return Ok(key);
    }
    let Some(passphrase) = passphrase else {
        return Err(SshError::KeyNeedsPassphrase {
            path: String::from("the key kept by the application"),
        });
    };
    key.decrypt(passphrase)
        .map_err(|e| SshError::KeyUnreadable {
            path: String::from("the key kept by the application"),
            reason: redact::safe_display(&e),
        })
}

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
