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

/// The interface's sources, which this guard did not look at for a year (T466).
fn interface() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri has no parent")
        .join("src")
}

/// Files of the interface, by extension. The catalogues are not among them.
fn interface_files(dir: &Path, into: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            interface_files(&path, into);
            continue;
        }
        // **The catalogues are Russian on purpose** — one of them is the Russian one. They
        // are the single place a sentence for a person is allowed to live, which is the whole
        // rule this guard exists to hold.
        if matches!(name.as_str(), "ru.ts" | "en.ts") {
            continue;
        }
        if name.ends_with(".ts") || name.ends_with(".tsx") || name.ends_with(".css") {
            into.push(path);
        }
    }
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
            // ⚠ **A line in the log is not a line on the screen** (T400). This exempts
            // `tracing::` and nothing else, and it narrows the rule towards what the rule
            // says it is for rather than away from it: a menu label reaches a person by
            // being handed to `MenuItem`, and no logging macro can hand it anything. Left
            // out, the rule forbids the module to explain a failure to whoever is reading
            // the trace — and the way that gets resolved in practice is by not explaining.
            if code.trim_start().starts_with("tracing::") {
                continue;
            }
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

/// Whether this line's Cyrillic is prose the code is speaking, rather than data or a
/// quotation.
///
/// **Three shapes are not prose, and the rule needs all three.**
///
/// 1. **A reference to the catalogue.** A test that asserts on what a person sees has to name
///    it, and `ru.ui.tasks.done` names it in the one place it lives. A sentence typed out
///    where the reference would do is a second copy of a wording, and the day the catalogue
///    changes they disagree silently.
/// 2. **A string under `src/shared/i18n`.** That directory *is* where the words live — one of
///    its files is the Russian catalogue. The units in `format.ts` and the encoder names in
///    `render.ts` are words for a person, in the place words for a person belong. What is not
///    excused there is the **comments**: a comment is the code speaking, wherever it sits.
/// 3. **A quotation inside a comment.** `/// Size: 4096 → «4,0 КБ»` is an English comment
///    showing what comes out. Taking the example away to satisfy a guard would leave a
///    sentence that describes formatting without showing any.
fn is_prose(line: &str, strings_are_data: bool) -> bool {
    let code = line.trim();
    for reference in [
        "ru.ui.",
        "ru.details.",
        "ru.errors.",
        "en.ui.",
        "en.details.",
        "en.errors.",
    ] {
        if code.contains(reference) {
            return false;
        }
    }

    // Everything outside quotes: `"…"`, `«…»` and backticks all mark something being shown
    // rather than said.
    let mut outside = String::new();
    let mut quote: Option<char> = None;
    // What came before, so a `/` can be told from a comment marker: a pattern only ever
    // follows an opening bracket, a comma or a colon. `/**` was swallowing half a line and
    // leaving the rest to be called prose.
    let mut previous = ' ';
    for c in code.chars() {
        match quote {
            Some(open) => {
                let closes = match open {
                    '«' => c == '»',
                    other => c == other,
                };
                if closes {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' | '`' | '«' => quote = Some(c),
                // A regular expression is a literal as much as a string is, and a test
                // matches on what a person sees with one: `findByText(/1 файл · 1,5 ГБ/)`.
                // Only where literals are data, though — outside those places a `/` is far
                // more often division or a path than a pattern.
                '/' if strings_are_data && matches!(previous, '(' | ',' | ':') => quote = Some('/'),
                _ => outside.push(c),
            },
        }
        if !c.is_whitespace() {
            previous = c;
        }
    }

    if strings_are_data {
        // Only what is said outside a literal counts.
        return has_cyrillic(&outside);
    }
    // Elsewhere a Russian string is a wording in the wrong place, and a Russian comment is
    // prose in the wrong language. Both are this guard's business — except a quotation
    // inside a comment, which is the third shape above.
    //
    // A wrapped line of a block comment starts with none of these markers, so a comment is
    // recognised by its markers *or* by having no code on it at all: a line that is neither
    // a statement nor a declaration is prose whatever it begins with. Found when
    // `LadderScreen`'s wrapped example escaped the rule.
    let is_comment = code.starts_with("//")
        || code.starts_with('*')
        || code.starts_with("/*")
        || !code.contains([';', '=', '{', '(']);
    if is_comment {
        has_cyrillic(&outside)
    } else {
        has_cyrillic(code)
    }
}

#[test]
fn the_interface_speaks_english_too() {
    // **The guard looked at half the code.** It has walked `src-tauri/src` since T398 and
    // never once opened the interface — where the same rule applies for the same reason, and
    // where a comment written in one language is stuck in it exactly as hard.
    let root = interface();
    let mut files = Vec::new();
    interface_files(&root, &mut files);
    assert!(
        files.len() > 30,
        "the interface's sources were not found: {} files",
        files.len()
    );

    let layer = root.join("shared").join("i18n");
    let mut found: Vec<String> = Vec::new();
    for file in &files {
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        // **Two places where a Russian string is data rather than a wording.**
        //
        // Under `shared/i18n` the words a person reads are the point — one of those files is
        // the Russian catalogue, and the units and encoder names beside it are words for a
        // person in the place words for a person belong.
        //
        // In a test, a Russian string is a fixture or an expected value. A server called
        // "Мой сервер", a path `/srv/раздача/видео`, a film titled "Название фильма" — these
        // are there **on purpose**, checking that the application holds Cyrillic in names and
        // paths, which is the first thing that breaks for this owner. Translating them would
        // not tidy the code, it would take the check away. Expectations on formatted output
        // ("20,0 Мбит/с") are the formatter's answer written down; referring to the formatter
        // instead would be the test agreeing with itself.
        //
        // A comment is prose in both places, and is still checked.
        let strings_are_data =
            file.starts_with(&layer) || file.components().any(|c| c.as_os_str() == "__tests__");
        for (i, line) in text.lines().enumerate() {
            if has_cyrillic(line) && is_prose(line, strings_are_data) {
                let shown = file.strip_prefix(&root).unwrap_or(file);
                found.push(format!("{}:{}  {}", shown.display(), i + 1, line.trim()));
            }
        }
    }

    assert!(
        found.is_empty(),
        "Russian prose in the interface ({} lines):\n  {}\n\n\
         The words a person reads live in src/shared/i18n. A sentence written anywhere else is \
         stuck in the language it was typed in, and cannot be reworded, translated or dropped \
         without editing the code.",
        found.len(),
        found.join("\n  ")
    );
}

#[test]
fn the_rule_can_tell_prose_from_a_quotation() {
    // The whole check rests on this telling apart, and a fault in it would make the guard
    // agree with itself and with nothing else — the same reason `snake_case` is checked
    // beside the comparison that uses it.
    assert!(is_prose("// Функции проходят как есть.", false));
    assert!(is_prose("  const x = \"Готово\";", false));
    assert!(!is_prose("/// Size: 4096 → «4,0 КБ» / \"4.0 KB\".", false));
    assert!(!is_prose(
        "expect(screen.getByText(ru.ui.tasks.done));",
        false
    ));

    // Where strings are data, one is where it belongs; a comment still is not.
    assert!(!is_prose("  ru: { kbit: \"кбит/с\" },", true));
    assert!(!is_prose("    name: \"Мой сервер\",", true));
    assert!(is_prose("  // Слова живут здесь.", true));

    // A pattern is a literal as much as a string is, where literals are data.
    assert!(!is_prose(
        "  expect(screen.getByText(/1 файл/)).toBeTruthy();",
        true
    ));
    // A comment marker is not a pattern, whatever follows it.
    assert!(!is_prose("/** Bitrate: 9_000_000 → «9,0 Мбит/с». */", true));
    // And a `/` outside those places is division or a path, not a pattern.
    assert!(is_prose("  const path = \"/srv/раздача\";", false));

    // A wrapped line of a block comment has no marker of its own, and used to slip through
    // on that alone.
    assert!(is_prose(
        "guess comes from the name, and это пример of one,",
        false
    ));
    assert!(!is_prose(
        "guess comes from the name, and `фильм 22.mp4` guesses down,",
        false
    ));
}
