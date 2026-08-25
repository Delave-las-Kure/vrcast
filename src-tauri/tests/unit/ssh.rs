//! Tests for the server-access layer that need no server (T022–T025).
//!
//! Everything that needs a real OpenSSH is checked by a separate run of
//! `cargo run --example ssh_smoke` — tests must not depend on somebody else's server or on a
//! network. What stays here is what does not depend on the network and therefore must be a
//! test: parsing keys, not disclosing secrets in debug output, and storing fingerprints.

use std::path::{Path, PathBuf};
use vrcast_studio_lib::ssh::{auth, fingerprint, Credentials, ServerAddress, SshError};
use vrcast_studio_lib::store::db::Db;

/// The key is made on the spot and never reaches the repository — see
/// `tests/support/test_key.rs`. These tests used to take a ready file, and on a developer's
/// machine it was there from earlier runs; in continuous integration it was not, and the
/// tests failed.
fn fixture(_name: &str) -> PathBuf {
    super::test_key::ensure().expect("the test key was not created")
}

const FIXTURE_PASSPHRASE: &str = super::test_key::PASSPHRASE;

#[test]
fn the_key_is_made_even_when_it_is_asked_for_all_at_once() {
    // The tests run in several threads and ask for the key at the same time. Without a lock
    // two of them decide the key is missing, both wipe the files and both call ssh-keygen;
    // the second runs into a file that already exists and exits with an error without saying
    // a word. A run on Windows caught this on 2026-08-25 — but only by luck, and so the race
    // is checked deliberately rather than left to chance.
    let threads: Vec<_> = (0..8)
        .map(|_| std::thread::spawn(super::test_key::ensure))
        .collect();

    for (i, t) in threads.into_iter().enumerate() {
        let outcome = t.join().expect("the thread panicked");
        let path = outcome.unwrap_or_else(|e| panic!("thread {i}: {e}"));
        assert!(
            path.exists(),
            "thread {i}: there is no key at the promised path"
        );
    }
}

#[test]
fn a_key_with_a_passphrase_reads_when_the_passphrase_is_given() {
    // FR-096: people do have keys protected by a passphrase, and that must work rather than
    // turn into an unintelligible failure to read a file.
    let key = auth::load_key(&fixture("encrypted_ed25519.key"), Some(FIXTURE_PASSPHRASE))
        .expect("a protected key would not read with the right passphrase");
    assert_eq!(key.algorithm().as_str(), "ssh-ed25519");
}

#[test]
fn a_key_with_no_passphrase_gives_an_error_of_its_own() {
    // Of its own, specifically: "a passphrase is needed" and "the file will not read" ask
    // different things of a person, and merging them into one error makes them guess
    // (FR-105).
    match auth::load_key(&fixture("encrypted_ed25519.key"), None) {
        Err(SshError::KeyNeedsPassphrase { path }) => {
            assert!(
                path.contains("encrypted_ed25519"),
                "the path was lost: {path}"
            );
        }
        Err(other) => panic!("KeyNeedsPassphrase was expected, got: {other}"),
        Ok(_) => panic!("a protected key read without a passphrase"),
    }
}

#[test]
fn a_wrong_passphrase_is_not_passed_off_as_an_unreadable_file() {
    let err = auth::load_key(
        &fixture("encrypted_ed25519.key"),
        Some("not-the-passphrase"),
    )
    .expect_err("the key read with a wrong passphrase");
    // Whatever the wording, the passphrase itself must not turn up in it.
    let text = err.to_string();
    assert!(
        !text.contains("not-the-passphrase"),
        "the passphrase reached the error text: {text}"
    );
}

#[test]
fn a_missing_key_file_gives_a_read_error() {
    match auth::load_key(Path::new("no-such-file.key"), None) {
        Err(SshError::KeyUnreadable { path, .. }) => assert!(path.contains("no-such-file")),
        Err(other) => panic!("KeyUnreadable was expected, got: {other}"),
        Ok(_) => panic!("a key that does not exist was read"),
    }
}

#[test]
fn the_debug_output_of_credentials_does_not_disclose_the_secret() {
    // The commonest way to leak is not "the password was printed" but "the whole structure
    // reached the output". So Debug for Credentials is written by hand (constitution,
    // principle IV).
    let pass = Credentials::Password(String::from("a-very-secret-password-1"));
    let shown = format!("{pass:?}");
    assert!(
        !shown.contains("a-very-secret-password-1"),
        "the password is visible: {shown}"
    );

    let key = Credentials::Key {
        path: PathBuf::from("/home/u/.ssh/id_ed25519"),
        passphrase: Some(String::from("a-secret-passphrase-2")),
    };
    let shown = format!("{key:?}");
    assert!(
        !shown.contains("a-secret-passphrase-2"),
        "the passphrase is visible: {shown}"
    );
    // The path must stay, though: it is not a secret and is needed to sort out trouble.
    assert!(
        shown.contains("id_ed25519"),
        "the path to the key was lost: {shown}"
    );
    assert!(
        shown.contains("set"),
        "it does not show that a passphrase is set at all: {shown}"
    );
}

#[test]
fn a_fingerprint_is_remembered_and_read_back() {
    let db = Db::open_in_memory().unwrap();
    let addr = ServerAddress::new("example.test", 22);

    assert_eq!(
        fingerprint::stored(&db, &addr).unwrap(),
        None,
        "a fingerprint came out of nowhere"
    );

    fingerprint::remember(&db, &addr, "SHA256:firstversion").unwrap();
    assert_eq!(
        fingerprint::stored(&db, &addr).unwrap().as_deref(),
        Some("SHA256:firstversion")
    );

    // Writing the same one again is not an error: repeating must be safe (principle V).
    fingerprint::remember(&db, &addr, "SHA256:firstversion").unwrap();
    assert_eq!(
        fingerprint::stored(&db, &addr).unwrap().as_deref(),
        Some("SHA256:firstversion")
    );

    // A deliberate replacement — the server was rebuilt, say, and a person confirmed it.
    fingerprint::remember(&db, &addr, "SHA256:secondversion").unwrap();
    assert_eq!(
        fingerprint::stored(&db, &addr).unwrap().as_deref(),
        Some("SHA256:secondversion")
    );

    fingerprint::forget(&db, &addr).unwrap();
    assert_eq!(fingerprint::stored(&db, &addr).unwrap(), None);
}

#[test]
fn a_fingerprint_is_tied_to_the_pair_of_address_and_port() {
    // One and the same host on different ports means different servers. Mixing their
    // fingerprints means either a false alarm or a substitution let through.
    let db = Db::open_in_memory().unwrap();
    let a22 = ServerAddress::new("example.test", 22);
    let a2222 = ServerAddress::new("example.test", 2222);

    fingerprint::remember(&db, &a22, "SHA256:the-key-of-twenty-two").unwrap();
    assert_eq!(fingerprint::stored(&db, &a2222).unwrap(), None);

    fingerprint::remember(&db, &a2222, "SHA256:the-key-of-the-other-port").unwrap();
    assert_eq!(
        fingerprint::stored(&db, &a22).unwrap().as_deref(),
        Some("SHA256:the-key-of-twenty-two"),
        "a write under another port overwrote a fingerprint that was not its own"
    );
}

// ---------- why a file operation failed (T071) ----------

#[test]
fn full_disk_is_not_reported_as_a_permission_problem() {
    // Every file trouble used to be reported as a permission problem with the hint "check
    // who owns the directory". On a full disk a person went to fix what was not broken while
    // the real cause lay in plain sight in the error text.
    use vrcast_studio_lib::commands::error::ErrorCode;
    use vrcast_studio_lib::ssh::SshError;

    let err = SshError::sftp("write failed: No space left on device");
    let app: vrcast_studio_lib::commands::error::AppError = err.into();
    // The code is the answer: from it the interface takes both the explanation and the
    // advice to free some room, in whichever language is chosen.
    assert_eq!(app.code, ErrorCode::RemoteDiskFull);
}

#[test]
fn each_file_failure_leads_to_its_own_answer() {
    use vrcast_studio_lib::commands::error::{AppError, ErrorCode};
    use vrcast_studio_lib::ssh::SshError;

    let cases = [
        ("Permission denied", ErrorCode::VideoDirDenied),
        ("No such file or directory", ErrorCode::FileMissingOnServer),
        ("connection reset by peer", ErrorCode::SshUnreachable),
        ("disk quota exceeded", ErrorCode::RemoteDiskFull),
    ];
    for (text, expected) in cases {
        let app: AppError = SshError::sftp(text).into();
        assert_eq!(app.code, expected, "recognised wrongly: {text}");
    }
}

#[test]
fn an_unfamiliar_failure_is_not_guessed_at() {
    // A wrong guess is worse than an honest "unknown": it sends people to fix the wrong
    // thing. The text must be kept, though — it can be searched for.
    use vrcast_studio_lib::commands::error::{AppError, ErrorCode};
    use vrcast_studio_lib::ssh::SshError;

    let app: AppError = SshError::sftp("SFTP status 4: something nobody has seen").into();
    assert_eq!(app.code, ErrorCode::Internal);
    assert!(
        app.cause
            .unwrap_or_default()
            .contains("something nobody has seen"),
        "the only text one can work it out from was lost"
    );
}
