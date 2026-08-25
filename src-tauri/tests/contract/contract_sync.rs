//! Comparing the contract between the core and the interface.
//!
//! The Rust core and its TypeScript reflection are two descriptions of one contract. They
//! drift apart quietly: the code builds, the types check, and an error handler in the
//! interface simply never fires because it waits for a code that no longer exists. It comes
//! to light at a person's machine, the moment the error finally happens.
//!
//! So the drift is caught here, at build time. Two rules, drawn from the review of
//! 2026-08-25:
//!
//! 1. Every comparison goes BOTH WAYS. A one-way one catches "in the core, missing in TS"
//!    but lets through the surplus in TS — a handler for an event the core never sends.
//! 2. Values are looked for only INSIDE the declaration that was parsed, not across the
//!    whole file: a `contains` over the file would count a code left in a comment or in
//!    somebody else's type.
//!
//! The Rust-side lists come from the `ALL` constants, born of the same macro as the enums
//! themselves — there is no hand-written list left that could fall behind.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use vrcast_studio_lib::commands::error::{DetailCode, ErrorCode};
use vrcast_studio_lib::tasks::state::{TaskKind, TaskState};

fn frontend_file(rel: &str) -> PathBuf {
    // The core lives in src-tauri/, the interface beside it, in src/.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri has no parent directory")
        .join(rel)
}

fn contract_ts() -> String {
    let path = frontend_file("src/shared/contract.ts");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()))
}

/// Every string literal from the declaration that begins with `marker` and is closed by `;`.
///
/// Comments are thrown away line by line BEFORE the quotes and the semicolon are looked
/// for: a quote in a comment must not widen the list, and a `;` in one must not cut the
/// block short. (Contract values never hold either `//` or `;` — that is what the parsing
/// leans on.)
fn declared_strings(ts: &str, marker: &str) -> HashSet<String> {
    let start = ts
        .find(marker)
        .unwrap_or_else(|| panic!("contract.ts has no declaration \"{marker}\""));
    let body = &ts[start + marker.len()..];

    let mut clean = String::new();
    let mut closed = false;
    for line in body.lines() {
        let line = line.split("//").next().unwrap_or("");
        if let Some(i) = line.find(';') {
            clean.push_str(&line[..i]);
            closed = true;
            break;
        }
        clean.push_str(line);
        clean.push('\n');
    }
    assert!(
        closed,
        "the declaration \"{marker}\" is not closed by a semicolon"
    );

    clean
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

/// A two-way comparison of a list, reporting each direction understandably.
fn assert_same_sets(what: &str, rust: HashSet<String>, ts: HashSet<String>) {
    let mut missing: Vec<_> = rust.difference(&ts).collect();
    let mut extra: Vec<_> = ts.difference(&rust).collect();
    missing.sort();
    extra.sort();

    assert!(
        missing.is_empty() && extra.is_empty(),
        "{what}: the contract has drifted.\n\
         In the core, missing from contract.ts: {missing:?}\n\
         In contract.ts, but the core never sends it: {extra:?}"
    );
}

#[test]
fn the_error_codes_match_both_ways() {
    let rust: HashSet<String> = ErrorCode::ALL
        .iter()
        .map(|c| c.as_str().to_owned())
        .collect();
    let ts = declared_strings(&contract_ts(), "export type ErrorCode =");
    assert_same_sets("error codes", rust, ts);
}

#[test]
fn the_detail_codes_match_both_ways() {
    // The details arrived along with the two languages: the core stopped composing sentences
    // and now names the case with a code while the interface picks the wording. A code
    // forgotten here is an empty space on the screen instead of an explanation, and it would
    // come to light at a person's machine.
    //
    // The completeness of the catalogues themselves is checked by the TypeScript compiler:
    // they are declared as `Record<DetailCode, ...>`, and a missing key fails the interface
    // build. What is compared here is the link before that — that the list of codes in TS is
    // the same one at all.
    let rust: HashSet<String> = DetailCode::ALL
        .iter()
        .map(|c| c.as_str().to_owned())
        .collect();
    let ts = declared_strings(&contract_ts(), "export type DetailCode =");
    assert_same_sets("detail codes", rust, ts);
}

#[test]
fn the_task_kinds_match_both_ways() {
    let rust: HashSet<String> = TaskKind::ALL
        .iter()
        .map(|k| k.as_str().to_owned())
        .collect();
    let ts = declared_strings(&contract_ts(), "export type TaskKind =");
    assert_same_sets("task kinds", rust, ts);
}

#[test]
fn the_task_states_match_both_ways() {
    let rust: HashSet<String> = TaskState::ALL
        .iter()
        .map(|s| s.as_str().to_owned())
        .collect();
    let ts = declared_strings(&contract_ts(), "export type TaskState =");
    assert_same_sets("task states", rust, ts);
}

#[test]
fn the_event_names_match_both_ways() {
    use vrcast_studio_lib::commands::events::names;

    // The list of names here is hand-written: the names module has no ALL of its own. A
    // name forgotten here is caught from the other side — a surplus value in EVENTS.
    let rust: HashSet<String> = [
        names::TASK_PROGRESS,
        names::TASK_DONE,
        names::TASK_NOTIFY,
        names::LIBRARY_CHANGED,
        names::SERVER_STATE,
        names::VIEWERS_UPDATE,
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();

    let ts = declared_strings(&contract_ts(), "export const EVENTS = {");
    assert_same_sets("event names", rust, ts);
}

// ---------- comparing SHAPES, not only lists (T075) ----------

/// The field names of an interface declared in TypeScript.
///
/// The parsing is deliberately simple and leans on how this file is written: one field per
/// line, `name: type;`. A full TypeScript parse would be a tool out of proportion here — and
/// should the file start being written differently, the comparison fails honestly rather
/// than pretending everything matched.
fn declared_fields(ts: &str, name: &str) -> HashSet<String> {
    let marker = format!("export interface {name} {{");
    let start = ts
        .find(&marker)
        .unwrap_or_else(|| panic!("contract.ts has no interface \"{name}\""));
    let body = &ts[start + marker.len()..];
    let end = body
        .find("\n}")
        .unwrap_or_else(|| panic!("the interface \"{name}\" is not closed"));

    let mut out = HashSet::new();
    for line in body[..end].lines() {
        // Comments are thrown away before parsing: `/** Something: with a colon */` would
        // otherwise give a field named "Something".
        let line = line.trim();
        if line.starts_with("//") || line.starts_with('*') || line.starts_with("/*") {
            continue;
        }
        let Some((left, _)) = line.split_once(':') else {
            continue;
        };
        let field = left.trim().trim_end_matches('?');
        if !field.is_empty() && field.chars().all(|c| c.is_alphanumeric() || c == '_') {
            out.insert(field.to_owned());
        }
    }
    assert!(
        !out.is_empty(),
        "the interface \"{name}\" turned out to have no fields"
    );
    out
}

/// The field names the core actually puts into JSON.
///
/// Taken from a real serialisation rather than from a struct's declaration: only what
/// crosses the boundary counts. A rename through `#[serde(rename)]` or an omission through
/// `skip_serializing_if` does not change the declaration — but it does change the contract.
fn serialized_fields<T: serde::Serialize>(value: &T) -> HashSet<String> {
    let json = serde_json::to_value(value).expect("the value will not serialise");
    let map = json
        .as_object()
        .expect("an object was expected: there are no fields to compare on a non-object");
    map.keys().cloned().collect()
}

/// Compare a shape both ways.
fn same_shape(rust: &HashSet<String>, ts: &HashSet<String>, what: &str) {
    let missing_in_ts: Vec<_> = rust.difference(ts).cloned().collect();
    let missing_in_rust: Vec<_> = ts.difference(rust).cloned().collect();

    assert!(
        missing_in_ts.is_empty(),
        "{what}: the core sends fields that are missing from contract.ts: {missing_in_ts:?}. \
         The interface will not read them, and it comes to light at a person's machine"
    );
    assert!(
        missing_in_rust.is_empty(),
        "{what}: contract.ts declares fields the core never sends: {missing_in_rust:?}. \
         The interface will wait for what never comes"
    );
}

#[test]
fn a_task_s_shape_matches_both_ways() {
    // The lists of values were compared before, but the field names were not. Renaming a
    // field in serde went by quietly: the build was whole, the types matched, and the
    // interface read `undefined` where it expected a number (debt T075).
    let record = vrcast_studio_lib::tasks::store::TaskRecord::new(
        "t1",
        TaskKind::Upload,
        Some(String::from("s1")),
    );
    same_shape(
        &serialized_fields(&record),
        &declared_fields(&contract_ts(), "Task"),
        "Task",
    );
}

#[test]
fn the_task_events_shapes_match_both_ways() {
    use vrcast_studio_lib::tasks::engine::TaskEvent;

    let progress = TaskEvent::Progress {
        id: String::from("t1"),
        state: TaskState::Running,
        progress: 0.5,
        stage: Some(DetailCode::StageConverting),
        speed_bps: Some(1),
        eta_s: Some(2),
    };
    // The event also has a tag field (`event`), declared in TypeScript too: it is what one
    // event is told from another by, and it has to match as well.
    same_shape(
        &serialized_fields(&progress),
        &declared_fields(&contract_ts(), "TaskProgressEvent"),
        "TaskProgressEvent",
    );

    let done = TaskEvent::Done {
        id: String::from("t1"),
        state: TaskState::Completed,
        error: None,
    };
    same_shape(
        &serialized_fields(&done),
        &declared_fields(&contract_ts(), "TaskDoneEvent"),
        "TaskDoneEvent",
    );
}

#[test]
fn an_examined_source_s_shape_matches_both_ways() {
    use vrcast_studio_lib::domain::source::{AudioTrack, SourceFile};

    let track = AudioTrack {
        index: 0,
        codec: String::from("aac"),
        channels: 2,
        bitrate_bps: Some(256_000),
        language: Some(String::from("rus")),
        title: None,
        is_default: true,
    };
    same_shape(
        &serialized_fields(&track),
        &declared_fields(&contract_ts(), "AudioTrack"),
        "AudioTrack",
    );

    let source = SourceFile {
        path: String::from("/v/a.mp4"),
        size_bytes: 1,
        duration_s: 1.0,
        width: 1920,
        height: 1080,
        fps: 24,
        bitrate_bps: 1,
        peak_bps: None,
        video_codec: String::from("h264"),
        pix_fmt: String::from("yuv420p"),
        color_transfer: None,
        audio_tracks: vec![track],
    };
    same_shape(
        &serialized_fields(&source),
        &declared_fields(&contract_ts(), "SourceFile"),
        "SourceFile",
    );
}

#[test]
fn the_playback_check_s_shape_matches_both_ways() {
    let verdict = vrcast_studio_lib::media::validate::classify("");
    same_shape(
        &serialized_fields(&verdict),
        &declared_fields(&contract_ts(), "Validation"),
        "Validation",
    );
}

#[test]
fn the_ffmpeg_info_s_shape_matches_both_ways() {
    let info = vrcast_studio_lib::media::ffmpeg::FfmpegInfo {
        version: String::from("ffmpeg version n8"),
        path: String::from("/x/ffmpeg"),
        has_x264: true,
        hardware: vec![String::from("h264_nvenc")],
    };
    same_shape(
        &serialized_fields(&info),
        &declared_fields(&contract_ts(), "FfmpegInfo"),
        "FfmpegInfo",
    );
}

#[test]
fn the_declaration_parser_does_not_take_a_comment_for_a_field() {
    // The parsing is simple, and a fault of its own would go unnoticed: a surplus "field"
    // out of a comment would make the comparison forever red, and a missed one forever
    // green.
    let ts = "export interface Sample {\n  /** Something: with a colon */\n  \
              // and a line comment: too\n  real: number;\n  \
              optional?: string;\n}\n";
    let fields = declared_fields(ts, "Sample");
    assert_eq!(
        fields.len(),
        2,
        "something surplus was parsed, or something needed was missed: {fields:?}"
    );
    assert!(fields.contains("real"));
    assert!(
        fields.contains("optional"),
        "the question mark was not dropped"
    );
}
