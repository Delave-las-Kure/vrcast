//! T015 — contract tests for the command layer.
//!
//! What is checked is the shape of the answer and the error codes, not the behaviour
//! underneath: the behaviour is the business of each layer's own tests. The point is that
//! the contract cannot be changed unnoticed — the interface reads the contract, and its
//! drift from the core would come to light at a person's machine.
//!
//! The commands are called directly, as ordinary functions: the thin wrappers for the shell
//! hold no logic, and demanding a live window with graphics for tests in continuous
//! integration will not do.

use std::sync::Arc;
use std::time::Duration;
use vrcast_studio_lib::commands::error::{AppError, DetailCode, ErrorCode};
use vrcast_studio_lib::commands::{api, AppState};
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;
use vrcast_studio_lib::tasks::state::{TaskKind, TaskState};

fn state() -> AppState {
    AppState::with_db(
        Arc::new(Db::open_in_memory().unwrap()),
        Arc::new(InMemorySecretStore::new()),
    )
    .expect("the application state would not assemble")
}

// ---------- the contract's completeness ----------

/// The secret-redaction registry is one per process, while the tests run in its threads in
/// parallel. Two tests that each call `forget_all` and then register their own wipe each
/// other out — and the loser reports a leak that never happened.
///
/// Caught on 2026-08-25 in the same run where this same race turned up in the unit checks:
/// a second secrets check, added beside the first, made the failure visible. A flickering
/// guard against leaking secrets is worse than none.
static REGISTRY: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Take the registry for the length of a test and clear it.
fn alone_with_registry() -> std::sync::MutexGuard<'static, ()> {
    let guard = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    vrcast_studio_lib::store::redact::forget_all();
    guard
}

/// Read one of the interface's catalogues.
fn catalogue(lang: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri has no parent directory")
        .join(format!("src/shared/i18n/{lang}.ts"));
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()))
}

/// Whether the catalogue holds an entry under this key.
///
/// The key is looked for at the start of a line rather than anywhere at all: a code
/// mentioned in passing in a comment must not count — otherwise the check passes on a
/// catalogue that holds no wording.
fn has_entry(catalogue: &str, key: &str) -> bool {
    catalogue
        .lines()
        .any(|line| line.trim_start().starts_with(&format!("{key}:")))
}

#[test]
fn every_code_has_a_wording_in_both_languages() {
    // This check used to demand a Russian message and hint right there in the core. The core
    // composes no sentences any more: it names the case with a code, while the wordings live
    // in the interface's catalogues, one per language (FR-105, FR-106).
    //
    // The requirement did not weaken from that but grew stronger: a wording must be in EVERY
    // language. A gap in one of the catalogues means an empty space on the screen instead of
    // an explanation — and a person would see it, not us.
    //
    // The TypeScript compiler checks the catalogues' completeness too (they are declared as
    // `Record<ErrorCode, ...>`), but this check does not rely on anyone having run the
    // interface build: it reads the files themselves.
    for lang in ["ru", "en"] {
        let text = catalogue(lang);
        for code in ErrorCode::ALL {
            assert!(
                has_entry(&text, code.as_str()),
                "the {lang} catalogue has no wording for the code {code}"
            );
        }
        for detail in DetailCode::ALL {
            assert!(
                has_entry(&text, detail.as_str()),
                "the {lang} catalogue has no wording for the detail {detail}"
            );
        }
    }
}

#[test]
fn the_codes_do_not_reach_the_text_meant_for_a_person() {
    // A technical code inside the text is a sign the wording was never written and a
    // placeholder was put in instead.
    let ru = catalogue("ru");
    for code in ErrorCode::ALL {
        let key = format!("{}:", code.as_str());
        let line = ru
            .lines()
            .find(|l| l.trim_start().starts_with(&key))
            .expect("the catalogue entry was found a moment ago");
        let after = line.split_once(':').map(|(_, r)| r).unwrap_or("");
        assert!(
            !after.contains(code.as_str()),
            "the wording for the code {code} holds the code itself: {line}"
        );
    }
}

#[test]
fn the_error_codes_do_not_repeat() {
    let mut seen = std::collections::HashSet::new();
    for code in ErrorCode::ALL {
        assert!(
            seen.insert(code.as_str()),
            "the code {} turns up twice",
            code.as_str()
        );
    }
    assert_eq!(seen.len(), ErrorCode::ALL.len());
}

#[test]
fn an_error_serialises_in_the_shape_the_contract_names() {
    // The shape { code, details?, cause? } is rule 2 of the contract. There are no ready
    // sentences in it: the core names the case, and the interface takes the wording from the
    // catalogue of the language in use.
    use vrcast_studio_lib::domain::wording::Detail;

    let err = AppError::new(ErrorCode::RemoteDiskFull)
        .with_detail(Detail::new(DetailCode::NotEnoughSpace).with("short_by", 1024_u64))
        .with_cause("stream-test.example.ru");
    let json = serde_json::to_value(&err).unwrap();

    assert_eq!(json["code"], "REMOTE_DISK_FULL");
    assert_eq!(json["details"][0]["key"], "NOT_ENOUGH_SPACE");
    // The number travels as a number: the units and the decimal separator differ between
    // languages, and choosing them is the interface's business, not the core's.
    assert_eq!(json["details"][0]["params"]["short_by"], 1024);
    assert_eq!(json["cause"], "stream-test.example.ru");

    assert!(
        json.get("message").is_none() && json.get("hint").is_none(),
        "the core is composing sentences again: {json}"
    );

    // Empty fields do not appear at all rather than arriving empty.
    let bare = serde_json::to_value(AppError::new(ErrorCode::Internal)).unwrap();
    assert!(
        bare.get("cause").is_none(),
        "an empty cause reached the answer"
    );
    assert!(
        bare.get("details").is_none(),
        "an empty list of details reached the answer"
    );
}

#[test]
fn an_error_s_detail_goes_through_secret_redaction() {
    // A detail often arrives from somebody else's library, which knows nothing of our rules
    // (constitution, principle IV).
    let _registry = alone_with_registry();
    let secret = "another-server-s-password-77";
    vrcast_studio_lib::store::redact::register(secret);

    let err = AppError::new(ErrorCode::SshAuthFailed)
        .with_cause(format!("the login failed, {secret} was used"));
    let json = serde_json::to_string(&err).unwrap();

    assert!(
        !json.contains(secret),
        "A SECRET IS IN THE COMMAND'S ANSWER: {json}"
    );
}

#[test]
fn a_substitution_in_a_detail_goes_through_secret_redaction() {
    // A new way out that arrived along with the two languages: a detail used to be one
    // string with the redaction standing on it, and now substitutions travel beside it — a
    // file name, a path, a profile's name. Any of them can come from the same place the
    // detail does, and go unnoticed (constitution, principle IV).
    use vrcast_studio_lib::domain::wording::Detail;

    let _registry = alone_with_registry();
    let secret = "a-key-passphrase-4242";
    vrcast_studio_lib::store::redact::register(secret);

    let err = AppError::new(ErrorCode::InvalidInput).with_detail(
        Detail::new(DetailCode::UploadSourceUnreadable)
            .with("path", format!("F:/{secret}/film.mp4")),
    );
    let json = serde_json::to_string(&err).unwrap();

    assert!(
        !json.contains(secret),
        "A SECRET IS IN THE COMMAND'S ANSWER: {json}"
    );
}

// ---------- the commands ----------

#[tokio::test]
async fn app_versions_returns_the_versions() {
    let s = state();
    // No server asked about: the About screen asks about none, and a version panel must not
    // depend on a machine being awake to say what the application itself is.
    let v = api::app_versions(&s, None).await.unwrap();

    assert!(!v.app.is_empty(), "the application's version is empty");
    assert!(v.schema >= 1, "the schema version was not filled in");
    // The server-side version arrives in Phase 7 — until then there is none, and that is
    // honest.
    assert!(v.server.is_none());
}

#[test]
fn a_new_application_s_task_list_is_empty() {
    let s = state();
    assert!(api::tasks_list(&s).unwrap().is_empty());
}

#[test]
fn reaching_for_a_task_that_does_not_exist_gives_the_contract_s_code() {
    let s = state();
    let err = api::task_get(&s, "no-such-task").expect_err("a task that does not exist was found");
    assert_eq!(err.code, ErrorCode::TaskNotFound);
    // The cause is kept: it is what tells which task was meant.
    assert!(err.cause.is_some());
}

#[test]
fn cancelling_a_task_that_does_not_exist_gives_the_contract_s_code() {
    let s = state();
    let err =
        api::task_cancel(&s, "no-such-task").expect_err("a task that does not exist was cancelled");
    assert_eq!(err.code, ErrorCode::TaskNotFound);
}

#[tokio::test]
async fn pausing_a_short_task_gives_a_code_of_its_own() {
    let s = state();
    // The work is long and cancellable rather than "sleep 600 ms": on a loaded machine a
    // short task would manage to finish before task_pause, and instead of the code under
    // test TASK_NOT_FOUND would arrive — a false failure.
    let id = s
        .tasks
        .submit(TaskKind::Probe, None, |ctx| async move {
            for _ in 0..600 {
                if ctx.is_cancelled() {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Ok(())
        })
        .await
        .unwrap();

    // Wait until the task really gets going.
    for _ in 0..50 {
        if matches!(
            s.tasks.get(&id).unwrap().map(|t| t.state),
            Some(TaskState::Running)
        ) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }

    let err = api::task_pause(&s, &id).expect_err("a short task was paused");
    assert_eq!(err.code, ErrorCode::TaskNotPausable);

    api::task_cancel(&s, &id).unwrap();
}

#[tokio::test]
async fn on_closing_each_task_is_explained_separately() {
    // FR-086. A general "tasks are running, close anyway?" is not enough: it gives nothing
    // to decide on.
    let s = state();

    let upload = s
        .tasks
        .submit(TaskKind::Upload, None, |ctx| async move {
            for _ in 0..100 {
                ctx.wait_while_paused().await;
                if ctx.is_cancelled() {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
            Ok(())
        })
        .await
        .unwrap();

    let convert = s
        .tasks
        .submit(TaskKind::Convert, None, |ctx| async move {
            for _ in 0..100 {
                if ctx.is_cancelled() {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
            Ok(())
        })
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(300)).await;

    let report = api::tasks_on_close(&s).unwrap();
    assert_eq!(report.len(), 2, "not every running task reached the report");

    let u = report
        .iter()
        .find(|t| t.id == upload)
        .expect("the transfer is missing");
    let c = report
        .iter()
        .find(|t| t.id == convert)
        .expect("the preparation is missing");

    // The difference between the kinds of task is the point of the requirement.
    assert_eq!(
        u.outcome, "resumes",
        "a transfer must carry on from where it got to"
    );
    assert_eq!(
        c.outcome, "restarts",
        "a preparation will not survive the closing"
    );
    // The explanation is a code with a substitution rather than a ready sentence: the
    // interface picks the wording in whichever language is chosen right now.
    assert_eq!(c.explanation.key, DetailCode::OnCloseRestartsLosing);
    assert_eq!(u.explanation.key, DetailCode::OnCloseResumesFrom);
    assert!(
        u.explanation.params.contains_key("percent"),
        "the explanation does not say how much is already done: {:?}",
        u.explanation
    );

    api::task_cancel(&s, &upload).unwrap();
    api::task_cancel(&s, &convert).unwrap();
}

#[test]
fn finished_tasks_do_not_reach_the_closing_report() {
    use vrcast_studio_lib::tasks::store;
    let s = state();

    let mut done = store::TaskRecord::new("t-finished", TaskKind::Upload, None);
    done.state = TaskState::Completed;
    store::upsert(&s.db, &done).unwrap();

    assert!(
        api::tasks_on_close(&s).unwrap().is_empty(),
        "a finished task reached the closing warning"
    );
}
