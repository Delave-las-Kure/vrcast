//! Principle II — nothing unchecked reaches viewers.
//!
//! **The rule, in the constitution's own terms**: what a preparation produces must be checked
//! for playability before it is sent, and a file that did not pass must not be offered
//! (FR-027). It is one of the principles the project marks as not up for discussion, and it
//! was kept in one place out of two.
//!
//! ⚠ **The half that was missing** (T499). `media::validate` was called from the single-file
//! preparation and from a command no screen calls. The ladder build — the path a whole season
//! goes down, and the one whose output a player switches to without asking anybody — encoded
//! each rung with the same call and sent it on the strength of ffmpeg having exited zero. A
//! variant is not a lesser file. It is precisely what a viewer is served.
//!
//! **Why a source check.** Producing a file that encodes cleanly and decodes badly needs a
//! broken encoder or a corrupted disk; a test cannot arrange one honestly, and a fixture that
//! pretended to would be checking the fixture. What can be stated exactly is the shape: every
//! path that sends a prepared file to a server decodes it first, and the decode stands between
//! the encode and the send rather than anywhere else. `spawn_hygiene` makes the same trade and
//! gives the same reason.

use std::path::{Path, PathBuf};

/// The paths that hand a locally prepared file to a server, and where each does its decode.
///
/// Named rather than discovered, because "sends a file to a server" is not a shape a scan can
/// recognise — and a list that is wrong is a check that passes over nothing. What keeps the
/// list honest is `every_named_path_is_still_there` below: an entry naming a function that has
/// gone fails, so the list cannot quietly stop describing the code.
const SENDS_A_PREPARED_FILE: &[(&str, &str, &str)] = &[
    (
        "src/commands/convert.rs",
        "validate::validate(",
        "Preparing one file. The verdict is what decides whether it may be offered for upload \
         at all, and it has been here since FR-027 was written.",
    ),
    (
        "src/tasks/ladder_build.rs",
        "validate::validate(",
        "Building a quality set. Every rung is a file a viewer is served, and until 2026-09-07 \
         none of them was decoded — the set went to the server because the encoder had exited \
         zero (T499).",
    ),
];

fn core(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn text_of(rel: &str) -> String {
    let path = core(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("could not read {rel}: {e}"))
}

#[test]
fn every_path_that_sends_a_prepared_file_decodes_it_first() {
    for (rel, needle, why) in SENDS_A_PREPARED_FILE {
        let text = text_of(rel);
        assert!(
            text.contains(needle),
            "{rel} sends a prepared file to a server and never decodes it. {why}\n\
             Principle II is not one of the negotiable ones: what a preparation produces is \
             checked before it is sent, and what did not pass is not offered."
        );
    }
}

/// In the ladder build, the decode stands **between** the encode and the send.
///
/// Order is the whole of it: decoding after the send checks a file that is already being
/// served, which is the thing the principle forbids in the same sentence it asks for the
/// check.
#[test]
fn the_ladder_decodes_a_variant_before_it_sends_it() {
    let text = text_of("src/tasks/ladder_build.rs");
    let encoded = text
        .find("convert::run(&convert, ctx)")
        .expect("the ladder build no longer encodes a variant — if it moved, move this too");
    let decoded = text
        .find("validate::validate(&out_path)")
        .expect("the ladder build no longer decodes a variant (T499)");
    let sent = text
        .find("send(job, &out_path, &variant.file)")
        .expect("the ladder build no longer sends a variant");

    assert!(
        encoded < decoded && decoded < sent,
        "the decode is not between the encode and the send, so a rung reaches the server \
         before anything has read it back"
    );

    // And the verdict is acted on. A decode whose answer is thrown away is the same as no
    // decode, and costs the time as well.
    let between = &text[decoded..sent];
    assert!(
        between.contains("if !verdict.ok"),
        "the variant is decoded and the answer is not looked at, which is a decode nobody \
         asked for and a rung nobody checked"
    );
}

#[test]
fn every_named_path_is_still_there() {
    for (rel, _, _) in SENDS_A_PREPARED_FILE {
        assert!(
            core(rel).is_file(),
            "{rel} is named here as a path that sends prepared files and is not in the core \
             any more — either it moved, and this must follow it, or the list has stopped \
             describing the application"
        );
    }
}
