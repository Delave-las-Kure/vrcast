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
        names::DEPLOY_PROGRESS,
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
        has_libvmaf: true,
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

// ---------- the shapes the interface SENDS (T367) ----------
//
// Everything above compares what the core **answers** with. Nothing compared what the
// interface **asks** with — and that is where three commands were broken from the day they
// were written.
//
// Tauri renames the arguments of a command itself: `{ serverId }` reaches a parameter named
// `server_id`. It does **not** rename anything inside a nested object — that is plain serde,
// and there is no `rename_all` anywhere in `src-tauri/src/commands`. So `call("ladder_build",
// { request: { serverId, .. } })` hands serde an object with no `server_id` in it, and
// `BuildRequest::server_id` has neither `Option` nor a default: the call fails before a
// single line of the command runs. "Build the set" never worked. Neither did capping a
// viewer. Found on 2026-08-28, from an owner's report that the quality set "did not work".
//
// It stayed invisible because every screen test mocks the whole of `ipc` and throws the
// argument away (`ladderBuild: (...a) => mockBuild(...a)`), and the contract comparison only
// ever looked at answers.
//
// So the payload is built here out of what `ipc.ts` itself declares it sends, and handed to
// the very type the command takes. Not a list written by hand beside the code it checks —
// the two descriptions are read from the two files and made to meet.

fn ipc_ts() -> String {
    let path = frontend_file("src/shared/ipc.ts");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()))
}

/// One field of a request, as the interface declares it.
struct Sent {
    name: String,
    optional: bool,
    ts_type: String,
}

/// The body of a braced block, from just after the opening brace.
fn block_body(after_brace: &str) -> &str {
    let mut depth = 1usize;
    for (i, c) in after_brace.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &after_brace[..i];
                }
            }
            _ => {}
        }
    }
    panic!("a block was opened and never closed");
}

/// The fields declared in a block of `name: type;` lines.
fn fields_in(body: &str) -> Vec<Sent> {
    // A field ends at a semicolon or at a line break, and both have to be honoured: the
    // requests written on the spot put the whole object on one line, and splitting by lines
    // alone read `path: string; codec?: string` as a single field named `path` whose type
    // "contains null". It still failed, but for the wrong reason — and a check that is red
    // for the wrong reason is one nobody can act on.
    let mut clean = String::new();
    for line in body.lines() {
        let line = line.trim();
        if line.starts_with("//") || line.starts_with('*') || line.starts_with("/*") {
            continue;
        }
        clean.push_str(line.split("//").next().unwrap_or(""));
        clean.push(';');
    }

    let mut out = Vec::new();
    for line in clean.split(';') {
        let line = line.trim();
        let Some((left, right)) = line.split_once(':') else {
            continue;
        };
        let optional = left.trim().ends_with('?');
        let name = left.trim().trim_end_matches('?').trim().to_owned();
        if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        out.push(Sent {
            name,
            optional,
            ts_type: right.trim().trim_end_matches(',').trim().to_owned(),
        });
    }
    assert!(!out.is_empty(), "a request turned out to have no fields");
    out
}

/// What one `ipc.ts` method declares it puts in its `request` object.
///
/// Both shapes are understood on purpose: a named type out of `contract.ts` — which is what
/// these ought to be — and an object literal written on the spot, which is what they drifted
/// into. Refusing to read the second would mean this check could not see the fault it exists
/// for.
fn sent_fields(method: &str) -> Vec<Sent> {
    let ipc = ipc_ts();
    let marker = format!("\n  {method}: (request: ");
    let start = ipc
        .find(&marker)
        .unwrap_or_else(|| panic!("ipc.ts has no method \"{method}\" that takes a request"));
    let rest = &ipc[start + marker.len()..];

    match rest.strip_prefix('{') {
        Some(inline) => fields_in(block_body(inline)),
        None => {
            let named: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            let ts = contract_ts();
            let marker = format!("export interface {named} {{");
            let at = ts.find(&marker).unwrap_or_else(|| {
                panic!("{method} asks for a type \"{named}\" that contract.ts does not declare")
            });
            fields_in(block_body(&ts[at + marker.len()..]))
        }
    }
}

/// A value of the shape the interface says it sends.
fn sample(ts_type: &str) -> serde_json::Value {
    let t = ts_type.trim();
    // A type that admits null is given null: that is the case the core has to survive, and
    // an `Option` that has only ever been shown a value is an `Option` nobody has tried.
    if t.contains("null") {
        return serde_json::Value::Null;
    }
    if t.ends_with("[]") {
        return serde_json::json!([]);
    }
    match t {
        "string" => serde_json::json!(""),
        "number" => serde_json::json!(0),
        "boolean" => serde_json::json!(true),
        _ => serde_json::Value::Null,
    }
}

/// Build the payload and hand it to the type the command takes.
///
/// Twice: once with only the fields the interface always sends, and once with the optional
/// ones too. The first is what a screen sends on a plain path — and it is the one that was
/// broken. The second catches a field that is only sent sometimes and is misspelt.
fn payload_is_readable<T: serde::de::DeserializeOwned>(method: &str) {
    let fields = sent_fields(method);

    for (what, all) in [
        ("without the optional fields", false),
        ("with every field", true),
    ] {
        let mut object = serde_json::Map::new();
        for f in &fields {
            if f.optional && !all {
                continue;
            }
            object.insert(f.name.clone(), sample(&f.ts_type));
        }
        let json = serde_json::Value::Object(object);
        serde_json::from_value::<T>(json.clone()).unwrap_or_else(|e| {
            panic!(
                "ipc.{method} sends something the core cannot read, {what}: {e}\n\
                 What the interface says it sends: {json}\n\
                 Nothing catches this at build time: the types check on both sides, because \
                 neither side has ever been shown the other's. It fails at a person's \
                 machine, on the button, every time."
            )
        });
    }
}

/// And nothing the interface sends is quietly thrown away.
///
/// Serde ignores a field it does not know by default, so a misspelt name is not an error —
/// it is a setting a person chose that never arrives. That is worse than a refusal: the
/// screen goes on showing the choice as made.
fn nothing_sent_is_dropped<T: serde::Serialize>(method: &str, empty: &T) {
    let known = serialized_fields(empty);
    let dropped: Vec<String> = sent_fields(method)
        .into_iter()
        .map(|f| f.name)
        .filter(|n| !known.contains(n))
        .collect();
    assert!(
        dropped.is_empty(),
        "ipc.{method} sends {dropped:?}, and the core has no such fields. Serde drops what it \
         does not know without a word, so the choice a person made on screen never arrives, \
         and the screen goes on showing it as made"
    );
}

#[test]
fn the_ladder_build_payload_can_be_read() {
    payload_is_readable::<vrcast_studio_lib::commands::ladder::BuildRequest>("ladderBuild");
}

#[test]
fn the_ladder_plan_payload_can_be_read() {
    payload_is_readable::<vrcast_studio_lib::commands::ladder::LadderRequest>("ladderPlan");
}

#[test]
fn the_measure_payloads_can_be_read() {
    payload_is_readable::<vrcast_studio_lib::commands::quality::MeasureRequest>(
        "qualityMeasurePreview",
    );
    payload_is_readable::<vrcast_studio_lib::commands::quality::MeasureRequest>(
        "qualityMeasureStart",
    );
}

#[test]
fn the_limit_payloads_can_be_read() {
    payload_is_readable::<vrcast_studio_lib::commands::limits::LimitRequest>("limitPreview");
    payload_is_readable::<vrcast_studio_lib::commands::limits::LimitRequest>("limitSet");
}

#[test]
fn the_upload_payload_can_be_read() {
    // The one that was always right, kept as the control: without it a fault in the parsing
    // above would turn every check here red at once and be read as a fault in the code.
    payload_is_readable::<vrcast_studio_lib::commands::upload::UploadRequest>("uploadStart");
}

#[test]
fn nothing_the_screens_send_is_quietly_dropped() {
    use vrcast_studio_lib::commands::{ladder, limits, quality};

    nothing_sent_is_dropped(
        "ladderPlan",
        &ladder::LadderRequest {
            path: String::new(),
            codec: String::new(),
            native_height: None,
            declared_layout: None,
            prefer_hardware: true,
        },
    );
    nothing_sent_is_dropped(
        "ladderBuild",
        &ladder::BuildRequest {
            server_id: String::new(),
            path: String::new(),
            slug: String::new(),
            rungs: Vec::new(),
            audio_track: 0,
            prefer_hardware: true,
        },
    );
    nothing_sent_is_dropped(
        "qualityMeasureStart",
        &quality::MeasureRequest {
            path: String::new(),
            codec: String::new(),
            native_height: None,
            prefer_hardware: true,
        },
    );
    nothing_sent_is_dropped(
        "limitSet",
        &limits::LimitRequest {
            server_id: String::new(),
            ip: String::new(),
            slug: String::new(),
            cap_bps: 0,
        },
    );
}

// ---------- every registered command is reachable, and nothing else is called (T377) ----------
//
// **The class of fault this exists for**: a capability written, registered, documented — and
// wired to nothing. It cannot be seen from the outside, because from the outside there is
// simply no button; and it cannot be seen from a test, because every screen test stands in
// for the whole of `ipc`.
//
// Found on 2026-08-28, three at once: `quality_measure_reuse`, `quality_measurements` and
// `quality_measure_forget` are registered in `lib.rs`, described in `contracts/ipc-commands.md`,
// carry an error code and wordings in both languages — and have no wrapper in `ipc.ts` at all.
// So lending a measurement to the next episode of a season, which FR-146 requires, exists only
// in the core. The task that promised it is marked done.
//
// The other direction matters just as much and is cheaper to get wrong: a wrapper calling a
// name the core no longer registers fails at the moment a person presses the button, with
// "command not found" and nothing on the screen to say what happened.

/// Every command name the core registers, out of `generate_handler!`.
fn registered_commands() -> HashSet<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()));

    let start = text
        .find("generate_handler![")
        .expect("lib.rs no longer registers commands with generate_handler!");
    let body = block_body_square(&text[start + "generate_handler![".len()..]);

    let mut out = HashSet::new();
    for line in body.lines() {
        let line = line
            .split("//")
            .next()
            .unwrap_or("")
            .trim()
            .trim_end_matches(',');
        // `commands::ipc::name`, and nothing else has that shape here.
        if let Some(name) = line.rsplit("::").next() {
            if !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            {
                out.insert(name.to_owned());
            }
        }
    }
    assert!(
        out.len() > 50,
        "only {} commands were parsed out of generate_handler! — the parsing has come adrift \
         from the file, and a check that finds nothing agrees with everything",
        out.len()
    );
    out
}

/// The body of a `[ … ]` block, from just after the opening bracket.
fn block_body_square(after: &str) -> &str {
    let mut depth = 1usize;
    for (i, c) in after.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return &after[..i];
                }
            }
            _ => {}
        }
    }
    panic!("generate_handler! was opened and never closed");
}

/// Every command name the interface actually calls.
fn called_commands() -> HashSet<String> {
    let ipc = ipc_ts();
    let mut out = HashSet::new();
    // `call<T>("name", …)` and `call("name", …)` — the one way this file reaches the core.
    for (i, _) in ipc.match_indices("call") {
        let rest = &ipc[i + "call".len()..];
        let rest = rest
            .strip_prefix('<')
            .map_or(rest, |r| r.find('>').map_or(r, |j| &r[j + 1..]));
        let Some(rest) = rest.strip_prefix('(') else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix('"') else {
            continue;
        };
        if let Some(end) = rest.find('"') {
            out.insert(rest[..end].to_owned());
        }
    }
    assert!(
        out.len() > 50,
        "only {} calls were parsed out of ipc.ts — the parsing has come adrift from the file",
        out.len()
    );
    out
}

/// Registered, and knowingly not offered yet — each with the task that removes it from here.
///
/// **The list is the point, not the exception.** Five capabilities were found unwired on
/// 2026-08-28, and the reason they stayed unwired is that nothing named them. Named, they are
/// work with an owner; unnamed, they are a feature everyone believes exists. Nothing may be
/// added here without a task number beside it, and `the_list_of_unwired_commands_does_not_rot`
/// below makes the list shrink on its own.
const NOT_WIRED_YET: [(&str, &str); 5] = [
    (
        "quality_measure_reuse",
        "T431 — lending a measurement to the next episode (FR-146)",
    ),
    (
        "quality_measurements",
        "T431 — the list to choose a lender from",
    ),
    (
        "quality_measure_forget",
        "T433 — a way out of a borrowed measurement",
    ),
    (
        "geo_status",
        "T461 — whether the tables of places are there and current",
    ),
    (
        "geo_update",
        "T461 — fetching them; without it a stale table has no button",
    ),
];

#[test]
fn every_registered_command_is_reachable_from_the_interface() {
    let registered = registered_commands();
    let called = called_commands();
    let known: HashSet<String> = NOT_WIRED_YET.iter().map(|(n, _)| (*n).to_owned()).collect();

    let unreachable: Vec<String> = {
        let mut v: Vec<String> = registered
            .difference(&called)
            .filter(|n| !known.contains(*n))
            .cloned()
            .collect();
        v.sort();
        v
    };

    assert!(
        unreachable.is_empty(),
        "these commands are registered in the core and called from nowhere: {unreachable:?}\n\n\
         A capability with no way in is not a capability. It cannot be found by using the \
         application, because there is no button; and it cannot be found by the screen tests, \
         because they stand in for the whole of `ipc`. If one of these is deliberately not \
         offered yet, it does not belong in `generate_handler!` until it is."
    );
}

#[test]
fn the_interface_calls_nothing_the_core_does_not_register() {
    let registered = registered_commands();
    let called = called_commands();

    let missing: Vec<String> = {
        let mut v: Vec<String> = called.difference(&registered).cloned().collect();
        v.sort();
        v
    };

    assert!(
        missing.is_empty(),
        "the interface calls commands the core does not register: {missing:?}\n\n\
         Nothing catches this at build time — the types check on both sides. It fails at a \
         person's machine, on the button, with \"command not found\"."
    );
}

#[test]
fn the_list_of_unwired_commands_does_not_rot() {
    // A list of exceptions that is never checked becomes a list of things nobody looks at.
    // The day one of these is wired up, this fails and the line comes out — so the list can
    // only ever shrink.
    let called = called_commands();
    let registered = registered_commands();

    let wired: Vec<&str> = NOT_WIRED_YET
        .iter()
        .filter(|(n, _)| called.contains(*n))
        .map(|(n, _)| *n)
        .collect();
    assert!(
        wired.is_empty(),
        "these are called from the interface now and no longer belong in NOT_WIRED_YET:          {wired:?}"
    );

    let gone: Vec<&str> = NOT_WIRED_YET
        .iter()
        .filter(|(n, _)| !registered.contains(*n))
        .map(|(n, _)| *n)
        .collect();
    assert!(
        gone.is_empty(),
        "these are in NOT_WIRED_YET and the core no longer registers them: {gone:?}.          The exception outlived the thing it excused"
    );
}

// ---------- the shapes match by TYPE, not only by name (T379) ----------
//
// **Where the name-only comparison is blind.** Everything above checks that both sides know
// the same fields. It says nothing about what is in them, and that is where the drift found
// on 2026-08-28 was hiding: `MeasurePreview.encoder` is an internally tagged enum in the core
// — `{"kind":"software"}`, always an object — and `string` in `contract.ts`. Both sides agree
// there is a field called `encoder`; they disagree about everything else.
//
// It has not bitten yet only because nothing draws it. The day something does, it draws
// `[object Object]` — or nothing, quietly.

/// What kind of JSON a value actually is.
fn json_kind(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// What kind of JSON a TypeScript type says to expect, when it says anything simple enough
/// to be sure about.
///
/// `None` for anything with a union, a generic or a name of its own: those are for a person
/// to read, and guessing at them would make this check argue with correct declarations.
fn ts_kind(ts_type: &str) -> Option<&'static str> {
    let t = ts_type.trim();
    if t.contains('|') || t.contains('<') {
        return None;
    }
    if t.ends_with("[]") {
        return Some("array");
    }
    match t {
        "string" => Some("string"),
        "number" => Some("number"),
        "boolean" => Some("boolean"),
        _ => None,
    }
}

/// Compare a serialised value against a declared interface, field by field, by kind.
fn same_kinds<T: serde::Serialize>(value: &T, interface: &str) {
    let json = serde_json::to_value(value).expect("the value will not serialise");
    let object = json.as_object().expect("an object was expected");

    let ts = contract_ts();
    let marker = format!("export interface {interface} {{");
    let at = ts
        .find(&marker)
        .unwrap_or_else(|| panic!("contract.ts has no interface \"{interface}\""));
    let declared = fields_in(block_body(&ts[at + marker.len()..]));

    let mut wrong: Vec<String> = Vec::new();
    for field in &declared {
        let Some(want) = ts_kind(&field.ts_type) else {
            continue;
        };
        let Some(got) = object.get(&field.name) else {
            continue; // The name comparison is somebody else's job.
        };
        // A field the core left empty says nothing about its type.
        if got.is_null() {
            continue;
        }
        let is = json_kind(got);
        if is != want {
            wrong.push(format!(
                "{}.{}: contract.ts says {}, the core sends {}",
                interface, field.name, field.ts_type, is
            ));
        }
    }

    assert!(
        wrong.is_empty(),
        "the contract and the core disagree about what is in these fields:\n  {}\n\n\
         Both sides know the field exists, so nothing above catches it. It reaches the screen \
         as an empty space or as [object Object].",
        wrong.join("\n  ")
    );
}

#[test]
fn the_measure_preview_says_what_it_actually_contains() {
    use vrcast_studio_lib::commands::quality::MeasurePreview;
    use vrcast_studio_lib::media::encoders::Encoder;

    let preview = MeasurePreview {
        source_key: String::from("1:film.mp4"),
        points: 12,
        already_measured: 0,
        about_seconds: 180,
        estimate_from_points: 0,
        chunk_starts: vec![1, 2, 3],
        anchor_mbps: 8,
        // The one that matters: an internally tagged enum is an object, on both variants.
        encoder: Encoder::Software,
        notices: Vec::new(),
    };
    same_kinds(&preview, "MeasurePreview");
}
