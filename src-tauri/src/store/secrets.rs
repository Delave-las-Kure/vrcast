//! T009 — secrets live only in the operating system's credential store.
//!
//! Constitution, principle IV. The application is handed out to people and holds the keys
//! to their servers: a leak here is someone else's server, not our own. So passwords,
//! passphrases and private keys are written neither into the settings nor into the local
//! database — what lies there is only a `SecretRef`, a pointer to an entry in the
//! operating system's store.
//!
//! The store differs per platform: Credential Manager on Windows, Secret Service on Linux.
//! The choice is made per platform in `Cargo.toml`; there is no difference here.
//!
//! **An important property**: every secret passing through this layer automatically joins
//! the list of things cut out of the output (`super::redact`). That way protection against
//! leaking into a log does not depend on whether the author of a particular line
//! remembered it.

use super::redact;

/// The service name in the operating system store. It is what a person sees the entries
/// listed under in their system credential manager.
const SERVICE: &str = "VRCast Studio";

/// A pointer to a secret. This — and not the value — is what is stored in the database
/// and what crosses the boundaries between layers.
///
/// `Debug` prints only the pointer: that is not the secret but its address.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SecretRef(String);

impl SecretRef {
    /// The secret for reaching a server: a password, or a key's passphrase.
    pub fn for_server(server_id: &str) -> Self {
        Self(format!("server/{server_id}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Rebuild the pointer from a value read out of the database.
    pub fn from_stored(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl std::fmt::Display for SecretRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("secret not found in the operating system store")]
    NotFound,

    /// The underlying library's message passes through redaction: it knows nothing of our
    /// rules and may well put into its error text the very thing we hide.
    #[error("the operating system credential store is unavailable: {0}")]
    Backend(String),
}

pub type Result<T> = std::result::Result<T, SecretError>;

/// A store of secrets.
///
/// Separated from its implementation by a trait not for abstraction's own sake, but so that
/// tests do not touch a person's real store: a test that leaves entries behind in someone's
/// system password manager is a bad test.
pub trait SecretStore: Send + Sync {
    fn set(&self, reference: &SecretRef, value: &str) -> Result<()>;
    fn get(&self, reference: &SecretRef) -> Result<String>;
    fn delete(&self, reference: &SecretRef) -> Result<()>;
}

/// The real operating system store.
#[derive(Debug, Default)]
pub struct OsSecretStore;

impl OsSecretStore {
    pub fn new() -> Self {
        Self
    }

    fn entry(reference: &SecretRef) -> Result<keyring::Entry> {
        keyring::Entry::new(SERVICE, reference.as_str())
            .map_err(|e| SecretError::Backend(redact::safe_display(&e)))
    }
}

impl SecretStore for OsSecretStore {
    fn set(&self, reference: &SecretRef, value: &str) -> Result<()> {
        // Registered BEFORE the write: should the write fail, the error message can no
        // longer carry the secret outside.
        redact::register(value);

        Self::entry(reference)?
            .set_password(value)
            .map_err(|e| SecretError::Backend(redact::safe_display(&e)))?;
        tracing::debug!(reference = %reference, "secret saved to the operating system store");
        Ok(())
    }

    fn get(&self, reference: &SecretRef) -> Result<String> {
        let value = Self::entry(reference)?.get_password().map_err(|e| {
            if matches!(e, keyring::Error::NoEntry) {
                SecretError::NotFound
            } else {
                SecretError::Backend(redact::safe_display(&e))
            }
        })?;
        redact::register(&value);
        Ok(value)
    }

    fn delete(&self, reference: &SecretRef) -> Result<()> {
        // The value is read BEFORE the deletion so that the masking comes off that value
        // in particular. Otherwise it would stay on the redaction list forever — no
        // disaster in itself, but the list grows with every deleted profile, and redaction
        // runs over every line of the log.
        let previous = Self::entry(reference)
            .ok()
            .and_then(|e| e.get_password().ok());

        match Self::entry(reference)?.delete_credential() {
            Ok(()) => {
                if let Some(value) = previous {
                    redact::forget(&value);
                }
                tracing::debug!(reference = %reference, "secret removed from the operating system store");
                Ok(())
            }
            // Deleting what is not there is not an error: repeating must be safe
            // (constitution, principle V).
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(SecretError::Backend(redact::safe_display(&e))),
        }
    }
}

/// A store in memory, for tests. It never touches a person's real store.
#[derive(Debug, Default)]
pub struct InMemorySecretStore {
    items: std::sync::RwLock<std::collections::HashMap<String, String>>,
}

impl InMemorySecretStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for InMemorySecretStore {
    fn set(&self, reference: &SecretRef, value: &str) -> Result<()> {
        redact::register(value);
        self.items
            .write()
            .map_err(|_| SecretError::Backend("the in-memory store is poisoned".into()))?
            .insert(reference.as_str().to_owned(), value.to_owned());
        Ok(())
    }

    fn get(&self, reference: &SecretRef) -> Result<String> {
        let value = self
            .items
            .read()
            .map_err(|_| SecretError::Backend("the in-memory store is poisoned".into()))?
            .get(reference.as_str())
            .cloned()
            .ok_or(SecretError::NotFound)?;
        redact::register(&value);
        Ok(value)
    }

    fn delete(&self, reference: &SecretRef) -> Result<()> {
        let previous = self
            .items
            .write()
            .map_err(|_| SecretError::Backend("the in-memory store is poisoned".into()))?
            .remove(reference.as_str());
        // The masking comes off THIS value in particular rather than off all of them at
        // once: the other profiles' secrets are still alive (T073).
        if let Some(value) = previous {
            redact::forget(&value);
        }
        Ok(())
    }
}
