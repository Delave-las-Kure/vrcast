//! T627 — the proof that a fresh login with our key works (`key_works`) signs in with the key
//! the profile says it uses, a managed key included (QA-20 №2).
//!
//! It used to know only a key made by the current run and a key file: a profile on a managed
//! key (the private key in the operating system's store, no file) had nothing to sign in with,
//! and the proof said "no" without trying. A password run cut after `SshHardening` — the
//! profile already switched by T616 — could then never be finished: `SshKey` was not seen
//! applied, and its apply failed with "a fresh login with it does not work". The real repeat
//! against a container is `tests/integration/deploy_key_kept.rs`.

use std::path::Path;

use vrcast_studio_lib::commands::deploy::proof_credentials;
use vrcast_studio_lib::domain::server_profile::AuthKind;
use vrcast_studio_lib::ssh::Credentials;

const STORED: &str =
    "-----BEGIN OPENSSH PRIVATE KEY-----\nstored\n-----END OPENSSH PRIVATE KEY-----\n";
const MADE: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nmade\n-----END OPENSSH PRIVATE KEY-----\n";

#[test]
fn a_managed_key_is_proved_with_the_key_from_the_store() {
    match proof_credentials(AuthKind::ManagedKey, None, None, Some(STORED)) {
        Some(Credentials::KeyText {
            openssh,
            passphrase,
        }) => {
            assert_eq!(openssh, STORED);
            assert_eq!(passphrase, None);
        }
        other => panic!("a managed key was not proved with itself: {other:?}"),
    }
}

#[test]
fn a_managed_key_with_nothing_in_the_store_has_nothing_to_prove_with() {
    assert!(proof_credentials(AuthKind::ManagedKey, None, None, None).is_none());
    assert!(proof_credentials(AuthKind::ManagedKey, None, None, Some("  ")).is_none());
}

#[test]
fn a_key_made_by_this_run_comes_first() {
    // A password profile: the run made a key, the store still holds the password.
    match proof_credentials(AuthKind::Password, Some(MADE), None, Some("hunter2")) {
        Some(Credentials::KeyText { openssh, .. }) => assert_eq!(openssh, MADE),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_key_file_is_proved_with_the_file_and_its_passphrase() {
    let path = Path::new("/home/me/.ssh/id_ed25519");
    match proof_credentials(AuthKind::Key, None, Some(path), Some("phrase")) {
        Some(Credentials::Key {
            path: p,
            passphrase,
        }) => {
            assert_eq!(p, path);
            assert_eq!(passphrase.as_deref(), Some("phrase"));
        }
        other => panic!("{other:?}"),
    }
    match proof_credentials(AuthKind::Key, None, Some(path), None) {
        Some(Credentials::Key { passphrase, .. }) => assert_eq!(passphrase, None),
        other => panic!("{other:?}"),
    }
    assert!(proof_credentials(AuthKind::Key, None, None, Some("phrase")).is_none());
}

#[test]
fn a_password_is_never_what_the_key_proof_signs_in_with() {
    // Proving "the password works" is not the question the proof asks.
    assert!(proof_credentials(AuthKind::Password, None, None, Some("hunter2")).is_none());
}
