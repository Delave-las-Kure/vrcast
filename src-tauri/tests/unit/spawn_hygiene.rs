//! T369 — no external program is started without saying it wants no window.
//!
//! **What this is about.** A released build on Windows has no console of its own
//! (`main.rs`: `windows_subsystem = "windows"`), so every child process that is started
//! without `CREATE_NO_WINDOW` is given a fresh console window by the system. It appears and
//! vanishes in a fraction of a second. Choosing one file on the preparation screen starts
//! five short programs, and every change to a field on it starts four more — so the screen
//! flickers with black rectangles, and it looks exactly like something crashing over and
//! over. Reported by the owner on 2026-08-28.
//!
//! It was right in one place all along — `ManagedProcess`, which starts the long tasks — and
//! wrong in seven others, all of them the short reads: examining a source, listing keyframes,
//! measuring peaks, asking FFmpeg what it can do, probing complexity. The flag was not
//! forgotten in seven places by seven accidents. It was forgotten because there was nothing
//! that made a person think about it.
//!
//! **So the check is on the source, not on the behaviour.** A window that appears for a
//! quarter of a second on somebody else's desktop cannot be seen from a test, and a check
//! that cannot fail is worse than none. What *can* be seen is a program being started
//! without going through the one place that knows about the flag. So that is what is looked
//! for: `Command::new` anywhere in the core outside `tasks/process.rs`.
//!
//! It is a blunt rule on purpose. A new way of starting programs should be an argument, not
//! an oversight — and this makes it one.

use std::path::{Path, PathBuf};

/// Where the core's sources live.
fn source_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// The one file allowed to start a program, because it is the one that knows how.
const THE_ONE_PLACE: &str = "process.rs";

fn rust_files(dir: &Path, into: &mut Vec<PathBuf>) {
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|e| panic!("could not read {}: {e}", dir.display()));
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, into);
        } else if path.extension().is_some_and(|e| e == "rs") {
            into.push(path);
        }
    }
}

#[test]
fn every_external_program_is_started_through_the_one_place_that_knows_about_windows() {
    let root = source_root();
    let mut files = Vec::new();
    rust_files(&root, &mut files);
    assert!(
        files.len() > 20,
        "the sources were not found where they were expected: {} files under {}",
        files.len(),
        root.display()
    );

    let mut strays: Vec<String> = Vec::new();
    for file in &files {
        if file.file_name().is_some_and(|n| n == THE_ONE_PLACE) {
            continue;
        }
        let text = std::fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("could not read {}: {e}", file.display()));
        for (i, line) in text.lines().enumerate() {
            // The word inside a comment is not a start — this very file is full of them,
            // and so is the module it guards.
            let code = line.split("//").next().unwrap_or("");
            if code.contains("Command::new") {
                let shown = file.strip_prefix(&root).unwrap_or(file);
                strays.push(format!("{}:{}", shown.display(), i + 1));
            }
        }
    }

    assert!(
        strays.is_empty(),
        "a program is started outside tasks/process.rs, so nothing gave it \
         CREATE_NO_WINDOW: {strays:?}\n\n\
         On Windows the system then hands it a console window of its own, which appears and \
         vanishes — and a released build has no console to inherit instead. Use \
         `tasks::process::quiet` for a short read, or `ManagedProcess` for anything that has \
         to be killable."
    );
}

#[test]
fn the_check_can_actually_fail() {
    // Without this, the day somebody moves the sources or renames the extension, the check
    // above would find no files, find no strays, and be green forever.
    let mut files = Vec::new();
    rust_files(&source_root(), &mut files);
    let starts_programs = files
        .iter()
        .filter(|f| f.file_name().is_some_and(|n| n == THE_ONE_PLACE))
        .any(|f| {
            std::fs::read_to_string(f)
                .map(|t| t.contains("Command::new"))
                .unwrap_or(false)
        });
    assert!(
        starts_programs,
        "tasks/process.rs no longer starts anything, so the rule above is guarding nothing \
         and would pass however the rest of the core started its programs"
    );
}
