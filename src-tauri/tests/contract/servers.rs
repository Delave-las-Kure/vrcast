//! T035 — contract tests for the server-management commands.
//!
//! The shape of the answer and the error codes are what is checked
//! (`contracts/ipc-commands.md`, "Servers"). A real server is neither needed here nor used:
//! the profiles live in the local database, and the one command that needs a network is
//! checked against a port that is certainly closed — precisely to make sure a failure looks
//! like data rather than like a refusal.

use super::support::{state, valid_input};
use vrcast_studio_lib::commands::error::{DetailCode, ErrorCode};
use vrcast_studio_lib::commands::servers::{api, StepStatus, TEST_STEPS};
use vrcast_studio_lib::domain::server_profile::AuthKind;
use vrcast_studio_lib::store::secrets::SecretRef;

const SECRET: &str = "server-password-for-the-test-9f3a";

#[test]
fn an_empty_profile_list_is_an_empty_list_rather_than_an_error() {
    let s = state();
    let list = api::servers_list(&s).expect("the profile list was not handed back");
    assert!(list.is_empty());
}

#[test]
fn an_added_profile_shows_in_the_list_while_the_secret_goes_to_the_store() {
    let s = state();
    let id =
        api::server_add(&s, valid_input("My server"), SECRET).expect("the profile was not added");

    let list = api::servers_list(&s).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, id);
    assert_eq!(list[0].name, "My server");

    // The profile holds only a pointer to the secret. The secret itself must lie in the
    // operating system's store.
    assert!(
        !list[0].secret_ref.is_empty(),
        "the profile does not point at a secret"
    );
    let stored = s
        .secrets
        .get(&SecretRef::from_stored(&list[0].secret_ref))
        .expect("the secret was not found in the store");
    assert_eq!(stored, SECRET);
}

#[test]
fn a_profile_with_unfit_fields_is_not_created() {
    let s = state();
    let mut input = valid_input("No domain");
    input.domain = String::new();

    let err = api::server_add(&s, input, SECRET).expect_err("a profile with no domain was created");
    assert_eq!(err.code, ErrorCode::InvalidInput);
    // The refusal names exactly the field that was left empty. This used to be checked by
    // a fragment of a Russian word in the text — now it goes by the code, and the check does
    // not depend on which language a person is looking at.
    assert!(
        err.says(DetailCode::DomainEmpty),
        "the refusal does not name the empty domain: {err}"
    );
    assert!(
        err.cause.as_deref().unwrap_or_default().contains("domain"),
        "the details hold no field name for the interface to highlight: {err}"
    );

    assert!(
        api::servers_list(&s).unwrap().is_empty(),
        "the unfit profile was stored after all"
    );
}

#[test]
fn the_secret_of_an_unfit_profile_does_not_stay_in_the_store() {
    // Otherwise, after a failed attempt, entries nothing points at would pile up in the
    // system password manager — and a person would have nothing to delete them with.
    let s = state();
    let mut input = valid_input("No domain");
    input.domain = String::new();
    let _ = api::server_add(&s, input, SECRET);

    let leftovers = s.secrets.get(&SecretRef::for_server("")).is_ok();
    assert!(
        !leftovers,
        "a secret from a profile that was never created was left behind"
    );
}

#[test]
fn two_profiles_with_the_same_name_are_not_set_up() {
    // The name is the only thing a person tells servers apart by in the list.
    let s = state();
    api::server_add(&s, valid_input("One"), SECRET).unwrap();

    let err = api::server_add(&s, valid_input("One"), SECRET)
        .expect_err("a second profile with the same name was set up");
    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert_eq!(api::servers_list(&s).unwrap().len(), 1);
}

#[test]
fn changing_a_profile_without_a_new_secret_leaves_the_old_one_alone() {
    // Otherwise editing the domain would wipe out the password, and a person would find out
    // at their next connection.
    let s = state();
    let id = api::server_add(&s, valid_input("Server"), SECRET).unwrap();
    let reference = SecretRef::from_stored(&api::servers_list(&s).unwrap()[0].secret_ref);

    let mut input = valid_input("Server");
    input.domain = String::from("new.example.com");
    api::server_update(&s, &id, input, None).expect("the profile was not changed");

    assert_eq!(
        s.secrets.get(&reference).expect("the secret vanished"),
        SECRET,
        "the secret was replaced although no new one was passed"
    );
    assert_eq!(api::servers_list(&s).unwrap()[0].domain, "new.example.com");
}

#[test]
fn a_secret_that_is_passed_replaces_the_old_one() {
    let s = state();
    let id = api::server_add(&s, valid_input("Server"), SECRET).unwrap();
    let reference = SecretRef::from_stored(&api::servers_list(&s).unwrap()[0].secret_ref);

    let fresh = "a-different-password-nothing-like-it";
    api::server_update(&s, &id, valid_input("Server"), Some(fresh)).unwrap();

    assert_eq!(s.secrets.get(&reference).unwrap(), fresh);
}

#[test]
fn deleting_a_profile_removes_its_secret_from_the_store_too() {
    // FR-005: deleting a profile, the application forgets the access too. A secret left
    // behind is access to somebody else's server that a person no longer remembers.
    let s = state();
    let id = api::server_add(&s, valid_input("Server"), SECRET).unwrap();
    let reference = SecretRef::from_stored(&api::servers_list(&s).unwrap()[0].secret_ref);

    api::server_remove(&s, &id).expect("the profile was not deleted");

    assert!(api::servers_list(&s).unwrap().is_empty());
    assert!(
        s.secrets.get(&reference).is_err(),
        "the deleted profile's secret stayed in the store"
    );
}

#[test]
fn deleting_twice_is_safe() {
    // The contract, rule 5: repeating the same command does not spoil the result.
    let s = state();
    let id = api::server_add(&s, valid_input("Server"), SECRET).unwrap();

    api::server_remove(&s, &id).unwrap();
    api::server_remove(&s, &id).expect("deleting a second time counted as an error");
}

#[test]
fn exactly_one_profile_is_active() {
    // FR-002. The rule is held by the database rather than by careful code — but the
    // contract must show it.
    let s = state();
    let first = api::server_add(&s, valid_input("First"), SECRET).unwrap();
    let second = api::server_add(&s, valid_input("Second"), SECRET).unwrap();

    api::server_set_active(&s, &first).unwrap();
    let active: Vec<String> = api::servers_list(&s)
        .unwrap()
        .into_iter()
        .filter(|p| p.is_active)
        .map(|p| p.id)
        .collect();
    assert_eq!(active, vec![first.clone()]);

    api::server_set_active(&s, &second).unwrap();
    let active: Vec<String> = api::servers_list(&s)
        .unwrap()
        .into_iter()
        .filter(|p| p.is_active)
        .map(|p| p.id)
        .collect();
    assert_eq!(active, vec![second], "two turned out to be active at once");
}

#[test]
fn a_confirmed_fingerprint_is_remembered() {
    // FR-092: confirming is a one-off act by a person, and it must survive a restart, or
    // they will be asked every time and will stop thinking before confirming.
    let s = state();
    let id = api::server_add(&s, valid_input("Server"), SECRET).unwrap();
    let fp = "SHA256:AbCdEfGhIjKlMnOpQrStUvWxYz0123456789abcdefg";

    api::server_fingerprint_confirm(&s, &id, fp).expect("the fingerprint was not confirmed");

    let profile = api::servers_list(&s).unwrap().remove(0);
    assert_eq!(profile.host_fingerprint.as_deref(), Some(fp));
}

#[test]
fn logging_in_by_key_with_no_path_to_a_key_is_rejected() {
    let s = state();
    let mut input = valid_input("By key");
    input.auth_kind = AuthKind::Key;
    input.key_path = None;

    let err =
        api::server_add(&s, input, SECRET).expect_err("login by key with no key was accepted");
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

#[tokio::test]
async fn the_connection_check_returns_every_step_marked_where_it_stopped() {
    // FR-003. This is the command's main property: a person must see what managed to pass,
    // not only a message about the last trouble. The port is certainly closed — the very
    // first step must fail while the rest arrive marked as not run.
    let s = state();
    let mut input = valid_input("Unreachable");
    input.host = String::from("127.0.0.1");
    input.port = 1;
    let id = api::server_add(&s, input, SECRET).unwrap();

    let steps = api::server_test(&s, &id)
        .await
        .expect("a failed step must not be a refusal of the command: it is data");

    assert_eq!(
        steps.len(),
        TEST_STEPS.len(),
        "not every step came back: {steps:?}"
    );
    let ids: Vec<&str> = steps.iter().map(|x| x.id.as_str()).collect();
    assert_eq!(
        ids,
        TEST_STEPS.to_vec(),
        "the order of the steps was changed"
    );

    assert_eq!(
        steps[0].status,
        StepStatus::Failed,
        "the network is suddenly reachable"
    );
    assert!(
        steps[0].detail.is_some(),
        "a failure with no explanation is useless"
    );
    for step in &steps[1..] {
        assert_eq!(
            step.status,
            StepStatus::Skipped,
            "the step {} ran after the previous one failed",
            step.id
        );
    }
    // The core no longer sends the steps' names: the interface takes them by step id from
    // its own catalogue, so one and the same name cannot drift between screens. What is
    // checked here is what is left of the core — that the step id is recognisable.
    for step in &steps {
        assert!(
            TEST_STEPS.contains(&step.id.as_str()),
            "a step with an unknown id: {}",
            step.id
        );
    }
}

#[tokio::test]
async fn checking_a_profile_that_does_not_exist_is_an_error_rather_than_an_empty_list() {
    let s = state();
    let err = api::server_test(&s, "no-such-server")
        .await
        .expect_err("a profile that does not exist was checked");
    assert_eq!(err.code, ErrorCode::InvalidInput);
}
