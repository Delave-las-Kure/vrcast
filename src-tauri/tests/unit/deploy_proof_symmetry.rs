//! T565 — `key_works` and `password_refused` close their connection the same way on success.
//!
//! **What was asymmetric.** Both proof closures in `commands::deploy` open a `Connection` of
//! their own so neither one signs in through the connection the deployment task is already
//! holding (T274's own reasoning: that one keeps working whatever the settings say, so it
//! would be the wrong witness). `password_refused` (via `passwords_are_off`) has always
//! closed its connection explicitly on success, sending a polite SSH disconnect. `key_works`
//! used to fold the connect straight into `.is_ok()`, discarding the `Connection` and leaving
//! the close to `Drop` — an implicit TCP close with no SSH disconnect message. Not a
//! correctness bug (nothing leaks; the socket closes either way) but an inconsistency between
//! two closures built for exactly the same purpose, called up to six times over one
//! deployment run (`SshKey` and `SshHardening` both call both proofs).
//!
//! **Why a source check and not a call through the real closures.** Both proofs are built
//! inline, deep inside `server_deploy`'s `state.tasks.submit(move |task| async move { ... })`
//! — not free functions, not `pub`, not reachable from a test without either raising the
//! whole task engine and a real SSH server for a one-line ordering question, or reworking the
//! production code's shape to make it testable purely to serve this test. Reading whether the
//! text of `key_works` really does what `password_refused` does is the direct question this
//! task asks, and it is the same choice this project already made for
//! `the_account_is_installed_beside_the_sweep_that_reads_it` (`registry.rs`) and
//! `purge_finished_before_is_called_at_startup` (`task_purge.rs`, T564).

use std::fs;
use std::path::Path;

fn deploy_source() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/deploy.rs");
    fs::read_to_string(&path).expect("could not read commands/deploy.rs")
}

/// The text of the `key_works` closure alone — from its own `let key_works = {` up to the
/// `let password_refused = {` that starts the next one. Bounded on both sides so a match
/// elsewhere in this 800-line file (there is a second, unrelated `key_works` in the
/// integration test fixtures) cannot be mistaken for this one.
fn key_works_closure_text(source: &str) -> &str {
    let start = source
        .find("let key_works = {")
        .expect("commands/deploy.rs no longer defines key_works where this test expects it");
    let after_start = &source[start..];
    let end = after_start
        .find("let password_refused = {")
        .expect("password_refused no longer follows key_works — re-check the bound");
    &after_start[..end]
}

#[test]
fn key_works_closes_its_connection_on_success_like_password_refused_does() {
    let source = deploy_source();
    let key_works = key_works_closure_text(&source);

    assert!(
        key_works.contains(".close().await"),
        "key_works no longer closes its connection explicitly on success — it went back to \
         relying on Drop, unlike password_refused/passwords_are_off (T565): {key_works}"
    );

    // Not just present anywhere in the closure — actually on the success branch of the
    // connect this closure makes, mirroring passwords_are_off's own `Ok(conn) => { conn.
    // close().await; ... }` shape. A `close()` call site elsewhere in the closure (there is
    // none today) would satisfy the weaker check above without answering the real question.
    assert!(
        key_works.contains("Ok(conn) =>") && key_works.contains("conn.close().await"),
        "key_works does not close on the Ok branch of its own connect in the same shape as \
         passwords_are_off: {key_works}"
    );
}

#[test]
fn password_refused_still_closes_on_success_as_the_reference_shape() {
    // The negative-control side of the same claim: if THIS one stopped closing, the test
    // above would still pass trivially (nothing left to be symmetric with) and silently stop
    // meaning anything. Checked directly, once, so a regression on this side is caught too.
    let source = deploy_source();
    let start = source
        .find("async fn passwords_are_off")
        .expect("passwords_are_off no longer exists where this test expects it");
    let text = &source[start..];
    let end = text
        .find("\npub mod ipc")
        .unwrap_or_else(|| text.len().min(2000));
    let passwords_are_off = &text[..end];

    assert!(
        passwords_are_off.contains("conn.close().await"),
        "passwords_are_off no longer closes its connection on success — the shape key_works \
         was brought in line with has itself regressed: {passwords_are_off}"
    );
}
