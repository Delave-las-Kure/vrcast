//! T621 — two `deploy_run`/`server_upgrade_run` calls for the same server that overlap
//! **before either has a task** — the second is refused as `DEPLOY_ALREADY_RUNNING`.
//!
//! **Why this is the race itself and not a model of it.** The window QA-19 №6 named lies
//! between `start()`'s guard and its `submit`: `gate::open` (a real SSH connection) and the
//! DNS look. Here the first call is really inside that window — the real `start()`, through
//! its guard, into `gate::open`, where it sits connecting to a local listener that accepts
//! the TCP connection and then says nothing (no SSH banner) until the test lets it go. No
//! task exists yet — the test checks that — so the old scan of the task list had nothing to
//! see, and the second call would have walked into `gate::open` after it (the test would see
//! it hang instead of being refused). Neither Docker nor DNS is needed for this: what is
//! checked is local — the claim in `TaskEngine` — and the delay between the check and the
//! submit is the production code's own, held open for as long as the test needs.
//!
//! Also checked: the claim is given back when the call fails before its task exists (the
//! connection broke), so a failed attempt does not block the next one; and a claim moved into
//! a task's work is given back when that work ends.

use std::sync::Arc;
use std::time::Duration;

use vrcast_studio_lib::commands::api as core;
use vrcast_studio_lib::commands::deploy::api as deploy;
use vrcast_studio_lib::commands::error::ErrorCode;
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::domain::dns_verdict::Ipv6Choice;
use vrcast_studio_lib::domain::server_profile::{AuthKind, ServerProfile};
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::{InMemorySecretStore, SecretRef};
use vrcast_studio_lib::tasks::state::TaskKind;

const SERVER: &str = "t621-server";

/// A listener that takes the TCP connection and then stays silent — an SSH client on it waits
/// for the banner. Dropping the returned sender closes every connection taken.
async fn silent_listener() -> (u16, tokio::sync::oneshot::Sender<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (close_tx, mut close_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        let mut held = Vec::new();
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    if let Ok((stream, _)) = accepted {
                        held.push(stream);
                    }
                }
                _ = &mut close_rx => return, // drops `held` and the listener
            }
        }
    });
    (port, close_tx)
}

fn app_state(port: u16) -> AppState {
    let state = AppState::with_db(
        Arc::new(Db::open_in_memory().unwrap()),
        Arc::new(InMemorySecretStore::new()),
    )
    .expect("the application state would not assemble");
    state
        .secrets
        .set(&SecretRef::from_stored("t621"), "not-a-real-password")
        .unwrap();
    vrcast_studio_lib::store::profiles::insert(
        &state.db,
        &ServerProfile {
            id: SERVER.to_owned(),
            name: String::from("T621"),
            host: String::from("127.0.0.1"),
            port,
            user: String::from("root"),
            auth_kind: AuthKind::Password,
            secret_ref: String::from("t621"),
            key_path: None,
            domain: String::from("stream.example.com"),
            video_dir: String::from("/var/lib/vrcast/videos"),
            cdn_base: None,
            // Confirmed, so `connect_raw` goes on to the network rather than refusing at once.
            host_fingerprint: Some(String::from("SHA256:t621-never-compared")),
            ipv6_mode: None,
            is_active: true,
        },
    )
    .expect("the profile would not be written");
    state
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_overlapping_starts_before_either_has_a_task_the_second_is_refused() {
    let (port, close) = silent_listener().await;
    let state = app_state(port);

    // The first call: through the guard, into `gate::open`, and held there.
    let first = {
        let state = state.clone();
        tokio::spawn(async move {
            deploy::deploy_run(&state, SERVER, Ipv6Choice::Keep, true, false).await
        })
    };
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !first.is_finished(),
        "the first call did not stay in the window"
    );
    assert!(
        core::tasks_list(&state).unwrap().is_empty(),
        "a task exists already — the window between the check and the submit is not being tested"
    );

    // The second — and a second of the other kind — overlapping it: refused, at once.
    for (what, second) in [
        (
            "deploy",
            tokio::time::timeout(
                Duration::from_secs(5),
                deploy::deploy_run(&state, SERVER, Ipv6Choice::Keep, true, false),
            )
            .await,
        ),
        (
            "upgrade",
            tokio::time::timeout(
                Duration::from_secs(5),
                deploy::server_upgrade_run(&state, SERVER, true),
            )
            .await,
        ),
    ] {
        let second = second.unwrap_or_else(|_| {
            panic!("the overlapping {what} was not refused: it went on to connect like the first")
        });
        let err = second.expect_err("the overlapping call was let through");
        assert_eq!(err.code, ErrorCode::DeployAlreadyRunning, "{what}: {err:?}");
        // No task yet on the other side, so nothing to name.
        assert_eq!(err.cause, None, "{what}");
    }

    // The first call's connection breaks before it has a task: it fails, for its own reason…
    drop(close);
    let first = tokio::time::timeout(Duration::from_secs(10), first)
        .await
        .expect("the first call did not end once its connection closed")
        .unwrap();
    let err = first.expect_err("the first call cannot succeed against a silent listener");
    assert_ne!(err.code, ErrorCode::DeployAlreadyRunning);
    assert!(core::tasks_list(&state).unwrap().is_empty());

    // …and the claim went with it: the next attempt is not refused as "already running".
    let again = tokio::time::timeout(
        Duration::from_secs(10),
        deploy::deploy_run(&state, SERVER, Ipv6Choice::Keep, true, false),
    )
    .await
    .expect("the next attempt hung");
    let err = again.expect_err("nothing listens any more");
    assert_ne!(
        err.code,
        ErrorCode::DeployAlreadyRunning,
        "a failed attempt kept the server's claim: {err:?}"
    );
}

#[tokio::test]
async fn a_claim_is_one_holder_at_a_time_and_a_task_gives_it_back_when_its_work_ends() {
    let state = app_state(1);
    let claim = state
        .tasks
        .claim("deploy:x")
        .expect("a free key was not given");
    assert!(
        state.tasks.claim("deploy:x").is_none(),
        "the same key was given twice"
    );
    assert!(
        state.tasks.claim("deploy:y").is_some(),
        "another key was blocked"
    );

    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    let id = state
        .tasks
        .submit(TaskKind::Deploy, None, move |_ctx| async move {
            let _claim = claim;
            let _ = release_rx.await;
            Ok(())
        })
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        state.tasks.claim("deploy:x").is_none(),
        "given back while the work runs"
    );

    let _ = release_tx.send(());
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !core::task_get(&state, &id).unwrap().state.is_final() {
        assert!(
            std::time::Instant::now() < deadline,
            "the stand-in task never ended"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        state.tasks.claim("deploy:x").is_some(),
        "not given back when the work ended"
    );
}
