//! T290a — the key the application makes for itself.
//!
//! Nothing here needs a server: what has to be right is that the two halves are a pair, that
//! the private one reads back, and that the public one is a line OpenSSH would accept in
//! `authorized_keys`. A key that looked fine and did not open the server would be found out
//! at the worst moment — the step after the one that turns the password off.

use vrcast_studio_lib::ssh::auth::load_key_text;
use vrcast_studio_lib::ssh::keygen;

#[test]
fn the_key_reads_back_and_its_halves_are_a_pair() {
    let made = keygen::make("vrcast-studio: test").expect("no key was made");

    // The private half goes into the store and comes back out as text. If it will not read,
    // the application has locked itself out of a server it is about to harden.
    let read = load_key_text(&made.private_openssh, None).expect("the key would not read back");

    // And the half we put on the server has to belong to the half we kept.
    let ours = read.public_key().to_openssh().expect("no public half");
    assert_eq!(
        ours.split_whitespace().take(2).collect::<Vec<_>>(),
        made.public_openssh
            .split_whitespace()
            .take(2)
            .collect::<Vec<_>>(),
        "the two halves are not a pair"
    );
}

#[test]
fn the_public_half_is_a_line_authorized_keys_would_take() {
    let made = keygen::make("vrcast-studio: server one").expect("no key was made");

    let mut parts = made.public_openssh.split_whitespace();
    assert_eq!(
        parts.next(),
        Some("ssh-ed25519"),
        "the wrong kind of key: {}",
        made.public_openssh
    );
    let body = parts.next().expect("no key body");
    assert!(body.len() > 40, "the body is too short to be a key: {body}");
    // The comment is the only thing on that line that says where the key came from. Without
    // it, a person looking at their own server's authorized_keys sees a stray key.
    assert_eq!(parts.next(), Some("vrcast-studio:"));

    assert!(
        !made.public_openssh.contains('\n'),
        "a public key with a newline in it would break the file it is appended to"
    );
}

#[test]
fn two_keys_are_two_keys() {
    // Not a formality: a generator that returned the same key every time would work
    // perfectly, on one server, and quietly give every other server the same way in.
    let one = keygen::make("vrcast-studio: a").expect("no key");
    let two = keygen::make("vrcast-studio: b").expect("no key");
    assert_ne!(one.public_openssh, two.public_openssh);
    assert_ne!(one.private_openssh, two.private_openssh);
}

#[test]
fn the_private_half_never_reaches_a_message() {
    // It is registered for redaction the moment it exists (constitution, principle IV). The
    // way a secret escapes is somebody's `{:?}` on a structure that happens to hold it, and
    // that is not something a comment prevents.
    let made = keygen::make("vrcast-studio: hidden").expect("no key");
    let pretend_error = format!("something went wrong: {}", made.private_openssh);
    let cleaned = vrcast_studio_lib::store::redact::redact(&pretend_error);
    assert!(
        !cleaned.contains("PRIVATE KEY"),
        "the private key came through a message untouched"
    );
}
