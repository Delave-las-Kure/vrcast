//! T398 — the core is written in English, and the tray shows words it was given.
//!
//! Two rules, both the owner's, and both of the kind that no compiler can hold:
//!
//! 1. **The core's own prose is English.** Comments, doc comments, names, test names. The
//!    words a person reads live in `src/shared/i18n/{ru,en}.ts` and nowhere else, so that a
//!    sentence can be reworded, translated or dropped without touching the core at all.
//! 2. **The tray shows nothing of its own.** Its menu is the one place in the core that puts
//!    words on a screen, so it is the one place where the first rule could be broken while
//!    looking perfectly reasonable — a menu entry has to say something, and the nearest
//!    string is right there.
//!
//! Written **before** the tray menu exists (T395), which is the only order in which the
//! second rule can be held: a check written afterwards would be written around whatever was
//! already there.

use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, into: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, into);
        } else if path.extension().is_some_and(|e| e == "rs") {
            into.push(path);
        }
    }
}

fn core() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn has_cyrillic(line: &str) -> bool {
    line.chars()
        .any(|c| matches!(c, 'а'..='я' | 'А'..='Я' | 'ё' | 'Ё'))
}

/// Files where Cyrillic is the subject rather than the language, each with the reason.
///
/// **Three of them, and each one is data or a quotation** — not prose:
///
/// - `domain/media.rs` transliterates: `'а' => "a"`. The letters *are* the table. Writing it
///   any other way would mean the table no longer says what it does.
/// - `domain/ladder.rs` quotes a line of the shell script this project was ported from,
///   which says what it says (constitution VI).
/// - `commands/limits.rs` names a section of `contracts/ipc-commands.md` by its own title.
///
/// A quotation is not the core speaking. Everything else is, and is English.
const CYRILLIC_IS_THE_SUBJECT: [(&str, &str); 3] = [
    (
        "media.rs",
        "the transliteration table: the letters are the data",
    ),
    (
        "ladder.rs",
        "quotes a line of the reference script it was ported from",
    ),
    (
        "limits.rs",
        "names a section of the contract by its own title",
    ),
];

#[test]
fn the_core_speaks_english() {
    let root = core();
    let mut files = Vec::new();
    rust_files(&root, &mut files);
    assert!(files.len() > 20, "the core's sources were not found");

    let excused: Vec<&str> = CYRILLIC_IS_THE_SUBJECT.iter().map(|(f, _)| *f).collect();
    let mut found: Vec<String> = Vec::new();

    for file in &files {
        let name = file
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if excused.contains(&name.as_str()) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            if has_cyrillic(line) {
                let shown = file.strip_prefix(&root).unwrap_or(file);
                found.push(format!("{}:{}  {}", shown.display(), i + 1, line.trim()));
            }
        }
    }

    assert!(
        found.is_empty(),
        "Russian prose in the core:\n  {}\n\n\
         The words a person reads live in src/shared/i18n. A sentence written here is stuck \
         in the language it was written in, and cannot be reworded, translated or dropped \
         without editing the core.",
        found.join("\n  ")
    );
}

#[test]
fn nothing_excused_has_stopped_needing_the_excuse() {
    // A list of exceptions nobody rechecks outlives the reasons for it. The day the
    // transliteration table moves or the quotation goes, this says so.
    let root = core();
    let mut files = Vec::new();
    rust_files(&root, &mut files);

    let mut stale: Vec<&str> = Vec::new();
    for (name, _) in CYRILLIC_IS_THE_SUBJECT {
        let still = files.iter().any(|f| {
            f.file_name().is_some_and(|n| n == name)
                && std::fs::read_to_string(f)
                    .map(|t| t.lines().any(has_cyrillic))
                    .unwrap_or(false)
        });
        if !still {
            stale.push(name);
        }
    }
    assert!(
        stale.is_empty(),
        "these no longer contain any Cyrillic and no longer need excusing: {stale:?}"
    );
}

#[test]
fn the_tray_puts_no_words_of_its_own_on_the_screen() {
    // The tray menu is the only place in the core that shows text. Its labels have to arrive
    // from the interface, which owns the wordings in both languages — so the tray module may
    // hold no string it could show.
    //
    // Written before the menu exists, on purpose: afterwards it would be written around
    // whatever was already there.
    let tray = core().join("tray");
    if !tray.is_dir() {
        return;
    }
    let mut files = Vec::new();
    rust_files(&tray, &mut files);

    // What the module legitimately names: the libraries it looks for.
    const NOT_FOR_READING: [&str; 2] = ["libayatana-appindicator3.so.1", "libappindicator3.so.1"];

    let mut showable: Vec<String> = Vec::new();
    for file in &files {
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            let mut rest = code;
            while let Some(open) = rest.find('"') {
                let after = &rest[open + 1..];
                let Some(close) = after.find('"') else { break };
                let literal = &after[..close];
                // A word with a space in it, or with Cyrillic, is a sentence for a person.
                // A library name, a path or a flag is not.
                let readable = literal.contains(' ') || has_cyrillic(literal);
                if readable && !NOT_FOR_READING.contains(&literal) {
                    showable.push(format!(
                        "{}:{}  \"{literal}\"",
                        file.file_name().unwrap_or_default().to_string_lossy(),
                        i + 1
                    ));
                }
                rest = &after[close + 1..];
            }
        }
    }

    assert!(
        showable.is_empty(),
        "the tray module holds words it could show:\n  {}\n\n\
         Menu labels come from src/shared/i18n, handed in from the interface. A label written \
         here exists in one language, and the language a person chose has nothing to do with \
         it.",
        showable.join("\n  ")
    );
}
