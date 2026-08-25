//! T011 — the secret-leak test (SC-011, constitution, principle IV).
//!
//! What is checked is not that the substitution works on a string but that a secret **cannot
//! be found by searching** after it has gone down two real paths for a leak: through the log
//! and through an error message. That is how it happens in life — not "somebody printed the
//! password" but "a structure holding the password reached the debug output" or "a library
//! pasted it into an error text".

use std::io::Write;
use std::sync::{Arc, Mutex};
use vrcast_studio_lib::store::redact::{self, MASK};
use vrcast_studio_lib::store::secrets::{InMemorySecretStore, SecretRef, SecretStore};

/// The redaction registry is one per process, and cargo runs tests in parallel
/// threads of that process. Two tests each calling `forget_all` and then registering
/// their own secrets clobber one another: one wipes what the other has just put in,
/// and the loser reports a leak that never happened.
///
/// Caught 2026-08-25: the check failed about one run in five, always on a different
/// test. A flaky guard against secret leaks is worse than none — it teaches people to
/// re-run until it passes, which is exactly how a real leak would get waved through.
static REGISTRY: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Take the registry for the duration of a test.
///
/// A poisoned lock is taken anyway: it means some other test panicked, and that test
/// will report its own failure. Refusing to run here would bury it under a second one.
fn alone_with_registry() -> std::sync::MutexGuard<'static, ()> {
    let guard = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    redact::forget_all();
    guard
}

/// The shared buffer the log writes into during a test.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
    type Writer = Captured;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// The tests run in one process and share one list of secrets, so they must not overlap.
/// One shared lock is simpler and surer than isolating the state.
fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: Mutex<()> = Mutex::new(());
    L.lock().unwrap_or_else(|e| e.into_inner())
}

#[test]
fn a_secret_does_not_reach_the_log() {
    let _g = lock();
    let _registry = alone_with_registry();

    // The value is invented outright. Taking a real password as the sample — even a dead
    // one — will not do: the test goes into a public repository along with all its history,
    // and clearing it out afterwards costs noticeably more.
    let secret = "Zq7#vT2wR9pL4mX-an-invented-password-for-the-test";
    redact::register(secret);

    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(redact::RedactingMakeWriter::new(captured.clone()))
        .with_ansi(false)
        .finish();

    tracing::subscriber::with_default(subscriber, || {
        // The very case: not "the password was printed" but a structure holding it was.
        tracing::error!("could not connect, credentials used: user=root password={secret}");
        tracing::info!(
            field = secret,
            "an event's field goes through redaction too"
        );
    });

    let out = captured.text();
    assert!(
        !out.is_empty(),
        "the log is empty — the test checked nothing"
    );
    assert!(
        !out.contains(secret),
        "THE SECRET WAS FOUND IN THE LOG:\n{out}"
    );
    assert!(out.contains(MASK), "the substitution did not fire:\n{out}");
}

#[test]
fn a_secret_does_not_reach_an_error_message() {
    let _g = lock();
    let _registry = alone_with_registry();

    let secret = "a-key-passphrase-9f3a2b";
    redact::register(secret);

    // An error from an underlying library, which knows nothing of our rules.
    #[derive(Debug, thiserror::Error)]
    #[error("authentication failed (the password {0} was passed)")]
    struct ChattyError(String);

    let err = ChattyError(secret.to_owned());

    let raw = err.to_string();
    assert!(
        raw.contains(secret),
        "the test is built wrong: the secret is not in the original error"
    );

    let safe = redact::safe_display(&err);
    assert!(
        !safe.contains(secret),
        "THE SECRET WAS FOUND IN THE ERROR: {safe}"
    );
    assert!(safe.contains(MASK));
}

#[test]
fn a_secret_from_the_store_registers_itself() {
    let _g = lock();
    let _registry = alone_with_registry();

    // The key property: the calling code does NOT register the secret by hand. It is enough
    // that the secret passed through the store.
    let store = InMemorySecretStore::new();
    let reference = SecretRef::for_server("srv_test");
    let secret = "a-password-nobody-registered";

    store.set(&reference, secret).unwrap();

    let leaked = format!("debug: {secret}");
    assert!(
        !redact::redact(&leaked).contains(secret),
        "a secret that passed through the store was not registered automatically"
    );
}

#[test]
fn a_private_key_is_cut_out_even_without_being_registered() {
    let _g = lock();
    let _registry = alone_with_registry();

    // The safety net for when a key was read from a file and reached the output past the
    // store — that is, in exactly the case where registration did not happen.
    let text = "key read:\n\
        -----BEGIN OPENSSH PRIVATE KEY-----\n\
        b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAAB\n\
        -----END OPENSSH PRIVATE KEY-----\nend";

    let out = redact::redact(text);
    assert!(
        !out.contains("b3BlbnNzaC1rZXktdjEA"),
        "the key's body was left behind: {out}"
    );
    assert!(
        !out.contains("BEGIN OPENSSH"),
        "the key's header was left behind: {out}"
    );
    assert!(
        out.contains("key read") && out.contains("end"),
        "too much was cut out: {out}"
    );
}

#[test]
fn a_truncated_key_is_cut_out_whole_as_well() {
    let _g = lock();
    let _registry = alone_with_registry();

    // The log may break off in the middle of a block. Better to lose the tail than to show
    // half a key.
    let text = "start\n-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXk";
    let out = redact::redact(text);
    assert!(
        !out.contains("b3BlbnNzaC1rZXk"),
        "half a key was left behind: {out}"
    );
    assert!(out.contains("start"));
}

#[test]
fn a_multi_line_key_does_not_slip_through_line_by_line_writing() {
    let _g = lock();
    let _registry = alone_with_registry();

    // The defect this test exists for: the log writes an event as one piece with newlines
    // inside it, while the writer cut it up line by line and masked only the line holding
    // "-----BEGIN". The key's body — lines of plain base64 — went into the log whole: a line
    // on its own holds neither a block marker nor a registered secret.
    let captured = Captured::default();
    {
        let mut w = redact::RedactingWriter::new(captured.clone());
        w.write_all(
            "key read:\n\
             -----BEGIN OPENSSH PRIVATE KEY-----\n\
             b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAAB\n\
             QyNTUxOQAAACDLnp6PPO2Fk3rCC1aXY0P3zXNZDCLYPFHzVv1cnJ2d\n\
             -----END OPENSSH PRIVATE KEY-----\n\
             end\n"
                .as_bytes(),
        )
        .unwrap();
        w.flush().unwrap();
    }

    let out = captured.text();
    assert!(
        !out.contains("b3BlbnNzaC1rZXktdjEA") && !out.contains("QyNTUxOQ"),
        "THE KEY'S BODY REACHED THE LOG: {out}"
    );
    assert!(
        out.contains("key read") && out.contains("end"),
        "too much was cut out: {out}"
    );
    assert!(out.contains(MASK), "the substitution did not fire: {out}");
}

#[test]
fn a_key_arriving_in_separate_writes_is_cut_out_too() {
    let _g = lock();
    let _registry = alone_with_registry();

    // The same defect, another path: each line of the key arrives in a write of its own. The
    // writer has to gather a block that has begun rather than letting the lines out one by
    // one.
    let captured = Captured::default();
    {
        let mut w = redact::RedactingWriter::new(captured.clone());
        for part in [
            "-----BEGIN OPENSSH PRIVATE KEY-----\n",
            "b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAAB\n",
            "-----END OPENSSH PRIVATE KEY-----\n",
            "an ordinary log line after it\n",
        ] {
            w.write_all(part.as_bytes()).unwrap();
        }
        w.flush().unwrap();
    }

    let out = captured.text();
    assert!(
        !out.contains("b3BlbnNzaC1rZXktdjEA"),
        "THE KEY'S BODY REACHED THE LOG: {out}"
    );
    assert!(
        out.contains("an ordinary log line after it"),
        "too much was cut out: {out}"
    );
    assert!(out.contains(MASK));
}

#[test]
fn a_value_that_is_too_short_is_not_registered() {
    let _g = lock();
    let _registry = alone_with_registry();

    // A three-character secret would turn up in a log by chance hundreds of times and riddle
    // it with masks, hiding everything useful along with it. Such "protection" does harm.
    redact::register("abc");
    let text = "abcdefghij — an ordinary text that holds abc";
    assert_eq!(
        redact::redact(text),
        text,
        "a short value was registered and spoilt the text"
    );
}

#[test]
fn of_two_nested_secrets_the_long_one_is_cut_out() {
    let _g = lock();
    let _registry = alone_with_registry();

    // Replace the short one first and a tail of the long one is left — and the tail of a
    // password is still a leak.
    let short = "password-part-1";
    let long = "password-part-1-and-the-rest-2";
    redact::register(short);
    redact::register(long);

    let input = format!("value: {long}");
    let out = redact::redact(&input);
    assert!(
        !out.contains("the-rest-2"),
        "a tail of the long secret was left behind: {out}"
    );
    assert_eq!(out, format!("value: {MASK}"));
}

#[test]
fn writing_in_pieces_does_not_let_a_torn_secret_through() {
    let _g = lock();
    let _registry = alone_with_registry();

    // The value is deliberately not ASCII: the halves below fall on either side of
    // multi-byte characters, which is what makes the buffering's UTF-8 handling matter.
    let secret = "секрет-разорванный-между-записями";
    redact::register(secret);

    let captured = Captured::default();
    {
        let mut w = redact::RedactingWriter::new(captured.clone());
        // An output stream can arrive in pieces, and a secret can be cut in half.
        w.write_all("start секрет-разорван".as_bytes()).unwrap();
        w.write_all("ный-между-записями end\n".as_bytes()).unwrap();
        w.flush().unwrap();
    }

    let out = captured.text();
    assert!(
        !out.contains(secret),
        "THE SECRET WENT THROUGH IN PIECES: {out}"
    );
    assert!(out.contains(MASK), "the substitution did not fire: {out}");
}

#[test]
fn forgetting_one_secret_leaves_the_others_masked() {
    // T073, principle IV. Deleting one profile must not take the masking off the others'
    // secrets: they are alive, and the very next error would carry someone else's password
    // into the log — until the application restarts, and with no sign that anything is wrong.
    let _registry = alone_with_registry();

    let deleted = "the-deleted-profile-s-password-1234";
    let kept = "the-other-profile-s-password-5678";
    redact::register(deleted);
    redact::register(kept);

    redact::forget(deleted);

    let text = format!("the login failed: {deleted} and {kept}");
    let redacted = redact::redact(&text);

    assert!(
        !redacted.contains(kept),
        "a live profile's secret stopped being cut out: {redacted}"
    );
    // The forgotten one is no longer cut out — which is what was asked: it is out of the
    // store already.
    assert!(redacted.contains(deleted));
}

#[test]
fn deleting_a_profile_unmasks_only_its_own_secret() {
    // The same trouble, but checked through a real store: what matters is not that `forget`
    // works but that deleting a profile calls exactly it.
    use vrcast_studio_lib::store::secrets::{InMemorySecretStore, SecretRef, SecretStore};

    let _registry = alone_with_registry();
    let store = InMemorySecretStore::new();

    let first = SecretRef::for_server("profile-one");
    let second = SecretRef::for_server("profile-two");
    store
        .set(&first, "the-first-profile-s-secret-1234")
        .unwrap();
    store
        .set(&second, "the-second-profile-s-secret-5678")
        .unwrap();

    store.delete(&first).unwrap();

    let redacted = redact::redact(
        "in the log: the-first-profile-s-secret-1234 and the-second-profile-s-secret-5678",
    );
    assert!(
        !redacted.contains("the-second-profile-s-secret-5678"),
        "deleting one profile took the masking off the second: {redacted}"
    );
}
