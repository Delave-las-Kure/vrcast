//! T593 — `deploy_run`/`server_upgrade_run` refuse a second concurrent call for the same
//! server, the same pattern `running_build_for` (T591, `commands/ladder.rs`) already gives
//! `ladder_build` and `running_upload_for` gives `upload_start`, for the same reason: this is
//! not the last line of defence — two clicks at the very same instant would both pass — but it
//! closes the realistic case, a repeat click on "agree, deploy" before `running` reaches the
//! UI. Found by an independent QA audit 2026-09-11 (round 15): `deploy_run`/`server_upgrade_run`
//! were the one pair of expensive, real-machine operations (`Lane::Network`, limit=1) that only
//! *serialised* a second call instead of refusing it — the second deploy would queue and then
//! run in full, redoing user creation, the firewall and the SSH key copy on the same server.
//!
//! **Why the stand-in task, and not a real second `deploy_run`.** `start()` calls
//! `look_at_domain`, which calls `dns::look_up` with `DEFAULT_PATIENCE = 30s`
//! (`src-tauri/src/net/dns.rs`) — the retry loop runs the full patience for any domain with no
//! matching A record, which a profile pointed at a throwaway container always is. So driving
//! two genuine `deploy_run` calls through a real domain would cost 30 real seconds per call for
//! no benefit: the guard sits *before* `gate::open`/DNS specifically so a doomed second call
//! never pays for either (see the comment on `running_deploy_for` and on `start()` itself,
//! `commands/deploy.rs`). What is checked here instead is the guard itself, on a real task-engine
//! slot: `state.tasks.submit(TaskKind::Deploy, ...)` is the exact same submission path a real
//! deploy takes, just with a stand-in body that waits on a `oneshot` instead of running the real
//! steps — real task engine, real guard, real profile on a real container, zero DNS wait.
//!
//! **Why there is no second test for "a different `server_id` is not blocked".**
//! `running_deploy_for` is a private function (`commands/deploy.rs`) with no test-only escape
//! hatch, and it should not get one just to be poked directly (T593's own brief: do not bend the
//! architecture for this). Driving it through a real second `deploy_run`/`server_upgrade_run` for
//! a different profile would need that profile's domain to resolve; a domain that plainly does
//! not (any throwaway container's) pays the same 30-second DNS wait described above for no extra
//! guard coverage — it would only be re-proving the DNS finding, not the scoping. The scoping
//! itself — `task.server_id.as_deref() != Some(server_id)` — is a one-line equality check read
//! directly off `commands/deploy.rs`, exercised the same way `running_build_for`'s identical line
//! is exercised in `ladder_build_race.rs`'s own "different slug" test; duplicating that pattern
//! here across a second Docker container was judged not worth the added runtime for a check this
//! mechanical. If `running_deploy_for` grows any conditional beyond the id comparison, that is
//! the point to add a companion unit test alongside it.
//!
//! Modelled on `ladder_build_race.rs` (T591): its `wait_for_final` is copied rather than
//! imported, for the same reason given there — each fixture file stays free to change its own
//! shape without disturbing the others.

use std::time::Duration;

use vrcast_studio_lib::commands::deploy::api as deploy;
use vrcast_studio_lib::commands::error::ErrorCode;
use vrcast_studio_lib::commands::servers::{api as servers, ServerInput};
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::domain::dns_verdict::Ipv6Choice;
use vrcast_studio_lib::domain::server_profile::AuthKind;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;
use vrcast_studio_lib::tasks::state::TaskKind;

use super::deploy_fixture::{DeployTarget, Flavour, ROOT_PASSWORD};

fn app_state() -> AppState {
    AppState::with_db(
        std::sync::Arc::new(Db::open_in_memory().unwrap()),
        std::sync::Arc::new(InMemorySecretStore::new()),
    )
    .expect("the application state would not assemble")
}

/// A profile pointed at a bare container, fingerprint confirmed — enough for `start()` to get
/// past `profile_of` and, were the guard not there, all the way to `gate::open`.
///
/// `Flavour::Clean` and a password profile at `127.0.0.1`, not `TestServer`/`confirm_fingerprint`
/// from `fixture.rs`/`library_ops.rs`: those fixtures assume an already-deployed server (a
/// different image), and what a fresh `deploy_run` is actually called against is a bare one —
/// see `deploy_clean.rs::a_server_reachable_only_by_password_can_be_planned_from_the_screen`,
/// which this setup is lifted from.
async fn setup() -> (DeployTarget, AppState, String) {
    let target = DeployTarget::start(Flavour::Clean).expect("the bare container would not come up");
    let state = app_state();

    let id = servers::server_add(
        &state,
        ServerInput {
            name: String::from("T593 container"),
            host: String::from("127.0.0.1"),
            port: target.port,
            user: String::from("root"),
            auth_kind: AuthKind::Password,
            key_path: None,
            domain: String::from("stream.example.com"),
            video_dir: None,
            cdn_base: None,
            ipv6_mode: None,
        },
        ROOT_PASSWORD,
    )
    .expect("the profile was not created");

    let seen = vrcast_studio_lib::commands::api::server_probe_fingerprint("127.0.0.1", target.port)
        .await
        .expect("the fingerprint was not obtained");
    servers::server_fingerprint_confirm(&state, &id, &seen)
        .expect("the fingerprint was not confirmed");

    (target, state, id)
}

/// Wait until `running_deploy_for` would no longer count this task — i.e. `is_final()`.
async fn wait_for_final(state: &AppState, task_id: &str, limit: Duration) {
    use vrcast_studio_lib::commands::api as core;
    let deadline = std::time::Instant::now() + limit;
    loop {
        let task = core::task_get(state, task_id).expect("the task vanished from the list");
        if task.state.is_final() {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "task '{task_id}' never reached a final state in the time allowed (state: {:?})",
            task.state
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_second_deploy_of_the_same_server_is_refused_while_the_first_runs() {
    let (_target, state, id) = setup().await; // kept in _target: the container must outlive the calls below

    // A real slot in the task engine — the exact path a real `deploy_run` takes to occupy one
    // — but the work inside waits on the test's own signal instead of running the real steps
    // (see the module doc on why: without this, either every call pays 30s of DNS, or the guard
    // is never checked against a genuinely occupied slot at all).
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    let task_id_1 = state
        .tasks
        .submit(TaskKind::Deploy, Some(id.clone()), move |_ctx| async move {
            let _ = release_rx.await;
            Ok(())
        })
        .await
        .expect("could not submit the stand-in task");

    // Refused, and fast: the guard sits before `gate::open`/DNS, so this must not cost a
    // network trip at all.
    let started = std::time::Instant::now();
    let err = deploy::deploy_run(&state, &id, Ipv6Choice::Keep, true)
        .await
        .expect_err("a second concurrent deploy_run went through");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the refusal took {:?} — the guard did not sit before the network work",
        started.elapsed()
    );
    assert_eq!(err.code, ErrorCode::DeployAlreadyRunning);

    // One guard for both kinds: an upgrade of the SAME server is refused too, while a deploy of
    // it is in flight.
    let err2 = deploy::server_upgrade_run(&state, &id, true)
        .await
        .expect_err("an upgrade went through while a deploy of the same server was running");
    assert_eq!(err2.code, ErrorCode::DeployAlreadyRunning);

    // The guard lifts once the task actually reaches a final state.
    let _ = release_tx.send(());
    wait_for_final(&state, &task_id_1, Duration::from_secs(10)).await;
}
