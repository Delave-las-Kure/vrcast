//! T381 — every notice the application can give has something that gives it.
//!
//! **What this is for.** A notice is a sentence written in both languages for a situation
//! the core is supposed to recognise. Writing the sentence is the easy half; the half that
//! gets forgotten is the branch that produces it. When it is forgotten, everything looks
//! finished — the code is declared, the wordings are there, the contract comparison is
//! content — and the situation, when it happens, is met with silence.
//!
//! Found on 2026-08-28: `NOTICE_HARDWARE_FAILED` says the graphics card refused and the work
//! went to the processor instead. `encoders::fallback_notice` builds it, nothing in the core
//! calls that function, and — the part that matters — **there is no fallback**. If NVENC
//! refuses mid-encode, which it does when its session limit is reached, the task simply
//! fails. The sentence explaining what happened cannot be reached by any path (R-41, T464).
//!
//! **One level deep, and said so.** This checks that a notice's producer is called from
//! somewhere in the core. It does not follow that caller upwards, so a producer called only
//! by another function nobody calls would pass. Full reachability wants a call graph; this
//! catches the class that has actually bitten, at a cost of forty lines.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

fn core_sources() -> Vec<(PathBuf, String)> {
    fn walk(dir: &Path, into: &mut Vec<(PathBuf, String)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, into);
            } else if path.extension().is_some_and(|e| e == "rs") {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    into.push((path, text));
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut out);
    assert!(out.len() > 20, "the core's sources were not found");
    out
}

/// Every `Notice*` and `Warn*` variant, read out of the file that declares them.
fn notice_codes() -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/domain/wording.rs");
    let text = std::fs::read_to_string(&path).expect("wording.rs would not read");
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some((name, _)) = line.split_once(" => ") else {
            continue;
        };
        if name.starts_with("Notice") || name.starts_with("Warn") {
            out.push(name.to_owned());
        }
    }
    assert!(
        out.len() > 5,
        "only {} notices were parsed out of wording.rs",
        out.len()
    );
    out
}

/// The function a piece of code sits inside: the nearest `fn` above it.
fn enclosing_fn(text: &str, at: usize) -> Option<String> {
    let before = &text[..at];
    let start = before.rfind("fn ")?;
    let rest = &before[start + 3..];
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

/// Notices with no path to a person, knowingly — each with the task that will settle it.
///
/// **Not a way of quieting the check.** A notice named here is one somebody has looked at and
/// written a task for; a notice not named here is one the build refuses to accept. The list
/// can only shrink: `no_reachable_notice_is_still_listed_as_orphaned` sees to that.
const ORPHANED: [(&str, &str); 0] = [];

#[test]
fn every_notice_has_a_producer_something_calls() {
    let sources = core_sources();
    let mut orphans: Vec<String> = Vec::new();

    let excused: HashSet<&str> = ORPHANED.iter().map(|(n, _)| *n).collect();

    for code in notice_codes() {
        if excused.contains(code.as_str()) {
            continue;
        }
        let needle = format!("DetailCode::{code}");
        let mut producers: HashSet<String> = HashSet::new();
        let mut built_anywhere = false;

        for (path, text) in &sources {
            // The declaration itself is not a use of it.
            if path.ends_with("wording.rs") {
                continue;
            }
            for (at, _) in text.match_indices(&needle) {
                built_anywhere = true;
                if let Some(name) = enclosing_fn(text, at) {
                    producers.insert(name);
                }
            }
        }

        if !built_anywhere {
            orphans.push(format!("{code}: nothing in the core ever builds it"));
            continue;
        }

        // Is any producer called from the core, somewhere other than where it is defined?
        // Called, as against merely defined: the same name with a bracket after it,
        // somewhere that is not its own `fn` line.
        let called = producers.iter().any(|producer| {
            let call = format!("{producer}(");
            sources.iter().any(|(_, text)| {
                text.match_indices(&call)
                    .any(|(at, _)| !text[..at].ends_with("fn "))
            })
        });
        if !called {
            orphans.push(format!(
                "{code}: built only by {producers:?}, and nothing in the core calls that"
            ));
        }
    }

    assert!(
        orphans.is_empty(),
        "these notices can never reach a person:\n  {}\n\n\
         The code is declared, the wordings are written in both languages, and the contract \
         comparison is content — but the situation, when it happens, is met with silence.",
        orphans.join("\n  ")
    );
}

#[test]
fn no_reachable_notice_is_still_listed_as_orphaned() {
    // The day somebody writes the branch, this fails and the line comes out. Without it the
    // list would outlive the faults it excuses, and a notice that works would go on being
    // described as one that cannot.
    let sources = core_sources();
    let mut wired: Vec<&str> = Vec::new();

    for (code, _) in ORPHANED {
        let needle = format!("DetailCode::{code}");
        let producers: HashSet<String> = sources
            .iter()
            .filter(|(path, _)| !path.ends_with("wording.rs"))
            .flat_map(|(_, text)| {
                text.match_indices(&needle)
                    .filter_map(|(at, _)| enclosing_fn(text, at))
                    .collect::<Vec<_>>()
            })
            .collect();
        let called = producers.iter().any(|producer| {
            let call = format!("{producer}(");
            sources.iter().any(|(_, text)| {
                text.match_indices(&call)
                    .any(|(at, _)| !text[..at].ends_with("fn "))
            })
        });
        if called {
            wired.push(code);
        }
    }

    assert!(
        wired.is_empty(),
        "these notices can reach a person now and no longer belong in ORPHANED: {wired:?}"
    );
}
