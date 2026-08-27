//! Working with the server: what cannot be checked without one.
//!
//! One line runs between this layer and `domain`: there lives **what is known**, here
//! lives **how to find it out**. The rules for parsing the catalogue live in
//! `domain::manifest` and are checked without a server; the order of reading and
//! writing it lives here, and is checked against a real OpenSSH.

pub mod active_use;
pub mod checksum;
pub mod detect;
pub mod disk;
pub mod env_import;
pub mod free_space;
pub mod hls_package;
pub mod hls_verify;
pub mod limits;
pub mod listing;
pub mod manifest_io;
pub mod probe_moov;
pub mod reconcile;
pub mod upload;
pub mod viewers;

/// Entries of the serving directory that are not video and are not shown in the library.
///
/// Both belong to the application (`contracts/server-contract.md`). Showing them to a
/// person would be offering them the chance to delete their own library's catalogue.
pub const SERVICE_ENTRIES: [&str; 2] = [manifest_io::MANIFEST_NAME, "_slow"];

/// Quote a string for a command on the server.
///
/// Paths come from a person's profile and contain anything at all: spaces, Cyrillic,
/// occasionally quotes. Dropping them into a command as they stand means both broken
/// paths and the chance of running something other than what was intended.
pub(crate) fn shell_quote(value: &str) -> String {
    // Inside single quotes a shell interprets nothing but the quote itself; it is
    // closed, escaped, and opened again.
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// Join a directory and a name into a path on the server.
pub(crate) fn join_remote(dir: &str, name: &str) -> String {
    format!("{}/{}", dir.trim_end_matches('/'), name)
}

/// Connect to a server by its profile.
///
/// One point for the whole application: connecting is the only place a secret is taken
/// out of the store, and spreading that across several places means forgetting the
/// fingerprint check in one of them sooner or later.
pub async fn connect(
    secrets: &dyn crate::store::secrets::SecretStore,
    profile: &crate::domain::server_profile::ServerProfile,
) -> crate::ssh::Result<crate::ssh::Connection> {
    use crate::domain::server_profile::AuthKind;
    use crate::ssh::{Connection, Credentials, ServerAddress, SshError};
    use crate::store::secrets::SecretRef;

    let addr = ServerAddress::new(&profile.host, profile.port);

    // Credentials are not sent to a server whose fingerprint is unconfirmed. This is
    // not a "strict setting" but a condition: no confirmation, no connection.
    let Some(expected) = profile.host_fingerprint.clone() else {
        return Err(SshError::HostKeyUnconfirmed { addr });
    };

    let secret = secrets
        .get(&SecretRef::from_stored(&profile.secret_ref))
        .map_err(|e| SshError::KeyUnreadable {
            path: profile.secret_ref.clone(),
            reason: e.to_string(),
        })?;

    let credentials = match profile.auth_kind {
        AuthKind::Key => Credentials::Key {
            path: profile.key_path.clone().unwrap_or_default().into(),
            passphrase: Some(secret),
        },
        AuthKind::Password => Credentials::Password(secret),
    };

    Connection::connect(addr, &profile.user, credentials, &expected).await
}
