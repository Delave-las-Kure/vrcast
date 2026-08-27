//! T290a — a key the application makes for a server it is about to deploy on.
//!
//! **Why it has to make one at all.** The ordinary first contact with a bought server is an
//! address and a root password. A deployment turns password logins off — that is what FR-126
//! asks for — and it may only do that once there is another way in, or it locks out the
//! person along with itself (R-12). So the way in has to exist before the password goes, and
//! expecting somebody to open a console and run `ssh-keygen` is exactly the console visit
//! SC-014 promises they will not need.
//!
//! **Where the private half lives.** In the operating system's store, whole, like every other
//! secret (constitution, principle IV; owner's decision 2026-08-27). Not in a file: a file is
//! a secret outside the place this project keeps secrets, and the reason to want one — being
//! able to use the key with ordinary `ssh` — is better served by handing it out on request
//! than by leaving it lying about.
//!
//! Ed25519 and nothing else: it is small, every OpenSSH since 6.5 takes it, and there is no
//! key size to get wrong.

use russh::keys::{Algorithm, PrivateKey};

use super::{Result, SshError};

/// A freshly made pair.
///
/// The two halves travel together and part at once: the public one goes to the server, the
/// private one to the store. Nothing here writes either of them anywhere.
pub struct MadeKey {
    /// The private half in OpenSSH's own format — what goes into the OS store.
    pub private_openssh: String,
    /// The public half as a line of `authorized_keys`.
    pub public_openssh: String,
}

/// Make one.
pub fn make(comment: &str) -> Result<MadeKey> {
    // The thread's own cryptographic generator, seeded from the operating system and reseeded
    // as it goes. `SysRng` — the raw system source — cannot be used here: it offers only a
    // fallible read, and making a key is not a place to handle "the system would not give
    // randomness this instant" by carrying on.
    let mut key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).map_err(|e| {
        SshError::KeyUnreadable {
            path: String::from("a key the application was making"),
            reason: e.to_string(),
        }
    })?;
    // The comment is what a person sees in `authorized_keys` on their own server, and it is
    // the only thing there that says where the key came from. Without it the line reads as
    // somebody's stray key.
    key.set_comment(comment);

    // Left without a passphrase on purpose. A passphrase protects a key **on disk**, from
    // whoever can read the file; this one is never on disk, and the store it lives in is
    // already locked by the person's own login. A passphrase here would be a second secret
    // kept beside the first, in the same place.
    let private = key
        .to_openssh(russh::keys::ssh_key::LineEnding::LF)
        .map_err(|e| SshError::KeyUnreadable {
            path: String::from("a key the application was making"),
            reason: e.to_string(),
        })?
        .to_string();
    // Registered so it can never reach a message: it goes through the store and through
    // nothing else, and the way a secret escapes is somebody's `{:?}`.
    crate::store::redact::register(&private);

    let public = key
        .public_key()
        .to_openssh()
        .map_err(|e| SshError::KeyUnreadable {
            path: String::from("a key the application was making"),
            reason: e.to_string(),
        })?;

    Ok(MadeKey {
        private_openssh: private,
        public_openssh: public,
    })
}
