//! T655 (QA-24A №6) — when the local database will not open, say so to the person.
//!
//! Refusing to work with a database that is damaged, newer than this build, or out of reach
//! is right: tasks would not survive a restart and there would be nowhere to keep profiles.
//! What was wrong was *how* it refused — a line in stderr and `exit(1)` before any window,
//! and a Windows release has no console, so a person clicking the icon saw nothing at all.
//!
//! By the owner's decision of 2026-09-30, the minimal answer: a native message box (it needs
//! neither the database nor the interface nor IPC) saying what happened, where the database
//! file and the log are, and the safe steps to take. **The database is not touched** — not
//! renamed, not deleted, not "repaired". The language is the system's (Russian or English):
//! the person's own choice is kept in the very database that failed to open.
//!
//! The wording is built by a pure function, [`message`], and that is what the tests check;
//! [`show`] only puts it on the screen. The sentences themselves are in `startup_words.rs`.

mod startup_words;
use startup_words as words;

use crate::logging::Journal;
use crate::store::db::DbError;
use std::path::{Path, PathBuf};

/// The language of the message box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Ru,
    En,
}

impl Lang {
    /// From a locale tag such as `ru_RU.UTF-8`, `ru-BY` or `en_US`. Only the primary subtag is
    /// looked at, and anything that is not Russian is English — the same rule the interface
    /// follows (`src/shared/i18n/index.tsx`, `systemLang`).
    pub fn from_tag(tag: &str) -> Self {
        if tag.trim().to_ascii_lowercase().starts_with("ru") {
            Lang::Ru
        } else {
            Lang::En
        }
    }

    /// The system's language.
    pub fn system() -> Self {
        #[cfg(windows)]
        {
            // The language of the Windows interface — what the WebView reports to the
            // interface as `navigator.language` as well. 0x19 is LANG_RUSSIAN.
            let id = unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() };
            if id & 0x3ff == 0x19 {
                Lang::Ru
            } else {
                Lang::En
            }
        }
        #[cfg(not(windows))]
        {
            ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"]
                .iter()
                .filter_map(|v| std::env::var(v).ok())
                .find(|v| !v.is_empty())
                .map(|v| Lang::from_tag(&v))
                .unwrap_or(Lang::En)
        }
    }

    fn words(self) -> &'static words::Words {
        match self {
            Lang::Ru => &words::RU,
            Lang::En => &words::EN,
        }
    }
}

/// What went wrong, as far as the person's next step is concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureKind {
    /// The file is not a database, or the database in it is damaged.
    NotADatabase,
    /// Made by a newer version of the application.
    TooNew { found: u32, known: u32 },
    /// The file or its folder cannot be opened or written: rights, read-only, a lock by
    /// another program.
    NoAccess,
    /// The data directory itself could not be determined.
    NoDataDir,
    /// Bringing the database up to this version's schema failed.
    UpgradeFailed,
    /// Anything else.
    Other,
}

impl FailureKind {
    /// Sort an error of [`crate::store::db::Db::open`].
    pub fn of(e: &DbError) -> Self {
        use rusqlite::ErrorCode as C;
        let sqlite = |e: &rusqlite::Error| match e.sqlite_error_code() {
            Some(C::NotADatabase | C::DatabaseCorrupt) => Some(FailureKind::NotADatabase),
            Some(
                C::PermissionDenied
                | C::CannotOpen
                | C::ReadOnly
                | C::DatabaseBusy
                | C::DatabaseLocked
                | C::AuthorizationForStatementDenied,
            ) => Some(FailureKind::NoAccess),
            _ => None,
        };
        match e {
            DbError::TooNew { found, known } => FailureKind::TooNew {
                found: *found,
                known: *known,
            },
            DbError::NoDataDir => FailureKind::NoDataDir,
            // `Db::open` reports a folder it could not create this way.
            DbError::Open(rusqlite::Error::InvalidPath(_)) => FailureKind::NoAccess,
            DbError::Open(e) | DbError::Sql(e) => sqlite(e).unwrap_or(FailureKind::Other),
            DbError::Migration { source, .. } => {
                sqlite(source).unwrap_or(FailureKind::UpgradeFailed)
            }
            DbError::BrokenReferences { .. } => FailureKind::UpgradeFailed,
        }
    }
}

/// The start-up has failed: what, and on which file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupFailure {
    pub kind: FailureKind,
    /// The database file, when it is known.
    pub db_path: Option<PathBuf>,
    /// The error itself, redacted — for the log and for searching.
    pub detail: String,
}

impl StartupFailure {
    /// From an error of opening the database at `db_path`.
    pub fn from_db(db_path: Option<PathBuf>, e: &DbError) -> Self {
        Self {
            kind: FailureKind::of(e),
            db_path,
            detail: crate::store::redact::safe_display(e),
        }
    }

    /// From anything else that stopped the start-up once the database was open.
    pub fn other(db_path: Option<PathBuf>, e: &dyn std::fmt::Display) -> Self {
        Self {
            kind: FailureKind::Other,
            db_path,
            detail: crate::store::redact::safe_display(e),
        }
    }
}

impl std::fmt::Display for StartupFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.kind)?;
        if let Some(p) = &self.db_path {
            write!(f, " ({})", p.display())?;
        }
        write!(f, ": {}", self.detail)
    }
}

/// What the message box says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub title: String,
    pub body: String,
    /// The label of the button that shows the database file in the file manager, when there
    /// is a file or folder to show.
    pub reveal: Option<String>,
    /// The label of the button that closes.
    pub close: String,
}

/// Put the values into a sentence of the catalogue.
fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = template.to_owned();
    for (name, value) in values {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

/// Build the message. Pure: everything it says comes from its arguments and the catalogue.
pub fn message(failure: &StartupFailure, journal: &Journal, lang: Lang, version: &str) -> Message {
    let w = lang.words();

    let db = failure
        .db_path
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| w.unknown_path.to_owned());
    let folder = failure
        .db_path
        .as_ref()
        .and_then(|p| p.parent())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| db.clone());
    let file = failure
        .db_path
        .as_ref()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "vrcast-studio.sqlite".into());
    let (found, known) = match &failure.kind {
        FailureKind::TooNew { found, known } => (found.to_string(), known.to_string()),
        _ => (String::new(), String::new()),
    };
    let why = match journal {
        Journal::NotWritten(why) => why.as_str(),
        Journal::File(_) => "",
    };
    let values = [
        ("file", file.as_str()),
        ("folder", folder.as_str()),
        ("found", found.as_str()),
        ("known", known.as_str()),
        ("version", version),
        ("why", why),
    ];
    let say = |template: &str| fill(template, &values);

    let what = match &failure.kind {
        FailureKind::NotADatabase => w.what_not_a_database,
        FailureKind::TooNew { .. } => w.what_too_new,
        FailureKind::NoAccess => w.what_no_access,
        FailureKind::NoDataDir => w.what_no_data_dir,
        FailureKind::UpgradeFailed => w.what_upgrade_failed,
        FailureKind::Other => w.what_other,
    };

    // The steps, in the order they are safe in: close first, keep a copy before anything is
    // renamed, and never "start afresh" where the data is merely out of reach or newer.
    let steps: Vec<&str> = match &failure.kind {
        FailureKind::TooNew { .. } => vec![w.step_close, w.step_keep_copy, w.step_run_newer],
        FailureKind::NotADatabase | FailureKind::UpgradeFailed | FailureKind::Other => vec![
            w.step_close,
            w.step_keep_copy,
            w.step_start_afresh,
            w.step_send_log,
        ],
        FailureKind::NoAccess => vec![
            w.step_close,
            w.step_check_access,
            w.step_start_again,
            w.step_send_log,
        ],
        FailureKind::NoDataDir => vec![w.step_close, w.step_check_data_dir, w.step_send_log],
    };

    let log_line = match journal {
        Journal::File(p) => p.display().to_string(),
        Journal::NotWritten(_) => say(w.log_not_written),
    };

    let mut body = say(what);
    body.push_str("\n\n");
    body.push_str(w.db_file);
    body.push_str(&db);
    body.push('\n');
    body.push_str(w.log);
    body.push_str(&log_line);
    body.push_str("\n\n");
    body.push_str(w.untouched);
    body.push_str("\n\n");
    body.push_str(w.what_to_do);
    for (i, s) in steps.iter().enumerate() {
        body.push_str(&format!("\n{}. {}", i + 1, say(s)));
    }
    body.push_str("\n\n");
    body.push_str(w.details);
    body.push_str(&failure.detail);

    Message {
        title: w.title.to_owned(),
        body,
        reveal: failure.db_path.is_some().then(|| w.reveal.to_owned()),
        close: w.close.to_owned(),
    }
}

/// What the "show" button points the file manager at: the file if it is there, otherwise the
/// nearest folder that is.
fn what_to_reveal(db_path: &Path) -> Option<PathBuf> {
    db_path
        .ancestors()
        .find(|p| p.exists())
        .map(|p| p.to_path_buf())
}

/// Put the message on the screen and wait for the person to close it.
///
/// A native message box through `rfd` — the same library the dialog plugin uses — so that
/// nothing of the application (window, interface, database) needs to be working for it.
pub fn show(failure: &StartupFailure, journal: &Journal) {
    let msg = message(failure, journal, Lang::system(), env!("CARGO_PKG_VERSION"));

    // Without a display there is nobody to show it to, and GTK would wait for ever for a
    // main loop that never started. stderr and the log have it already.
    #[cfg(all(unix, not(target_os = "macos")))]
    if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return;
    }

    let buttons = match &msg.reveal {
        Some(reveal) => rfd::MessageButtons::OkCancelCustom(reveal.clone(), msg.close.clone()),
        None => rfd::MessageButtons::OkCustom(msg.close.clone()),
    };
    let answer = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(&msg.title)
        .set_description(&msg.body)
        .set_buttons(buttons)
        .show();

    if let (Some(reveal), rfd::MessageDialogResult::Custom(pressed)) = (&msg.reveal, answer) {
        if &pressed == reveal {
            if let Some(target) = failure.db_path.as_deref().and_then(what_to_reveal) {
                if let Err(e) = tauri_plugin_opener::reveal_item_in_dir(&target) {
                    tracing::warn!(error = %e, "the database file could not be shown");
                }
            }
        }
    }
}
