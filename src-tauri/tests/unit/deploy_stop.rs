//! T609 — a cancelled deployment is not written down as `Cancelled`, and keeps the server's
//! deployment slot, until the server confirms that nothing of the run is left.
//!
//! **Why here, with a stand-in for the server** — the same reasoning as `cutting_stop.rs`
//! (T605). What is checked is the rule between three parts that need no server to meet:
//! `tasks::deploy::confirm_stopped` (does not return until a stop attempt says "confirmed"),
//! the real `TaskEngine` (writes `Cancelled` only once the work returns) and the real
//! `running_deploy_for` guard behind `deploy_run`/`server_upgrade_run` (reads the task's
//! state out of the same engine). Whether the stop itself works on a real server — the
//! command in flight let finish, nothing marked left, the SSH undo timer spared — is
//! `tests/integration/deploy_cancel.rs`'s job.
//!
//! The guard is asked through the public commands, which check it before any connection is
//! made: refused → `DEPLOY_ALREADY_RUNNING`; let through → they fail for their own reasons
//! (the profile has no confirmed fingerprint), which is not what is asked here.
//!
//! Also here: the pure pieces the mechanism rests on — reading the stop script's answer,
//! the command wrapper that carries the mark, how patient a stop is with a command that may
//! still be running, and how a failed apt step describes itself (T614).

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use vrcast_studio_lib::commands::api as core;
use vrcast_studio_lib::commands::deploy::api as deploy;
use vrcast_studio_lib::commands::error::{AppError, ErrorCode};
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::domain::deploy_steps::StepId;
use vrcast_studio_lib::domain::dns_verdict::Ipv6Choice;
use vrcast_studio_lib::domain::marked::{read_stop, stop_script, StopReport, Stopped};
use vrcast_studio_lib::domain::server_profile::{AuthKind, ServerProfile};
use vrcast_studio_lib::domain::wording::DetailCode;
use vrcast_studio_lib::server::deploy::{
    apt_complaint, leftovers_script, DeployError, RunMark, APT_HEAL, RUN_VAR, TEMP_PLACES,
    TEMP_SUFFIX,
};
use vrcast_studio_lib::server::marked::{Patience, MAX_GRACE_S};
use vrcast_studio_lib::ssh::CommandOutput;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;
use vrcast_studio_lib::tasks::state::{TaskKind, TaskState};

const SERVER: &str = "t609-server";

fn app_state() -> AppState {
    let state = AppState::with_db(
        Arc::new(Db::open_in_memory().unwrap()),
        Arc::new(InMemorySecretStore::new()),
    )
    .expect("the application state would not assemble");
    vrcast_studio_lib::store::profiles::insert(
        &state.db,
        &ServerProfile {
            id: SERVER.to_owned(),
            name: String::from("T609"),
            host: String::from("192.0.2.1"),
            port: 22,
            user: String::from("root"),
            auth_kind: AuthKind::Password,
            secret_ref: String::from("t609"),
            key_path: None,
            domain: String::from("stream.example.com"),
            video_dir: String::from("/var/lib/vrcast/videos"),
            cdn_base: None,
            host_fingerprint: None,
            ipv6_mode: None,
            is_active: true,
        },
    )
    .expect("the profile would not be written");
    state
}

/// Is a second deployment and a second upgrade of the same server refused as "already
/// running"? Both, or neither — they are checked together (`running_deploy_for`).
async fn second_run_refused(state: &AppState) -> bool {
    let deploy_again = deploy::deploy_run(state, SERVER, Ipv6Choice::Keep, true, false).await;
    let upgrade_again = deploy::server_upgrade_run(state, SERVER, true).await;
    let refused = |r: &Result<String, AppError>| matches!(r, Err(e) if e.code == ErrorCode::DeployAlreadyRunning);
    assert_eq!(
        refused(&deploy_again),
        refused(&upgrade_again),
        "a deployment and an upgrade of the same server answered differently: \
         {deploy_again:?} / {upgrade_again:?}"
    );
    refused(&deploy_again)
}

async fn wait_for_state(
    state: &AppState,
    id: &str,
    wanted: fn(TaskState) -> bool,
    limit: Duration,
) -> TaskState {
    let deadline = Instant::now() + limit;
    loop {
        let task = core::task_get(state, id).expect("the task vanished");
        if wanted(task.state) {
            return task.state;
        }
        assert!(
            Instant::now() < deadline,
            "the task never reached the state waited for (it is {:?})",
            task.state
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancelled_deployment_is_not_cancelled_and_keeps_the_server_until_the_stop_is_confirmed()
{
    let state = app_state();
    let server_back = Arc::new(AtomicBool::new(false));
    let attempts = Arc::new(AtomicUsize::new(0));

    let id = {
        let server_back = server_back.clone();
        let attempts = attempts.clone();
        state
            .tasks
            .submit(TaskKind::Deploy, Some(SERVER.to_owned()), move |task| async move {
                // The run is under way; the cancel comes; the first stop, through the run's own
                // connection, cannot be confirmed — the connection is gone.
                task.cancel_token().cancelled().await;
                let stop_again = move |_mark: String, _patience: Patience| -> BoxFuture<'static, Result<Stopped, String>> {
                    let server_back = server_back.clone();
                    let attempts = attempts.clone();
                    Box::pin(async move {
                        attempts.fetch_add(1, Ordering::SeqCst);
                        if server_back.load(Ordering::SeqCst) {
                            Ok(Stopped::AlreadyGone)
                        } else {
                            Err(String::from("could not reach the server"))
                        }
                    })
                };
                vrcast_studio_lib::tasks::deploy::confirm_stopped(
                    &task,
                    0.4,
                    "t609-mark",
                    Err(String::from("the connection died before the stop could be asked")),
                    &|| None,
                    &stop_again,
                )
                .await;
                Err(AppError::new(ErrorCode::TaskCancelled))
            })
            .await
            .expect("the task was not submitted")
    };

    wait_for_state(
        &state,
        &id,
        |s| s == TaskState::Running,
        Duration::from_secs(5),
    )
    .await;
    assert!(
        second_run_refused(&state).await,
        "a running deployment did not hold its server"
    );
    state.tasks.cancel(&id).expect("the cancel was refused");

    // Past the first retry (2 s): at least one more attempt has failed by now.
    tokio::time::sleep(Duration::from_millis(3_000)).await;
    let task = core::task_get(&state, &id).unwrap();
    assert_eq!(
        task.state,
        TaskState::Running,
        "the deployment was written down as cancelled although nobody has confirmed that its \
         commands on the server are gone — constitution III"
    );
    assert_eq!(
        task.stage,
        Some(DetailCode::StageStopUnconfirmed),
        "a person looking at a cancelled task still running is owed the reason"
    );
    assert!(
        attempts.load(Ordering::SeqCst) >= 1,
        "the unconfirmed stop was never tried again"
    );
    assert!(
        second_run_refused(&state).await,
        "the server was let go while the cancelled run's commands may still be running on it \
         — a second deployment would meet them at dpkg's lock (T609 phase A)"
    );

    // The server answers again: the next attempt confirms, and only then does it end.
    server_back.store(true, Ordering::SeqCst);
    let ended = wait_for_state(&state, &id, |s| s.is_final(), Duration::from_secs(15)).await;
    assert_eq!(ended, TaskState::Cancelled);
    assert!(
        !second_run_refused(&state).await,
        "the server is still held after the stop was confirmed and the task ended"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_deployment_is_not_failed_and_keeps_the_server_until_the_stop_is_confirmed() {
    // T615. The connection broke in the middle of a step's command: the run comes back with
    // the step's failure, and dpkg of that command may still be running on the server. The
    // same rule as a cancel: nothing final — here `Failed` — until the stop is confirmed.
    let state = app_state();
    let server_back = Arc::new(AtomicBool::new(false));
    let attempts = Arc::new(AtomicUsize::new(0));
    let first_asked = Arc::new(AtomicBool::new(false));

    let id = {
        let server_back = server_back.clone();
        let attempts = attempts.clone();
        let first_asked = first_asked.clone();
        state
            .tasks
            .submit(TaskKind::Deploy, Some(SERVER.to_owned()), move |task| async move {
                let stop_again = move |_mark: String, _patience: Patience| -> BoxFuture<'static, Result<Stopped, String>> {
                    let server_back = server_back.clone();
                    let attempts = attempts.clone();
                    Box::pin(async move {
                        attempts.fetch_add(1, Ordering::SeqCst);
                        if server_back.load(Ordering::SeqCst) {
                            Ok(Stopped::EndedOnItsOwn)
                        } else {
                            Err(String::from("could not reach the server"))
                        }
                    })
                };
                // What the engine hands back when a step's command lost its channel (the
                // shape `deploy_network_cut.rs` measured: a step failure, not an SSH error).
                let outcome: Result<(), DeployError> = Err(DeployError::Step {
                    id: StepId::Packages,
                    detail: String::from("apt failed (no exit status, the channel broke) and said nothing"),
                    advice: None,
                });
                vrcast_studio_lib::tasks::deploy::settle(
                    &task,
                    0.2,
                    "t615-mark",
                    outcome,
                    false,
                    async {
                        first_asked.store(true, Ordering::SeqCst);
                        Err(String::from("the run's own connection is gone"))
                    },
                    &|| None,
                    &stop_again,
                )
                .await
                .map_err(|e| vrcast_studio_lib::tasks::deploy::failed(e, &[]))
            })
            .await
            .expect("the task was not submitted")
    };

    // Past the first retry (2 s): at least one more attempt has failed by now.
    tokio::time::sleep(Duration::from_millis(3_000)).await;
    assert!(
        first_asked.load(Ordering::SeqCst),
        "a failed run was not asked to confirm its stop at all"
    );
    let task = core::task_get(&state, &id).unwrap();
    assert_eq!(
        task.state,
        TaskState::Running,
        "the deployment was written down as failed although nobody has confirmed that its \
         commands on the server are gone — a repeat would meet them at dpkg's lock"
    );
    assert_eq!(task.stage, Some(DetailCode::StageStopUnconfirmed));
    assert!(attempts.load(Ordering::SeqCst) >= 1);
    assert!(
        second_run_refused(&state).await,
        "the server was let go while the failed run's commands may still be running on it"
    );

    server_back.store(true, Ordering::SeqCst);
    let ended = wait_for_state(&state, &id, |s| s.is_final(), Duration::from_secs(15)).await;
    assert_eq!(ended, TaskState::Failed);
    let error = core::task_get(&state, &id)
        .unwrap()
        .error
        .expect("no error was kept");
    assert_eq!(
        error.code,
        ErrorCode::DeployStepFailed,
        "the failure is not the step's own: {error:?}"
    );
    let cause = error.cause.unwrap_or_default();
    assert!(
        cause.contains("channel broke") && !cause.contains("could not reach"),
        "the task failed with the stop's words rather than the step's: {cause}"
    );
    assert!(!second_run_refused(&state).await);
}

#[tokio::test]
async fn a_failure_whose_stop_is_confirmed_at_once_keeps_its_own_error() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    let task = vrcast_studio_lib::tasks::engine::TaskContext::detached(db);
    let asked = Arc::new(AtomicUsize::new(0));
    let asked_in = asked.clone();
    let stop_again = move |_: String, _: Patience| -> BoxFuture<'static, Result<Stopped, String>> {
        asked_in.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(String::from("must not be asked")) })
    };
    let outcome: Result<(), DeployError> = Err(DeployError::NotTaken {
        id: StepId::Firewall,
    });
    let settled = vrcast_studio_lib::tasks::deploy::settle(
        &task,
        0.5,
        "m",
        outcome,
        false,
        async { Ok(Stopped::AlreadyGone) },
        &|| None,
        &stop_again,
    )
    .await;
    assert!(
        matches!(
            settled,
            Err(DeployError::NotTaken {
                id: StepId::Firewall
            })
        ),
        "{settled:?}"
    );
    assert_eq!(asked.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_run_that_ended_well_is_not_stopped_and_a_cancelled_one_is_cancelled() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    let task = vrcast_studio_lib::tasks::engine::TaskContext::detached(db);
    let first_asked = AtomicBool::new(false);
    let well = vrcast_studio_lib::tasks::deploy::settle(
        &task,
        1.0,
        "m",
        Ok(7),
        false,
        async {
            first_asked.store(true, Ordering::SeqCst);
            Err(String::from("must not be asked"))
        },
        &|| None,
        &no_second_try,
    )
    .await;
    assert_eq!(well.ok(), Some(7));
    assert!(
        !first_asked.load(Ordering::SeqCst),
        "a run that ended well was sent a stop"
    );

    // Cancelled on the last step, which had just finished: still `Cancelled`, as in T609.
    let cancelled = vrcast_studio_lib::tasks::deploy::settle(
        &task,
        1.0,
        "m",
        Ok(7),
        true,
        async { Ok(Stopped::AlreadyGone) },
        &|| None,
        &no_second_try,
    )
    .await;
    assert!(
        matches!(cancelled, Err(DeployError::Cancelled)),
        "{cancelled:?}"
    );
    // And a cancelled run that also failed is `Cancelled`, not the failure.
    let both = vrcast_studio_lib::tasks::deploy::settle::<()>(
        &task,
        0.5,
        "m",
        Err(DeployError::NotTaken {
            id: StepId::Firewall,
        }),
        true,
        async { Ok(Stopped::AlreadyGone) },
        &|| None,
        &no_second_try,
    )
    .await;
    assert!(matches!(both, Err(DeployError::Cancelled)), "{both:?}");
}

fn no_second_try(_: String, _: Patience) -> BoxFuture<'static, Result<Stopped, String>> {
    Box::pin(async { Err(String::from("must not be asked")) })
}

#[test]
fn what_an_interrupted_write_left_is_looked_for_by_name_where_writes_happen() {
    let script = leftovers_script();
    assert_eq!(TEMP_SUFFIX, ".vrcast.tmp");
    // By name, never by search (T622): an administrator's `*.vrcast.tmp` elsewhere in /etc is
    // not ours.
    assert!(!script.contains("find "), "{script}");
    assert!(!script.contains('*'), "{script}");
    assert!(
        script.ends_with("; true"),
        "a tidy-up must never fail the run: {script}"
    );
    // Exactly one quoted word per written path, each `<path>.vrcast.tmp`.
    let quoted: Vec<&str> = script.split('\'').skip(1).step_by(2).collect();
    let expected: Vec<String> = TEMP_PLACES
        .iter()
        .map(|p| format!("{p}{TEMP_SUFFIX}"))
        .collect();
    assert_eq!(quoted, expected, "{script}");
}

/// Every path a run writes through a temp file beside it, read from the sources: each
/// `put_file(CONST, …)` (the constant resolved in the same file) and each `{CONST}.vrcast.tmp`
/// a shell script writes itself. Walks all of `src/`, so a new writer anywhere is seen.
fn written_through_a_temp_file() -> std::collections::BTreeSet<String> {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    fn resolve(source: &str, name: &str, file: &std::path::Path) -> String {
        let name = name.trim();
        if let Some(literal) = name.strip_prefix('"') {
            return literal.trim_end_matches('"').to_owned();
        }
        let wanted = format!("const {name}: &str = \"");
        let line = source
            .lines()
            .find(|l| l.contains(&wanted))
            .unwrap_or_else(|| panic!("{}: `{name}` is not a path constant here", file.display()));
        line.split('"').nth(1).unwrap().to_owned()
    }
    let mut files = Vec::new();
    walk(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    let put_file_itself = std::path::Path::new("server").join("deploy").join("mod.rs");
    let mut found = std::collections::BTreeSet::new();
    for file in files {
        let source = std::fs::read_to_string(&file).unwrap();
        for line in source.lines().filter(|l| !l.trim_start().starts_with("//")) {
            if let Some((_, rest)) = line.split_once(".put_file(") {
                let arg = rest.split(',').next().unwrap();
                found.insert(resolve(&source, arg, &file));
            }
            // `put_file` itself writes `{path}{TEMP_SUFFIX}` for whatever it is handed.
            if file.ends_with(&put_file_itself) {
                continue;
            }
            for suffix in [".vrcast.tmp", "{TEMP_SUFFIX}"] {
                let mut rest = line;
                while let Some(at) = rest.find(suffix) {
                    let name = rest[..at]
                        .strip_suffix('}')
                        .and_then(|b| b.rsplit_once('{'))
                        .map(|(_, n)| n)
                        .unwrap_or_else(|| {
                            panic!(
                                "{}: a temp file not named after a path constant: {line}",
                                file.display()
                            )
                        });
                    found.insert(resolve(&source, name, &file));
                    rest = &rest[at + suffix.len()..];
                }
            }
        }
    }
    found
}

#[test]
fn the_tidied_places_are_exactly_the_places_written() {
    let written = written_through_a_temp_file();
    let listed: std::collections::BTreeSet<String> =
        TEMP_PLACES.iter().map(|p| (*p).to_owned()).collect();
    assert_eq!(
        TEMP_PLACES.len(),
        listed.len(),
        "a path is listed twice in TEMP_PLACES"
    );
    // Both directions: a write missing from the list would never be tidied; a listed path
    // nobody writes would be removed on a server where it is not ours.
    assert_eq!(
        written, listed,
        "TEMP_PLACES and the writes in the sources differ"
    );
    assert!(listed.contains("/usr/share/keyrings/caddy-stable-archive-keyring.gpg"));
    assert!(listed.contains("/etc/caddy/Caddyfile"));
}

#[tokio::test]
async fn a_stop_confirmed_the_first_time_is_not_tried_again() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    let task = vrcast_studio_lib::tasks::engine::TaskContext::detached(db);
    let asked = Arc::new(AtomicUsize::new(0));
    let asked_in = asked.clone();
    let stop_again = move |_: String, _: Patience| -> BoxFuture<'static, Result<Stopped, String>> {
        asked_in.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(String::from("must not be asked")) })
    };
    let how = vrcast_studio_lib::tasks::deploy::confirm_stopped(
        &task,
        0.5,
        "m",
        Ok(Stopped::AlreadyGone),
        &|| None,
        &stop_again,
    )
    .await;
    assert_eq!(how, Stopped::AlreadyGone);
    assert_eq!(asked.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_retry_is_patient_with_a_command_that_may_still_be_running() {
    // The first stop failed because the connection died while a command was running; the
    // command may go on for the rest of its ten minutes. The retry must say so to the server
    // (wait, do not signal) rather than kill what might be dpkg.
    let db = Arc::new(Db::open_in_memory().unwrap());
    let task = vrcast_studio_lib::tasks::engine::TaskContext::detached(db);
    let until = Instant::now() + Duration::from_secs(500);
    let seen: Arc<std::sync::Mutex<Vec<Patience>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_in = seen.clone();
    let stop_again = move |_: String, p: Patience| -> BoxFuture<'static, Result<Stopped, String>> {
        seen_in.lock().unwrap().push(p);
        Box::pin(async { Ok(Stopped::EndedOnItsOwn) })
    };
    vrcast_studio_lib::tasks::deploy::confirm_stopped(
        &task,
        0.5,
        "m",
        Err(String::from("gone")),
        &|| Some(until),
        &stop_again,
    )
    .await;
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert!(
        !seen[0].then_signal && seen[0].grace_s == MAX_GRACE_S,
        "a command with minutes left was going to be signalled: {:?}",
        seen[0]
    );
}

#[test]
fn patience_waits_for_what_may_still_run_and_signals_only_once_its_time_is_up() {
    assert_eq!(Patience::for_remaining(Duration::ZERO), Patience::NONE);
    let short = Patience::for_remaining(Duration::from_millis(12_300));
    assert_eq!(short.grace_s, 13);
    assert!(
        short.then_signal,
        "a command whose time runs out inside the wait is signalled then"
    );
    let long = Patience::for_remaining(Duration::from_secs(590));
    assert_eq!(long.grace_s, MAX_GRACE_S);
    assert!(
        !long.then_signal,
        "a command with minutes left must not be signalled"
    );
}

#[test]
fn every_command_of_a_run_carries_its_mark_in_a_group_of_its_own() {
    let run = RunMark::fresh();
    let other = RunMark::fresh();
    assert_ne!(run.as_str(), other.as_str(), "two runs share a mark");
    let wrapped = run.wrap("echo 'it''s' && apt-get install -y x");
    assert!(
        wrapped.starts_with(&format!("{RUN_VAR}='{}' setsid -w ", run.as_str())),
        "the mark or the group is missing: {wrapped}"
    );
    // The command goes through whole, as one argument.
    assert!(
        wrapped.ends_with(r#"-c 'echo '\''it'\'''\''s'\'' && apt-get install -y x'"#),
        "{wrapped}"
    );
    assert!(!run.is_stopping());
    run.ask_to_stop();
    assert!(run.is_stopping());
    assert_eq!(
        run.may_run_until(),
        None,
        "nothing was sent, so nothing may be running"
    );
}

#[test]
fn the_ssh_undo_timer_drops_the_mark() {
    // Read from the step's source rather than trusted: the timer is the one process a run
    // leaves behind on purpose, and a stop aimed at the mark would kill it if it kept it.
    let source = include_str!("../../src/server/deploy/ssh_hardening.rs");
    assert!(
        source.contains("env -u {RUN_VAR} setsid nohup sh -c 'sleep {UNDO_AFTER_SECONDS}"),
        "the SSH undo timer no longer drops the run's mark"
    );
}

#[test]
fn a_stop_told_to_wait_says_running_and_that_is_not_a_confirmation() {
    let script = stop_script(RUN_VAR);
    assert!(
        script.contains(&format!("\"{RUN_VAR}=$WANT\"*)")),
        "the scan looks for another variable"
    );
    assert!(script.contains("AFTER=\"${5:-signal}\""));
    assert!(matches!(
        read_stop("VRCAST_STOP running 812:812\n"),
        StopReport::StillRunning(_)
    ));
    assert_eq!(
        read_stop("VRCAST_STOP ended 5210ms\n"),
        StopReport::Confirmed {
            how: Stopped::EndedOnItsOwn,
            elapsed_ms: Some(5210)
        }
    );
}

#[test]
fn a_failed_apt_step_says_what_apt_said() {
    let said = CommandOutput {
        exit_code: Some(100),
        stdout: String::from("Reading package lists...\n"),
        stderr: String::from(
            "W: some warning\nE: dpkg was interrupted, you must manually run 'dpkg --configure -a' to correct the problem.\n",
        ),
    };
    let text = apt_complaint(&said);
    assert!(text.contains("exit 100"), "{text}");
    assert!(text.contains("dpkg was interrupted"), "{text}");

    let long: String = (1..=20).map(|i| format!("line {i}\n")).collect();
    let text = apt_complaint(&CommandOutput {
        exit_code: None,
        stdout: String::new(),
        stderr: long,
    });
    assert!(
        text.contains("line 20") && !text.contains("line 14\n"),
        "{text}"
    );
    assert!(text.contains("channel broke"), "{text}");
}

#[test]
fn the_repair_runs_only_when_dpkg_has_something_unfinished_and_waits_for_its_lock() {
    assert!(APT_HEAL.contains("dpkg --audit"));
    assert!(APT_HEAL.contains("dpkg --configure -a"));
    assert!(APT_HEAL.contains("-f install -y"));
    assert!(APT_HEAL.contains("Acquire::Retries="));
}
