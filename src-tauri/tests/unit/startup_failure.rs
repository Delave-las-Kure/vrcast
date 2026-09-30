//! T655 (QA-24A №6) — a local database that will not open leads to a message, not to silence.
//!
//! Three things are checked:
//! - the real `Db::open` failures (not a database, newer schema, no access) go down the
//!   failure path of the real start-up (`AppState::bootstrap_at`) as the right kind, with the
//!   file named — and the file is left exactly as it was;
//! - the message built for each says what happened, names the database file and the log, and
//!   gives safe steps, in Russian and in English;
//! - the log is written to a file, redacted, and set aside when it grows too large.

use std::path::{Path, PathBuf};
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::logging::{self, Journal, LogFile};
use vrcast_studio_lib::startup_failure::{message, FailureKind, Lang, StartupFailure};
use vrcast_studio_lib::store::db::{Db, SCHEMA_VERSION};

/// A folder of its own for one test, removed when the test is done.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "vrcast-t655-{name}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The start-up's failure, or a panic if it did not fail.
fn fails(path: PathBuf) -> StartupFailure {
    match AppState::bootstrap_at(path) {
        Ok(_) => panic!("the start-up went ahead on a database it must refuse"),
        Err(f) => f,
    }
}

fn journal() -> Journal {
    Journal::File(PathBuf::from(
        "C:/Data/VRCast Studio/logs/vrcast-studio.log",
    ))
}

// ─── The real failures of Db::open go down the failure path ─────────────────────────────

#[test]
fn a_file_that_is_not_a_database_is_reported_as_such_and_left_alone() {
    let dir = TempDir::new("notdb");
    let path = dir.path().join("vrcast-studio.sqlite");
    let junk: Vec<u8> = b"this is not a database, it is a text file pretending to be one\n"
        .iter()
        .cycle()
        .take(8192)
        .copied()
        .collect();
    std::fs::write(&path, &junk).unwrap();

    let failure = fails(path.clone());
    assert_eq!(failure.kind, FailureKind::NotADatabase, "{failure}");
    assert_eq!(failure.db_path.as_deref(), Some(path.as_path()));
    assert!(
        failure.detail.contains("not a database"),
        "the particulars keep the error: {}",
        failure.detail
    );

    // Not deleted, not renamed, not rewritten.
    assert_eq!(std::fs::read(&path).unwrap(), junk, "the file was changed");
    let names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["vrcast-studio.sqlite".to_string()], "{names:?}");

    let m = message(&failure, &journal(), Lang::Ru, "0.1.0");
    assert!(m.body.contains(&path.display().to_string()), "{}", m.body);
    assert!(m.body.contains("повреждён"), "{}", m.body);
}

#[test]
fn a_newer_schema_is_reported_with_both_numbers_and_left_alone() {
    let dir = TempDir::new("newer");
    let path = dir.path().join("vrcast-studio.sqlite");
    {
        let db = Db::open(&path).unwrap();
        db.with_conn(|c| {
            c.execute_batch("PRAGMA user_version = 999;")?;
            Ok(())
        })
        .unwrap();
    }

    let failure = fails(path.clone());
    assert_eq!(
        failure.kind,
        FailureKind::TooNew {
            found: 999,
            known: SCHEMA_VERSION
        }
    );
    assert_eq!(failure.db_path.as_deref(), Some(path.as_path()));

    // Still the newer version's database: nothing was migrated "down" or over it.
    let version: i64 = rusqlite::Connection::open(&path)
        .unwrap()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 999);

    let m = message(&failure, &journal(), Lang::En, "0.1.0");
    assert!(m.body.contains("schema 999"), "{}", m.body);
    assert!(
        m.body.contains(&format!("schema {SCHEMA_VERSION} at most")),
        "{}",
        m.body
    );
    assert!(m.body.contains(&path.display().to_string()), "{}", m.body);
}

#[test]
fn a_database_that_cannot_be_opened_is_reported_as_no_access() {
    let dir = TempDir::new("noaccess");

    // Where the file should be, a folder stands: SQLite cannot open it (SQLITE_CANTOPEN).
    let path = dir.path().join("vrcast-studio.sqlite");
    std::fs::create_dir_all(&path).unwrap();
    let failure = fails(path.clone());
    assert_eq!(failure.kind, FailureKind::NoAccess, "{failure}");
    assert_eq!(failure.db_path.as_deref(), Some(path.as_path()));
    assert!(path.is_dir(), "the folder in the way was removed");

    // Its folder cannot be made, because a file is in the way.
    let blocker = dir.path().join("blocker");
    std::fs::write(&blocker, b"x").unwrap();
    let path = blocker.join("vrcast-studio.sqlite");
    let failure = fails(path.clone());
    assert_eq!(failure.kind, FailureKind::NoAccess, "{failure}");
    assert_eq!(std::fs::read(&blocker).unwrap(), b"x");

    let m = message(&failure, &journal(), Lang::Ru, "0.1.0");
    assert!(m.body.contains("Нет доступа"), "{}", m.body);
    assert!(m.body.contains(&path.display().to_string()), "{}", m.body);
}

// ─── The message ────────────────────────────────────────────────────────────────────────

fn failure(kind: FailureKind) -> StartupFailure {
    StartupFailure {
        kind,
        db_path: Some(PathBuf::from(
            "C:/Users/me/AppData/Roaming/VRCast/VRCast Studio/data/vrcast-studio.sqlite",
        )),
        detail: "[the particulars]".into(),
    }
}

const DB: &str = "C:/Users/me/AppData/Roaming/VRCast/VRCast Studio/data/vrcast-studio.sqlite";
const LOG: &str = "C:/Data/VRCast Studio/logs/vrcast-studio.log";

#[test]
fn every_message_names_the_file_the_log_and_says_nothing_was_touched() {
    let kinds = [
        FailureKind::NotADatabase,
        FailureKind::TooNew {
            found: 999,
            known: 20,
        },
        FailureKind::NoAccess,
        FailureKind::UpgradeFailed,
        FailureKind::Other,
    ];
    for kind in kinds {
        for lang in [Lang::Ru, Lang::En] {
            let m = message(&failure(kind.clone()), &journal(), lang, "0.1.0");
            let b = &m.body;
            assert!(b.contains(DB), "{kind:?} {lang:?}: no database path\n{b}");
            assert!(b.contains(LOG), "{kind:?} {lang:?}: no log path\n{b}");
            assert!(b.contains("[the particulars]"), "{kind:?} {lang:?}\n{b}");
            assert!(b.contains("\n1. "), "{kind:?} {lang:?}: no steps\n{b}");
            let untouched = match lang {
                Lang::Ru => "ничего не удаляло и не меняло",
                Lang::En => "has not deleted or changed anything",
            };
            assert!(b.contains(untouched), "{kind:?} {lang:?}\n{b}");
            assert!(m.reveal.is_some(), "a known file can be shown");
            assert!(!m.title.is_empty() && !m.close.is_empty());
        }
    }
}

#[test]
fn not_a_database_says_to_keep_a_copy_before_anything_else() {
    let m = message(
        &failure(FailureKind::NotADatabase),
        &journal(),
        Lang::Ru,
        "0.1.0",
    );
    let b = &m.body;
    assert!(b.contains("повреждён или не является базой"), "{b}");
    let close = b.find("1. Закройте VRCast Studio").expect(b);
    let copy = b
        .find("2. Сохраните копию файла vrcast-studio.sqlite")
        .expect(b);
    assert!(close < copy);
    assert!(b.contains("vrcast-studio.sqlite-wal"), "{b}");
    assert!(b.contains("vrcast-studio.sqlite.broken"), "{b}");
    assert_eq!(m.title, "VRCast Studio не может запуститься");
    assert_eq!(m.reveal.as_deref(), Some("Показать файл базы"));
}

#[test]
fn a_newer_schema_says_which_version_to_run_and_not_to_start_afresh() {
    let kind = FailureKind::TooNew {
        found: 999,
        known: 20,
    };
    let ru = message(&failure(kind.clone()), &journal(), Lang::Ru, "0.1.0").body;
    assert!(ru.contains("более новая версия"), "{ru}");
    assert!(ru.contains("схема 999"), "{ru}");
    assert!(ru.contains("не новее 20"), "{ru}");
    assert!(ru.contains("версию VRCast Studio не старше"), "{ru}");
    assert!(ru.contains("новее 0.1.0"), "{ru}");
    assert!(ru.contains("Сохраните копию"), "{ru}");
    // Starting over would throw away data a newer version can still read.
    assert!(!ru.contains(".broken"), "{ru}");

    let en = message(&failure(kind), &journal(), Lang::En, "0.1.0").body;
    assert!(en.contains("newer version"), "{en}");
    assert!(en.contains("newer than 0.1.0"), "{en}");
    assert!(!en.contains(".broken"), "{en}");
}

#[test]
fn no_access_points_at_the_folder_and_not_at_starting_afresh() {
    let m = message(
        &failure(FailureKind::NoAccess),
        &journal(),
        Lang::En,
        "0.1.0",
    );
    let b = &m.body;
    assert!(b.contains("cannot be reached"), "{b}");
    assert!(
        b.contains("C:/Users/me/AppData/Roaming/VRCast/VRCast Studio/data"),
        "{b}"
    );
    assert!(b.contains("read-only"), "{b}");
    assert!(!b.contains(".broken"), "{b}");
    assert_eq!(m.title, "VRCast Studio cannot start");
}

#[test]
fn a_log_that_is_not_written_is_said_so_and_an_unknown_path_has_nothing_to_show() {
    let f = StartupFailure {
        kind: FailureKind::NoDataDir,
        db_path: None,
        detail: "could not determine the application data directory".into(),
    };
    let j = Journal::NotWritten("no folder".into());
    let ru = message(&f, &j, Lang::Ru, "0.1.0");
    assert!(
        ru.body.contains("не записывается (no folder)"),
        "{}",
        ru.body
    );
    assert!(ru.body.contains("(не определён)"), "{}", ru.body);
    assert!(ru.reveal.is_none());
    let en = message(&f, &j, Lang::En, "0.1.0");
    assert!(
        en.body.contains("not being written (no folder)"),
        "{}",
        en.body
    );
}

#[test]
fn the_language_follows_the_primary_subtag() {
    for tag in ["ru", "ru_RU.UTF-8", "ru-BY", "RU"] {
        assert_eq!(Lang::from_tag(tag), Lang::Ru, "{tag}");
    }
    for tag in ["en_US.UTF-8", "de-DE", "", "C", "uk-UA"] {
        assert_eq!(Lang::from_tag(tag), Lang::En, "{tag}");
    }
}

// ─── The log file ───────────────────────────────────────────────────────────────────────

#[test]
fn the_log_is_written_to_a_file_and_redacted() {
    let dir = TempDir::new("log");
    // A private key is cut out by its shape, not by registration — so this does not touch the
    // process-wide registry the redaction tests take turns with.
    let body = format!("b3BlbnNzaC1rZXktdjE{}", uuid::Uuid::new_v4().simple());
    let key =
        format!("-----BEGIN OPENSSH PRIVATE KEY-----\n{body}\n-----END OPENSSH PRIVATE KEY-----");

    let file = LogFile::open(dir.path()).unwrap();
    let path = file.path().to_path_buf();
    assert_eq!(path, dir.path().join(logging::FILE_NAME));

    tracing::subscriber::with_default(logging::subscriber_with(false, Some(file)), || {
        tracing::error!(error = %"file is not a database", "could not prepare the stores");
        tracing::info!(key = %key, "signing in");
    });

    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("could not prepare the stores"), "{text}");
    assert!(text.contains("file is not a database"), "{text}");
    assert!(text.contains("signing in"), "{text}");
    assert!(
        !text.contains(&body),
        "the key reached the log file:\n{text}"
    );
    assert!(
        !text.contains('\u{1b}'),
        "colour codes in the file:\n{text}"
    );
}

#[test]
fn the_log_is_set_aside_when_it_grows_too_large() {
    let dir = TempDir::new("rotate");
    let file = LogFile::open_with(dir.path(), 2_000, 1).unwrap();
    let current = file.path().to_path_buf();
    let aside = dir.path().join(format!("{}.1", logging::FILE_NAME));

    tracing::subscriber::with_default(logging::subscriber_with(false, Some(file)), || {
        for i in 0..200 {
            tracing::info!(line = i, "a line of ordinary chatter to fill the log");
        }
    });

    let now = std::fs::metadata(&current).unwrap().len();
    let before = std::fs::metadata(&aside).unwrap().len();
    assert!(now <= 2_000, "the current file is {now} bytes");
    assert!(before <= 2_000, "the set-aside file is {before} bytes");
    // Only one set-aside file is kept.
    assert!(!dir
        .path()
        .join(format!("{}.2", logging::FILE_NAME))
        .exists());
    // The newest line is in the current file, whole.
    let text = std::fs::read_to_string(&current).unwrap();
    assert!(text.contains("line=199"), "{text}");
    assert!(
        text.lines().all(|l| l.contains("ordinary chatter")),
        "{text}"
    );
}

#[test]
fn a_log_already_too_large_is_set_aside_on_opening() {
    let dir = TempDir::new("reopen");
    let current = dir.path().join(logging::FILE_NAME);
    std::fs::write(&current, vec![b'x'; 3_000]).unwrap();

    let file = LogFile::open_with(dir.path(), 2_000, 1).unwrap();
    drop(file);
    assert_eq!(std::fs::metadata(&current).unwrap().len(), 0);
    let aside = dir.path().join(format!("{}.1", logging::FILE_NAME));
    assert_eq!(std::fs::metadata(&aside).unwrap().len(), 3_000);
}

#[test]
fn a_log_in_a_folder_that_cannot_be_made_is_reported_not_fatal() {
    let dir = TempDir::new("nolog");
    let blocker = dir.path().join("blocker");
    std::fs::write(&blocker, b"x").unwrap();

    let (file, journal) = logging::open_journal(Some(blocker.join("logs")));
    assert!(file.is_none());
    match journal {
        Journal::NotWritten(why) => assert!(why.contains("vrcast-studio.log"), "{why}"),
        other => panic!("{other:?}"),
    }
    let (file, journal) = logging::open_journal(None);
    assert!(file.is_none());
    assert!(matches!(journal, Journal::NotWritten(_)));

    let logs = dir.path().join("logs");
    let (file, journal) = logging::open_journal(Some(logs.clone()));
    assert!(file.is_some());
    assert_eq!(journal, Journal::File(logs.join(logging::FILE_NAME)));
}

/// "Forget everything" (FR-114) removes the whole data directory while the running
/// application still holds its log open. The open file must not keep the folder there.
#[test]
fn an_open_log_does_not_stop_its_folder_being_removed() {
    let dir = TempDir::new("forget");
    let data = dir.path().join("data");
    let file = LogFile::open(&data.join("logs")).unwrap();
    tracing::subscriber::with_default(logging::subscriber_with(false, Some(file.clone())), || {
        tracing::info!("still running");
    });

    std::fs::remove_dir_all(&data).expect("the open log kept the data folder from going");
    assert!(!data.exists());

    // And the application goes on logging without falling over.
    tracing::subscriber::with_default(logging::subscriber_with(false, Some(file)), || {
        tracing::info!("after forgetting");
    });
}
