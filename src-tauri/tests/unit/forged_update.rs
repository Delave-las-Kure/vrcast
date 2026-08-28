//! T354 — an update signed by another key is refused by the key this application ships.
//!
//! **A signing mechanism nobody has tried to fool is a mechanism about which only one thing is
//! known: that it does not get in the way.** So a key that is not ours signed a payload, and
//! that forgery is offered here to the public key the released application actually carries.
//!
//! **What this proves, exactly.** The key in `tauri.conf.json` — read from the file, not copied
//! here — refuses a signature made by another key, and accepts the one made for it. The steps
//! are the plugin's own, in the same order and with the same crate: base64-decode the key,
//! `PublicKey::decode`, base64-decode the signature, `Signature::decode`, `verify`. Compared
//! against `tauri-plugin-updater` 2.10.1, `verify_signature`, on 2026-08-28.
//!
//! **What it does not prove: that the plugin calls this at all.** That would take fooling the
//! running application, and the way to do that — `tauri`'s `test` feature and a mock runtime —
//! was tried on 2026-08-28 and abandoned for a measured reason: it adds an import the test
//! binary cannot resolve on Windows, and the **whole** unit suite, all four hundred and
//! thirty-five of them, stopped starting at all with `STATUS_ENTRYPOINT_NOT_FOUND` before the
//! first line ran. Putting `WebView2Loader.dll` beside the binary did not help. One case won
//! is not worth four hundred and thirty-five lost, and the part that is ours — which key we
//! trust — is the part checked here. The verifying itself is Tauri's code, and the release
//! path exercises it end to end the first time a real update is offered.
//!
//! **The forgery is committed; the key that made it is not.** A throwaway pair was made on
//! 2026-08-28, signed `payload.bin`, and its private half was deleted. What is in the
//! repository is the payload, the signature over it, and the foreign *public* key — enough to
//! see what signed the thing, and not enough to sign anything else.

use base64::Engine;
use minisign_verify::{PublicKey, Signature};
use serde_json::Value;

/// The very settings the application ships with.
const CONF: &str = include_str!("../../tauri.conf.json");
/// A payload standing in for an installer, and the signature a foreign key made over it.
const PAYLOAD: &[u8] = include_bytes!("../fixtures/updater/payload.bin");
const FORGED_SIG: &str = include_str!("../fixtures/updater/payload.bin.sig");
/// The public half of the key that made that signature. Here so that what signed the fixture
/// can be seen rather than taken on trust.
const FOREIGN_PUBKEY: &str = include_str!("../fixtures/updater/foreign.key.pub");

/// Our own public key, exactly as the application will carry it.
fn our_pubkey() -> String {
    let conf: Value = serde_json::from_str(CONF).expect("tauri.conf.json is not valid JSON");
    conf.pointer("/plugins/updater/pubkey")
        .and_then(Value::as_str)
        .expect("there is no public key in the settings")
        .to_owned()
}

/// The plugin's own steps, in the plugin's own order.
///
/// Both the key and the signature travel base64-encoded — that is how they sit in the settings
/// and in `latest.json`, and how the bundler writes a `.sig` file — so the decoding is part of
/// the path and is done here too.
fn verifies(pubkey_base64: &str, signature_base64: &str, data: &[u8]) -> Result<(), String> {
    let engine = base64::engine::general_purpose::STANDARD;
    let key_text = engine
        .decode(pubkey_base64.trim())
        .map_err(|e| format!("the key is not base64: {e}"))?;
    let key = PublicKey::decode(
        std::str::from_utf8(&key_text).map_err(|e| format!("the key is not text: {e}"))?,
    )
    .map_err(|e| format!("the key would not decode: {e}"))?;

    // The `.sig` file the bundler writes is **already** base64 — that is why its contents go
    // straight into `latest.json` as the signature field, and why the plugin decodes once. An
    // earlier draft of this encoded it again, and minisign rejected the result before looking
    // at anything: a refusal that would have passed for the one this test is about.
    let sig_text = engine
        .decode(signature_base64.trim())
        .map_err(|e| format!("the signature is not base64: {e}"))?;
    let signature = Signature::decode(
        std::str::from_utf8(&sig_text).map_err(|e| format!("the signature is not text: {e}"))?,
    )
    .map_err(|e| format!("the signature would not decode: {e}"))?;

    key.verify(data, &signature, true)
        .map_err(|e| format!("refused: {e}"))
}

#[test]
fn a_signature_made_by_another_key_is_refused() {
    let refusal = verifies(&our_pubkey(), FORGED_SIG, PAYLOAD).expect_err(
        "a payload signed by a key that is not ours was accepted. Every released copy verifies \
         updates with this key, and this says it would let somebody else's update through",
    );
    assert!(
        refusal.starts_with("refused:"),
        "it was refused, but before the signature was even looked at: {refusal}"
    );
}

#[test]
fn the_same_signature_is_good_for_the_key_that_made_it() {
    // **Without this the test above proves nothing.** An empty fixture, a truncated one, or a
    // signature over different bytes would all be "refused" too — and the refusal would be
    // about the fixture being broken rather than about the key being wrong. Here the very same
    // signature and the very same bytes are accepted by the key that signed them, so the
    // refusal above can only be the mismatch it is meant to be.
    verifies(FOREIGN_PUBKEY, FORGED_SIG, PAYLOAD)
        .expect("the fixture does not verify against the key that made it, so it is broken");
}

#[test]
fn the_key_that_signed_the_fixture_is_not_ours() {
    // The day somebody regenerates the fixture with the production key, the two tests above
    // would still pass — one of them for the wrong reason, and nothing would say so.
    assert_ne!(
        FOREIGN_PUBKEY.trim(),
        our_pubkey().trim(),
        "the fixture was signed with our own key, so nothing above is being forged"
    );
}

#[test]
fn a_payload_the_signature_was_not_made_for_is_refused() {
    // The other half of what a signature is for. A mechanism that checked the key and not the
    // bytes would pass every test above and still let a swapped installer through.
    let tampered = {
        let mut bytes = PAYLOAD.to_vec();
        bytes.push(b'!');
        bytes
    };
    verifies(FOREIGN_PUBKEY, FORGED_SIG, &tampered)
        .expect_err("bytes that were never signed were accepted by the signature over others");
}
