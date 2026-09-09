//! T503 — a stop that arrives in the last phase must actually stop something.
//!
//! **What went wrong.** `finish` — the phase after the bytes are across — asked about
//! cancellation nowhere at all. It computes the checksum of the whole file, which on thirty
//! gigabytes is minutes, compares it with the server's, and renames the file into serving.
//! Pressing stop through any of that did nothing: the work ran to its end, the film entered
//! serving, and the engine then wrote the task down as `Cancelled` — because a cancellation
//! outweighs the outcome, which is right for a failure and a lie about a success. A person saw
//! "cancelled" beside a film that was being served.
//!
//! **Why this is a source check and not a behavioural one.** The window is a race by nature:
//! to hit it a test has to cancel inside a checksum that finishes in milliseconds on any file
//! small enough to put in a fixture, and on a file large enough to be slow the test costs
//! minutes and still races. A check that can only pass is worse than no check. `spawn_hygiene`
//! makes the same trade for the same reason, and states it the same way: what cannot be seen
//! from a test is guarded at the source, where the property is exactly stateable — before each
//! irreversible act, ask.
//!
//! **The unclosable half is not hidden.** Between the last check and the rename finishing
//! there is a moment, and a stop arriving in it is real. Nothing in this function can undo it:
//! un-publishing would delete a file somebody may already have started watching, which is not
//! what "stop the upload" asked for. So the task says so — a notice beside the cancelled row,
//! which is what makes "cancelled" and "it is being served" agree instead of contradict.
//!
//! The other half of cancelling an upload — the wait between attempts, which used to drop the
//! cancellation on the floor — is guarded in `spawn_hygiene.rs` (T521), beside the rule about
//! what the type system could not refuse.
//!
//! **T570 moved the checksum comparison and the publish call out of `finish` itself**, into
//! `finish_once` — a single attempt, retried through `checksum_and_publish`'s own reconnect
//! loop when the connection breaks partway through this phase. The property this file guards
//! did not move with them by accident: it moved because the two are now different functions,
//! and the checks below follow the calls to wherever they actually live.

use std::path::{Path, PathBuf};

fn upload_rs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/upload.rs")
}

/// The body of `async fn NAME(`, from its signature to the closing brace of the function.
///
/// Braces are counted rather than matched with a pattern: a body holds nested blocks, and a
/// reader that stopped at the first `}` would check the first few lines while looking
/// thorough — the failure with no symptom, which is the one this file exists to avoid.
fn function_body(text: &str, signature: &str) -> String {
    let at = text.find(signature).unwrap_or_else(|| {
        panic!("`{signature}` is gone from upload.rs — if it was renamed, rename it here too")
    });
    let open = text[at..].find('{').expect("the function has no body") + at;
    let mut depth = 0;
    for (i, c) in text[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return text[open..open + i].to_owned();
                }
            }
            _ => {}
        }
    }
    panic!("`{signature}` is never closed")
}

#[test]
fn the_last_phase_asks_about_stopping_before_every_step_it_cannot_take_back() {
    let text = std::fs::read_to_string(upload_rs()).expect("could not read upload.rs");
    let finish_body = function_body(&text, "async fn finish(");
    let finish_once_body = function_body(&text, "async fn finish_once(");

    // The reader must have found the real thing, or the checks below pass over a fragment.
    assert!(
        finish_body.contains("checksum::local"),
        "`checksum::local` is not in the body of `finish` that was read — the reader found \
         the wrong span, and the checks below would mean nothing"
    );
    for landmark in ["checksum::matches", "upload::publish("] {
        assert!(
            finish_once_body.contains(landmark),
            "`{landmark}` is not in the body of `finish_once` that was read — T570 put the \
             checksum comparison and the publish call there, and if that has moved again this \
             check has to follow it"
        );
    }

    // Before the checksum work starts (`finish` itself, ahead of the retry loop): a large
    // file's checksum is minutes of work, and pressing stop through it must not be ignored.
    let asks_in_finish: Vec<usize> = finish_body
        .match_indices("ctx.is_cancelled()")
        .map(|(i, _)| i)
        .collect();
    let checksum_at = finish_body
        .find("checksum::local")
        .expect("landmark checked above");
    assert!(
        asks_in_finish.iter().any(|&at| at < checksum_at),
        "nothing in `finish` asks about stopping before the checksum, and on a large file \
         that is minutes of work a person has already asked to end"
    );

    // Between the comparison and the rename into serving — the last moment a stop can still
    // be obeyed cleanly. This is `finish_once` now, one attempt of the reconnecting loop
    // rather than the single pass `finish` used to run directly.
    let asks_in_finish_once: Vec<usize> = finish_once_body
        .match_indices("ctx.is_cancelled()")
        .map(|(i, _)| i)
        .collect();
    let compared_at = finish_once_body
        .find("checksum::matches")
        .expect("landmark checked above");
    let publish_at = finish_once_body
        .find("upload::publish(")
        .expect("landmark checked above");
    assert!(
        asks_in_finish_once
            .iter()
            .any(|&at| at > compared_at && at < publish_at),
        "nothing in `finish_once` asks about stopping between the comparison and the rename \
         into serving — the last moment a stop can still be obeyed"
    );
}

#[test]
fn the_moment_that_cannot_be_taken_back_is_said_out_loud() {
    let text = std::fs::read_to_string(upload_rs()).expect("could not read upload.rs");
    let finish_body = function_body(&text, "async fn finish(");

    // T570: the publish call itself now lives in `finish_once`, one attempt inside
    // `checksum_and_publish`'s reconnect loop — but the notice is said in `finish`, once the
    // loop as a whole has confirmed the file is published, on the ordinary path and on the
    // reconnected one alike. So what is checked here is that `finish` still asks and still
    // says, after the point where publishing (in whichever attempt actually did it) is known
    // to have succeeded.
    let outcome_known_at = finish_body
        .find("checksum_and_publish(")
        .expect("`finish` no longer calls checksum_and_publish — T570's reconnect loop is gone");
    let notice_at = finish_body.find("NoticeCancelledAfterPublish");

    assert!(
        notice_at.is_some_and(|at| at > outcome_known_at),
        "a stop arriving while the file was entering serving leaves the task marked cancelled \
         and the file being served, and nothing says so. Both are true and neither alone tells \
         a person what is on the server — the notice is what makes them agree."
    );
}
