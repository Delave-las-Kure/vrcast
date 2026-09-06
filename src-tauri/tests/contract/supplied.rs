//! T531 — a command is called, and the argument its whole logic turns on is never sent.
//!
//! ⚠ **Four times in one sweep, and the guards that exist saw none of them.** T500: the
//! stalls screen calls `diag_explain_stalls` with two arguments of three, so both branches
//! that need the third are unreachable and every lagging viewer falls into the catch-all.
//! T501: the viewers screen sends a medium's id where the core builds a path out of its slug,
//! so capping a viewer can never succeed. T510: both deployment proofs hand the gate a
//! fingerprint they took a line earlier, so the comparison cannot fail. T522: the ladder
//! screen sends `{ path }` alone, and the three fields that carry what the person knows about
//! the film go by default forever.
//!
//! `contract_sync` asks whether the shapes agree, and they do — a field that is optional on
//! both sides agrees with itself. `reachable.test.ts` asks whether anything calls the command,
//! and something does. **Neither asks whether the call carries what the command needs**, and
//! the answer was no four times.
//!
//! **Two questions, because either alone is blind.**
//!
//! 1. A request field the interface never writes anywhere. `serde` fills it with a default,
//!    the call type-checks, and the core branches on something no screen can set.
//! 2. An optional parameter on an `ipc` wrapper. TypeScript is exactly as silent about a
//!    caller that omits it, and that silence is T500 in one line.
//!
//! **What this cannot see.** A field supplied with the wrong value — T501 sends `slug`, spelt
//! correctly, holding an id. Naming is not meaning, and no scan of the source will find that
//! one; only reading the two sides together does. This catches the half that is mechanical.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

fn frontend(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri has no parent directory")
        .join(rel)
}

/// Fields the core accepts that no screen sends, each with the reason and what closes it.
///
/// **This list is the point of the check, and it must shrink.** An entry says "the core can
/// take this, deliberately nobody sends it yet, because —". An entry with no reason is a
/// screen somebody forgot to finish, which is what this is here to find.
const NOT_SENT_YET: &[(&str, &str, &str)] = &[
    (
        "LadderRequest",
        "native_height",
        "Found by this guard on its first run, 2026-09-06, and it is a real gap rather than a \
         deferred one: an upscaled master — 1080p stretched to 2160p — is planned as genuine \
         4K, so every rung is sized for detail that is not in the file. The core has always taken \
         it; what is missing is the control that asks for it. Closed by T492's edit \
         screen or by an ask on the ladder screen.",
    ),
    (
        "MeasureRequest",
        "native_height",
        "The same gap one layer down, and the pair is why it matters: the measurement and the \
         ladder must agree about what the material is, and today both guess. Closed with the \
         entry above, by the same control.",
    ),
    (
        "LadderRequest",
        "declared_layout",
        "Side-by-side and over-under material: 3580x1080 is a 4K load and reads as FHD, which \
         is measured and recorded in the project's own notes. The core takes the person's \
         word for it and no screen offers the word. Closed by the same control.",
    ),
];

/// Wrapper parameters that may be left out, each with the reason and what closes it.
///
/// **An optional argument is a promise that the command works without it.** Where that is
/// false the compiler says nothing. Two kinds of entry live here and they are not the same:
/// one says the promise is true and why, the other names the open task where it is false.
/// Only the first kind is finished, and the list must shrink from the second end.
const MAY_BE_OMITTED: &[(&str, &str, &str)] = &[
    (
        "diagExplainStalls",
        "file",
        "NOT a true promise \u{26a0} this is T500, and this guard found it on its first run. \
         Without the shape both `TheFileItself` and `ThePlayer` are unreachable, so every \
         lagging viewer falls into the catch-all and SC-012 can name two culprits of four. \
         It stays optional because the fix is not a wire to reconnect: the screen knows a \
         server, the shape belongs to a file on it, and somebody has to decide where a \
         remote file's average and ten-second peak are measured. Closed by T500.",
    ),
    (
        "appVersions",
        "serverId",
        "Two questions in one command by design: with an identifier it answers what the server \
         runs as well, and the About screen asks it both ways — once at start-up with nothing \
         deployed, once with a server chosen. Neither answer is degraded; they are different \
         questions.",
    ),
];

fn commands_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands")
}

/// Every `pub struct …Request` under `src/commands`, with its public field names.
fn request_structs() -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    let mut stack = vec![commands_dir()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("could not read {}: {e}", dir.display()))
        {
            let path = entry.expect("could not read a directory entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()));
            out.extend(structs_in(&text));
        }
    }
    out.sort();
    out
}

/// The request structs of one file.
///
/// Deliberately literal about the shape it accepts — `pub struct NameRequest {` on its own
/// line, fields as `    pub name:` — because the alternative is a parser, and a parser that
/// quietly matches nothing is the failure this whole file exists to prevent. If a struct is
/// ever written differently, `every_excuse_still_names_something_real` is what says so.
fn structs_in(text: &str) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let Some(rest) = line.strip_prefix("pub struct ") else {
            continue;
        };
        let Some(name) = rest.strip_suffix(" {") else {
            continue;
        };
        if !name.ends_with("Request") {
            continue;
        }
        let mut fields = Vec::new();
        for body in lines.by_ref() {
            if body == "}" {
                break;
            }
            if let Some(field) = body.strip_prefix("    pub ") {
                if let Some((name, _)) = field.split_once(':') {
                    fields.push(name.to_owned());
                }
            }
        }
        out.push((name.to_owned(), fields));
    }
    out
}

/// Every identifier the interface writes, excluding its tests and the contract itself.
///
/// **The contract is excluded on purpose.** It declares every field by name; counting it
/// would mean every field is always "sent", and the check would pass on an empty interface.
/// Tests are excluded for the reason `reachable.test.ts` gives: a test reaches what it
/// imports, so counting them lets a screen nobody can use look connected.
fn words_the_interface_writes() -> HashSet<String> {
    let root = frontend("src");
    let mut seen = HashSet::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("could not read {}: {e}", dir.display()))
        {
            let path = entry.expect("could not read a directory entry").path();
            if path.is_dir() {
                if path.file_name().and_then(|n| n.to_str()) != Some("__tests__") {
                    stack.push(path);
                }
                continue;
            }
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default();
            if ext != "ts" && ext != "tsx" {
                continue;
            }
            if path.ends_with("shared/contract.ts") {
                continue;
            }
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()));
            let mut word = String::new();
            for c in text.chars() {
                if c.is_ascii_alphanumeric() || c == '_' {
                    word.push(c);
                } else if !word.is_empty() {
                    seen.insert(std::mem::take(&mut word));
                }
            }
            if !word.is_empty() {
                seen.insert(word);
            }
        }
    }
    seen
}

#[test]
fn every_field_the_core_takes_is_one_some_screen_can_send() {
    let excused: HashSet<(&str, &str)> = NOT_SENT_YET.iter().map(|(s, f, _)| (*s, *f)).collect();
    let written = words_the_interface_writes();

    let mut unsent = Vec::new();
    for (name, fields) in request_structs() {
        for field in fields {
            if written.contains(&field) || excused.contains(&(name.as_str(), field.as_str())) {
                continue;
            }
            unsent.push(format!("{name}.{field}"));
        }
    }

    assert!(
        unsent.is_empty(),
        "the core branches on request fields no screen writes anywhere: {unsent:?}\n\
         Either a screen must ask for them, or they must be deleted from the request. \
         If neither yet, put them in NOT_SENT_YET **with the reason and what closes it** — \
         an entry without one is the thing this check exists to find."
    );
}

#[test]
fn no_wrapper_lets_a_caller_leave_out_what_the_command_needs() {
    let path = frontend("src/shared/ipc.ts");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()));
    let excused: HashSet<(&str, &str)> = MAY_BE_OMITTED.iter().map(|(w, p, _)| (*w, *p)).collect();

    // The wrapper whose parameter list we are inside. A wrapper is declared at one level of
    // indentation, as `  name: (` — anything deeper is a nested function and not the door.
    let mut wrapper: Option<&str> = None;
    let mut optional = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("  ") {
            if !rest.starts_with(' ') {
                wrapper = rest
                    .split_once(':')
                    .filter(|(_, after)| after.trim_start().starts_with('('))
                    .map(|(name, _)| name);
            }
        }
        let Some(name) = wrapper else { continue };
        // `foo?: T` in a parameter position. The `(` case catches a one-line wrapper, the
        // bare case a parameter on its own line.
        for piece in line.split(['(', ',']) {
            let piece = piece.trim();
            let Some((param, _)) = piece.split_once("?:") else {
                continue;
            };
            if param.is_empty() || !param.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                continue;
            }
            if excused.contains(&(name, param)) {
                continue;
            }
            optional.push(format!("{name}({param}?)"));
        }
    }

    assert!(
        optional.is_empty(),
        "these wrappers let a caller leave an argument out, and the compiler will not \
         mention it: {optional:?}\n\
         Make it required, or put it in MAY_BE_OMITTED **with the reason the command still \
         does its whole job without it**. T500 is what the silent version costs."
    );
}

/// An excuse for something that no longer exists is a comment pretending to be a check.
#[test]
fn every_excuse_still_names_something_real() {
    let structs = request_structs();
    let mut rotten = Vec::new();

    for (name, field, _) in NOT_SENT_YET {
        let found = structs
            .iter()
            .any(|(s, fields)| s == name && fields.iter().any(|f| f == field));
        if !found {
            rotten.push(format!("NOT_SENT_YET: {name}.{field}"));
        }
    }

    let ipc =
        std::fs::read_to_string(frontend("src/shared/ipc.ts")).expect("could not read ipc.ts");
    for (wrapper, param, _) in MAY_BE_OMITTED {
        if !ipc.contains(&format!("{wrapper}:")) || !ipc.contains(&format!("{param}?:")) {
            rotten.push(format!("MAY_BE_OMITTED: {wrapper}({param}?)"));
        }
    }

    assert!(
        rotten.is_empty(),
        "these excuses name something that is no longer there: {rotten:?}\n\
         Delete the entry — the gap it described is closed."
    );
}

/// The scan itself must find things, or all three tests above pass on nothing.
///
/// **The failure this rules out is the one that has no symptom.** A struct written in a shape
/// the parser does not match simply disappears, and a check over an empty list is a check
/// that always passes — worse than no check, because it is believed.
#[test]
fn the_scan_reaches_both_sides() {
    let structs = request_structs();
    assert!(
        structs.len() >= 6,
        "only {} request structs were found under src/commands — the parser has stopped \
         matching the shape they are written in",
        structs.len()
    );
    // Named as well as counted: six structs parsed wrongly would meet a count.
    for wanted in ["LadderRequest", "MeasureRequest", "UploadRequest"] {
        assert!(
            structs
                .iter()
                .any(|(name, fields)| name == wanted && !fields.is_empty()),
            "{wanted} was not found with any fields - the parser matched a name, not a body"
        );
    }
    assert!(
        structs.iter().any(|(name, fields)| name == "LadderRequest"
            && fields.iter().any(|f| f == "declared_layout")),
        "LadderRequest.declared_layout was not found, and it is what this guard was built on"
    );

    let written = words_the_interface_writes();
    assert!(
        written.contains("ladderPlan"),
        "the interface was scanned and `ladderPlan` is not among its words — src/ was not read"
    );
    assert!(
        !written.contains("declared_layout"),
        "`declared_layout` now appears in the interface: if a screen sends it, delete its \
         entry from NOT_SENT_YET"
    );
}
