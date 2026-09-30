//! T647 — a deployment's stop goes to the server the run started on, never to where the
//! profile points now (QA-23 №1).
//!
//! The stop used to read the profile afresh at each attempt and took everything with it: a
//! profile pointed at server B while a run on A was cut off sent the next attempt to B, where
//! nothing carries A's mark — `VRCAST_STOP none`, read as "A's run is gone". Here the run's
//! target is taken as `start` takes it, the profile is then pointed at B (new address, new
//! password, B's fingerprint confirmed), and the production `stop_through_gate` is watched on
//! two loopback listeners. Each takes the TCP connection and closes it at once, before any
//! SSH exchange: nothing is signed in to and no command is run, so this needs neither Docker
//! nor a real server. What is checked is where the attempt goes.
//!
//! The made key after `SshHardening` signing in for real is checked on a container
//! (`tests/integration/deploy_key_kept.rs`).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use vrcast_studio_lib::commands::deploy::{stop_target_of, stop_through_gate};
use vrcast_studio_lib::commands::servers::api as servers;
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::server::gate::StopTarget;
use vrcast_studio_lib::server::marked::Patience;
use vrcast_studio_lib::ssh::Credentials;
use vrcast_studio_lib::store::profiles;

use super::support::{state, valid_input};

const PASSWORD_A: &str = "t647-the-password-the-run-began-with";
const PASSWORD_B: &str = "t647-the-password-of-the-other-server";
const MADE_KEY: &str = "t647-stands-in-for-the-private-key-the-run-made";

/// A listener that counts TCP connections and drops each at once.
async fn counting_listener() -> (u16, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(AtomicUsize::new(0));
    let counter = seen.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            counter.fetch_add(1, Ordering::SeqCst);
            drop(stream);
        }
    });
    (port, seen)
}

/// A profile on A, confirmed; the run's target taken as `start` takes it; then the profile
/// pointed at B — new port, new password, B's fingerprint confirmed.
fn a_run_on_a_then_the_profile_on_b(state: &AppState, port_a: u16, port_b: u16) -> StopTarget {
    let mut input = valid_input("T647");
    input.host = String::from("127.0.0.1");
    input.port = port_a;
    let id = servers::server_add(state, input.clone(), PASSWORD_A).unwrap();
    servers::server_fingerprint_confirm(state, &id, "SHA256:t647-server-A").unwrap();
    let at_start = profiles::get(&state.db, &id).unwrap().unwrap();
    let target = stop_target_of(state.secrets.as_ref(), &at_start)
        .expect("a confirmed profile has a stop target");

    input.port = port_b;
    input.user = String::from("someone-else");
    servers::server_update(state, &id, input, Some(PASSWORD_B)).unwrap();
    servers::server_fingerprint_confirm(state, &id, "SHA256:t647-server-B").unwrap();
    let now = profiles::get(&state.db, &id).unwrap().unwrap();
    assert_eq!(now.port, port_b, "the profile was not pointed at B");
    target
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_stop_goes_to_the_server_the_run_started_on_not_where_the_profile_points_now() {
    let (port_a, seen_a) = counting_listener().await;
    let (port_b, seen_b) = counting_listener().await;
    let state = state();
    let target = a_run_on_a_then_the_profile_on_b(&state, port_a, port_b);

    // The target itself says A, as A's account, by A's fingerprint.
    assert_eq!(target.address().port, port_a);
    assert_eq!(target.user(), "root");
    assert_eq!(target.fingerprint(), "SHA256:t647-server-A");

    // Before `SshKey`: the run's own password only — one attempt, to A.
    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        stop_through_gate(&target, None, "t647-mark", Patience::NONE),
    )
    .await
    .expect("the attempt hung");
    assert!(
        outcome.is_err(),
        "a listener that says nothing confirmed a stop: {outcome:?}"
    );
    // After `SshKey`: the made key first, then the run's own password — both to A.
    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        stop_through_gate(&target, Some(MADE_KEY), "t647-mark", Patience::NONE),
    )
    .await
    .expect("the attempt hung");
    assert!(outcome.is_err());

    // Whatever a late connection would have to say, it has had the time to say it.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (a, b) = (seen_a.load(Ordering::SeqCst), seen_b.load(Ordering::SeqCst));
    println!("T647: run_started_on=A, attempts_on_A={a}, attempts_on_B={b}");
    assert_eq!(
        b, 0,
        "the stop of a run on A went to B, where the profile points now"
    );
    assert_eq!(
        a, 3,
        "one attempt with the run's password, then the made key and the password: all to A"
    );
}

#[tokio::test]
async fn the_run_signs_in_with_what_it_began_with_and_the_made_key_first() {
    let (port_a, _) = counting_listener().await;
    let (port_b, _) = counting_listener().await;
    let state = state();
    let target = a_run_on_a_then_the_profile_on_b(&state, port_a, port_b);

    // Before `SshKey` is on the server: the password the run began with — not B's.
    let ways = target.ways_in(None);
    assert_eq!(ways.len(), 1);
    match &ways[0] {
        Credentials::Password(p) => assert_eq!(p, PASSWORD_A, "not the run's own password"),
        other => panic!("the run began on a password and signs in with {other:?}"),
    }

    // After it (and after `SshHardening`, when the password is refused): the made key first,
    // the run's own password after it.
    let ways = target.ways_in(Some(MADE_KEY));
    assert_eq!(ways.len(), 2);
    match &ways[0] {
        Credentials::KeyText {
            openssh,
            passphrase: None,
        } => assert_eq!(openssh, MADE_KEY),
        other => panic!("the made key is not tried first: {other:?}"),
    }
    assert!(matches!(&ways[1], Credentials::Password(p) if p == PASSWORD_A));
}

#[test]
fn the_run_s_secrets_are_masked_and_never_printed() {
    let _registry = super::basics::alone_with_registry();
    let state = state();
    // Nothing connects here: the ports only have to differ.
    let target = a_run_on_a_then_the_profile_on_b(&state, 40001, 40002);

    let printed = format!("{target:?} {:?}", target.ways_in(Some(MADE_KEY)));
    for secret in [PASSWORD_A, MADE_KEY] {
        assert!(
            !printed.contains(secret),
            "a secret of the run's reached `Debug`: {printed}"
        );
        let masked = vrcast_studio_lib::store::redact::redact(secret);
        assert_ne!(
            masked, secret,
            "the run's secret is not on the redaction list"
        );
    }
}

#[test]
fn a_run_s_own_secret_is_registered_when_the_target_is_taken() {
    let _registry = super::basics::alone_with_registry();
    let state = state();
    let mut input = valid_input("T647 registered");
    input.host = String::from("127.0.0.1");
    let id = servers::server_add(&state, input, PASSWORD_A).unwrap();
    servers::server_fingerprint_confirm(&state, &id, "SHA256:t647").unwrap();
    let profile = profiles::get(&state.db, &id).unwrap().unwrap();
    // Cleared again, so the only thing that can put it back is taking the target from a
    // copy of the credentials the run holds.
    vrcast_studio_lib::store::redact::forget_all();
    let own = Credentials::Password(PASSWORD_A.to_owned());
    let _target = StopTarget::new(&profile, Some(own)).unwrap();
    assert_ne!(
        vrcast_studio_lib::store::redact::redact(PASSWORD_A),
        PASSWORD_A,
        "the run's password is not masked from the moment the run holds it"
    );
}

#[test]
fn no_confirmed_fingerprint_no_target() {
    let state = state();
    let id = servers::server_add(&state, valid_input("T647 unconfirmed"), PASSWORD_A).unwrap();
    let profile = profiles::get(&state.db, &id).unwrap().unwrap();
    assert!(
        stop_target_of(state.secrets.as_ref(), &profile).is_none(),
        "a stop target without a confirmed fingerprint would send credentials unchecked"
    );
}
