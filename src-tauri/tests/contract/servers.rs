//! T035 — contract tests for the server-management commands.
//!
//! The shape of the answer and the error codes are what is checked
//! (`contracts/ipc-commands.md`, "Servers"). A real server is neither needed here nor used:
//! the profiles live in the local database, and the one command that needs a network is
//! checked against a port that is certainly closed — precisely to make sure a failure looks
//! like data rather than like a refusal.

use super::support::{state, valid_input};
use vrcast_studio_lib::commands::error::{DetailCode, ErrorCode};
use vrcast_studio_lib::commands::servers::{api, ServerInput, StepStatus, TEST_STEPS};
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

// ---------- editing an address is meeting a different machine (2026-09-05) ----------

/// Confirm a fingerprint the way the wizard does, so the tests below start from a profile
/// that has actually seen a machine.
fn confirmed(s: &vrcast_studio_lib::commands::AppState, id: &str, fingerprint: &str) {
    api::server_fingerprint_confirm(s, id, fingerprint).expect("the fingerprint was not confirmed");
    assert_eq!(
        api::servers_list(s).unwrap()[0].host_fingerprint.as_deref(),
        Some(fingerprint),
        "the test is built wrong: nothing was confirmed, so there is nothing to lose"
    );
}

#[test]
fn editing_a_field_that_is_not_the_address_keeps_the_confirmed_machine() {
    // The other half of the rule, and the reason it is a rule and not a blanket clearing:
    // renaming a profile or pointing it at another directory does not change which computer
    // is at the other end, and making somebody confirm it again for that would teach them to
    // confirm without looking.
    let s = state();
    let id = api::server_add(&s, valid_input("Server"), SECRET).unwrap();
    confirmed(
        &s,
        &id,
        "SHA256:aFingerprintOfTheMachineWeAgreedTo0000000000",
    );

    let mut input = valid_input("Renamed");
    input.domain = String::from("new.example.com");
    api::server_update(&s, &id, input, None).expect("the profile was not changed");

    assert_eq!(
        api::servers_list(&s).unwrap()[0]
            .host_fingerprint
            .as_deref(),
        Some("SHA256:aFingerprintOfTheMachineWeAgreedTo0000000000"),
        "the confirmation was thrown away although the machine never changed"
    );
}

#[test]
fn moving_a_profile_to_another_address_stops_it_claiming_to_have_seen_the_machine() {
    // ⚠ **The fingerprint used to be carried across unconditionally**, with a comment saying
    // confirmation is a deliberate act of a person's own. So it is — about *one machine*. A
    // fingerprint says "this is the computer I looked at and agreed to"; move the address and
    // the profile goes on saying it was confirmed about a computer that is no longer there.
    //
    // What that produced was not a silent connection to the wrong machine — the check refuses
    // a mismatch — but a refusal blaming the server for a changed key, when what changed was
    // the address, with no way out offered (FR-092).
    for (what, edit) in [
        (
            "the host",
            (|i: &mut ServerInput| i.host = String::from("10.0.0.9")) as fn(&mut _),
        ),
        (
            "the port",
            (|i: &mut ServerInput| i.port = 2222) as fn(&mut _),
        ),
    ] {
        let s = state();
        let id = api::server_add(&s, valid_input("Server"), SECRET).unwrap();
        confirmed(
            &s,
            &id,
            "SHA256:aFingerprintOfTheMachineWeAgreedTo0000000000",
        );

        let mut input = valid_input("Server");
        edit(&mut input);
        api::server_update(&s, &id, input, None).expect("the profile was not changed");

        assert_eq!(
            api::servers_list(&s).unwrap()[0].host_fingerprint,
            None,
            "{what} was changed and the profile still claims to have seen that machine"
        );
    }
}

// ---------- T626 (QA-20 №1): a stale form does not undo the key a deployment made ----------

/// A profile as a deployment leaves it after `SshKey` (T616): `managed_key`, the private key
/// in the store under the profile's own reference.
fn managed(s: &vrcast_studio_lib::commands::AppState) -> (String, SecretRef) {
    let mut input = valid_input("Server");
    input.auth_kind = AuthKind::ManagedKey;
    let id = api::server_add(s, input, MADE_KEY).expect("the profile was not added");
    let reference = SecretRef::from_stored(&api::servers_list(s).unwrap()[0].secret_ref);
    (id, reference)
}

const MADE_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nmade-by-the-deployment\n";

#[test]
fn a_stale_password_form_does_not_put_a_managed_key_profile_back_on_password() {
    // The deploy screen (and an edit form opened before the run) held the profile as it was:
    // `auth_kind = password`. Written back without a secret, it left "password" over a store
    // holding the private key, and every later sign-in sent the key as a password.
    for secret in [None, Some("")] {
        let s = state();
        let (id, reference) = managed(&s);

        let err = api::server_update(&s, &id, valid_input("Server"), secret)
            .expect_err("a stale form moved the profile off the made key");
        assert_eq!(err.code, ErrorCode::InvalidInput);
        assert!(
            err.says(DetailCode::ProfileAuthNeedsSecret),
            "the refusal does not say why: {err}"
        );

        let after = &api::servers_list(&s).unwrap()[0];
        assert_eq!(
            after.auth_kind,
            AuthKind::ManagedKey,
            "the profile was changed anyway"
        );
        assert_eq!(
            s.secrets.get(&reference).unwrap(),
            MADE_KEY,
            "the key was touched"
        );
    }
}

#[test]
fn moving_off_the_made_key_with_a_new_secret_is_allowed() {
    // A deliberate change — the person types the password (or the passphrase of their own
    // key): store and profile agree again, so there is nothing to refuse.
    let s = state();
    let (id, reference) = managed(&s);

    api::server_update(&s, &id, valid_input("Server"), Some("a-new-root-password"))
        .expect("a deliberate change of the way of signing in was refused");

    assert_eq!(
        api::servers_list(&s).unwrap()[0].auth_kind,
        AuthKind::Password
    );
    assert_eq!(s.secrets.get(&reference).unwrap(), "a-new-root-password");
}

#[test]
fn editing_other_fields_of_a_managed_key_profile_keeps_its_key() {
    // The ordinary edit of such a profile: its own `auth_kind` sent back, no secret.
    let s = state();
    let (id, reference) = managed(&s);

    let mut input = valid_input("Renamed");
    input.auth_kind = AuthKind::ManagedKey;
    api::server_update(&s, &id, input, None).expect("an ordinary edit was refused");

    let after = &api::servers_list(&s).unwrap()[0];
    assert_eq!(after.name, "Renamed");
    assert_eq!(after.auth_kind, AuthKind::ManagedKey);
    assert_eq!(s.secrets.get(&reference).unwrap(), MADE_KEY);
}

#[test]
fn a_profile_is_not_declared_managed_key_over_a_password() {
    // The other way round: a store holding a password, a profile saying "managed key" — the
    // password would be sent as a key.
    let s = state();
    let id = api::server_add(&s, valid_input("Server"), SECRET).unwrap();

    let mut input = valid_input("Server");
    input.auth_kind = AuthKind::ManagedKey;
    let err = api::server_update(&s, &id, input, None)
        .expect_err("a password profile was declared managed_key without its key");
    assert!(err.says(DetailCode::ProfileAuthNeedsSecret), "{err}");
    assert_eq!(
        api::servers_list(&s).unwrap()[0].auth_kind,
        AuthKind::Password
    );
}

#[test]
fn the_ipv6_choice_is_saved_alone_and_leaves_the_way_of_signing_in_alone() {
    // What the deploy screen calls now instead of `server_update` with the whole profile.
    use vrcast_studio_lib::domain::server_profile::Ipv6Mode;
    let s = state();
    let (id, reference) = managed(&s);

    api::server_set_ipv6_mode(&s, &id, Ipv6Mode::Disable).expect("the choice was not saved");

    let after = &api::servers_list(&s).unwrap()[0];
    assert_eq!(after.ipv6_mode, Some(Ipv6Mode::Disable));
    assert_eq!(after.auth_kind, AuthKind::ManagedKey);
    assert_eq!(after.key_path, None);
    assert_eq!(s.secrets.get(&reference).unwrap(), MADE_KEY);

    api::server_set_ipv6_mode(&s, &id, Ipv6Mode::Keep).unwrap();
    assert_eq!(
        api::servers_list(&s).unwrap()[0].ipv6_mode,
        Some(Ipv6Mode::Keep)
    );

    let err = api::server_set_ipv6_mode(&s, "srv_nobody", Ipv6Mode::Keep)
        .expect_err("a choice was saved for a profile that does not exist");
    assert!(err.says(DetailCode::ProfileNotFound), "{err}");
}

// ---------- T636 (QA-21 №3): the check and the write are one step ----------

/// What a deployment does to a password profile after `SshKey` (T616), through the very
/// function the run uses: the made key into the store, the profile onto `managed_key`.
fn keep_made_key(s: &vrcast_studio_lib::commands::AppState, id: &str) -> Result<(), String> {
    let profile = vrcast_studio_lib::store::profiles::get(&s.db, id)
        .unwrap()
        .expect("the profile vanished");
    vrcast_studio_lib::commands::deploy::key_keeper(s, &profile, MADE_KEY.to_owned())()
}

#[test]
fn a_switch_to_the_made_key_between_the_check_and_the_write_is_not_written_over() {
    // QA-21 №3, the interleaving: the form reads `password`, the check "password → password,
    // no secret" passes, the deployment keeps its key, and the form's old `password` used to
    // be written over it — every step succeeding. `between` stands where the switch landed;
    // it goes around the application's own lock the way only another process could, which is
    // exactly what the conditional write is there for.
    for secret in [None, Some("")] {
        let s = state();
        let id = api::server_add(&s, valid_input("Server"), SECRET).unwrap();
        let reference = SecretRef::from_stored(&api::servers_list(&s).unwrap()[0].secret_ref);

        let at_start = vrcast_studio_lib::store::profiles::get(&s.db, &id)
            .unwrap()
            .unwrap();

        let mut renamed = valid_input("Renamed");
        renamed.auth_kind = AuthKind::Password;
        let err = api::server_update_between(&s, &id, renamed, secret, &mut || {
            s.secrets.set(&reference, MADE_KEY).unwrap();
            assert!(
                vrcast_studio_lib::store::profiles::switch_to_managed_key(&s.db, &at_start)
                    .unwrap(),
                "the switch in between did not happen"
            );
        })
        .expect_err("a check made against `password` wrote over a profile now on managed_key");
        assert_eq!(err.code, ErrorCode::InvalidInput);
        assert!(err.says(DetailCode::ProfileAuthNeedsSecret), "{err}");

        let after = &api::servers_list(&s).unwrap()[0];
        assert_eq!(
            after.auth_kind,
            AuthKind::ManagedKey,
            "the switch was undone"
        );
        assert_eq!(after.name, "Server", "the stale form was written anyway");
        assert_eq!(s.secrets.get(&reference).unwrap(), MADE_KEY);
    }
}

#[test]
fn a_new_password_typed_while_the_key_was_kept_is_checked_again_and_then_agrees() {
    // The other outcome of the same interleaving: the person typed a new password. Re-checked
    // against `managed_key`, that is a deliberate move off the made key with a secret — allowed,
    // and the profile and the store agree on the password.
    let s = state();
    let id = api::server_add(&s, valid_input("Server"), SECRET).unwrap();
    let reference = SecretRef::from_stored(&api::servers_list(&s).unwrap()[0].secret_ref);
    let at_start = vrcast_studio_lib::store::profiles::get(&s.db, &id)
        .unwrap()
        .unwrap();

    api::server_update_between(
        &s,
        &id,
        valid_input("Server"),
        Some("a-new-root-password"),
        &mut || {
            s.secrets.set(&reference, MADE_KEY).unwrap();
            vrcast_studio_lib::store::profiles::switch_to_managed_key(&s.db, &at_start).unwrap();
        },
    )
    .expect("a deliberate password was refused");
    assert_eq!(
        api::servers_list(&s).unwrap()[0].auth_kind,
        AuthKind::Password
    );
    assert_eq!(s.secrets.get(&reference).unwrap(), "a-new-root-password");
}

#[test]
fn the_deployment_keeps_its_key_only_over_a_password_profile() {
    // `switch_to_managed_key` is conditional too: a profile the person has since pointed at a
    // key file of their own is not taken back by a run that started on its password.
    let s = state();
    let id = api::server_add(&s, valid_input("Server"), SECRET).unwrap();
    let reference = SecretRef::from_stored(&api::servers_list(&s).unwrap()[0].secret_ref);
    let profile_at_start = vrcast_studio_lib::store::profiles::get(&s.db, &id)
        .unwrap()
        .unwrap();

    let mut own_key = valid_input("Server");
    own_key.auth_kind = AuthKind::Key;
    own_key.key_path = Some(String::from("C:/keys/id_ed25519"));
    api::server_update(&s, &id, own_key, Some("my-passphrase")).unwrap();

    let keep =
        vrcast_studio_lib::commands::deploy::key_keeper(&s, &profile_at_start, MADE_KEY.into());
    keep().expect_err("the run took back a profile that is no longer on a password");

    let after = &api::servers_list(&s).unwrap()[0];
    assert_eq!(after.auth_kind, AuthKind::Key);
    assert_eq!(after.key_path.as_deref(), Some("C:/keys/id_ed25519"));
    assert_eq!(s.secrets.get(&reference).unwrap(), "my-passphrase");
}

#[test]
fn the_deployment_switches_only_the_way_of_signing_in_not_the_rest_of_its_old_copy() {
    // The run's copy of the profile is from its start; a rename since then stays.
    let s = state();
    let id = api::server_add(&s, valid_input("Server"), SECRET).unwrap();
    let profile_at_start = vrcast_studio_lib::store::profiles::get(&s.db, &id)
        .unwrap()
        .unwrap();
    api::server_update(&s, &id, valid_input("Renamed during the run"), None).unwrap();

    vrcast_studio_lib::commands::deploy::key_keeper(&s, &profile_at_start, MADE_KEY.into())()
        .expect("the key was not kept");

    let after = &api::servers_list(&s).unwrap()[0];
    assert_eq!(after.auth_kind, AuthKind::ManagedKey);
    assert_eq!(after.key_path, None);
    assert_eq!(after.name, "Renamed during the run");
}

#[test]
fn a_form_and_a_deployment_on_two_threads_never_leave_password_over_the_key() {
    // The real race, many times over: whichever lands first, the profile and the store agree.
    for _ in 0..50 {
        let s = std::sync::Arc::new(state());
        let id = api::server_add(&s, valid_input("Server"), SECRET).unwrap();
        let reference = SecretRef::from_stored(&api::servers_list(&s).unwrap()[0].secret_ref);

        let (s1, id1) = (s.clone(), id.clone());
        let form = std::thread::spawn(move || {
            api::server_update(&s1, &id1, valid_input("Renamed"), None).is_ok()
        });
        let (s2, id2) = (s.clone(), id.clone());
        let run = std::thread::spawn(move || keep_made_key(&s2, &id2).is_ok());
        let _ = form.join().unwrap();
        assert!(run.join().unwrap(), "the deployment could not keep its key");

        let after = &api::servers_list(&s).unwrap()[0];
        assert_eq!(after.auth_kind, AuthKind::ManagedKey);
        assert_eq!(s.secrets.get(&reference).unwrap(), MADE_KEY);
    }
}

// ---------- T642 (QA-22 №2): the key is kept only for the server it was made for ----------

/// The run's copy of the profile at its start, as `commands::deploy::start` takes it.
fn profile_now(
    s: &vrcast_studio_lib::commands::AppState,
    id: &str,
) -> vrcast_studio_lib::domain::server_profile::ServerProfile {
    vrcast_studio_lib::store::profiles::get(&s.db, id)
        .unwrap()
        .expect("the profile vanished")
}

#[test]
fn a_profile_pointed_at_another_server_during_the_run_does_not_get_the_old_servers_key() {
    // QA-22 №2: the run started for server A; meanwhile the person moved the same profile to
    // server B (another address, port or user), left it on a password and typed B's password.
    // The key made for A used to be written over B's password and the profile switched to
    // `managed_key` — B out of reach. Now the key is not kept, B's password stays, and the
    // error is what stops the run before `SshHardening` (the run turns a keeper's error into
    // a failed `SshHardening` before sending anything of that step).
    type Move = (&'static str, fn(&mut ServerInput));
    let moves: [Move; 3] = [
        ("address", |i| i.host = String::from("198.51.100.20")),
        ("port", |i| i.port = 2222),
        ("user", |i| i.user = String::from("deploy")),
    ];
    for (what, point_elsewhere) in moves {
        let s = state();
        let id = api::server_add(&s, valid_input("Server"), SECRET).unwrap();
        let reference = SecretRef::from_stored(&api::servers_list(&s).unwrap()[0].secret_ref);
        let at_start = profile_now(&s, &id);

        let mut b = valid_input("Server");
        point_elsewhere(&mut b);
        api::server_update(&s, &id, b.clone(), Some("password-of-server-b"))
            .unwrap_or_else(|e| panic!("moving the {what} was refused: {e}"));

        let err =
            vrcast_studio_lib::commands::deploy::key_keeper(&s, &at_start, MADE_KEY.to_owned())()
                .expect_err("the key made for A was kept over a profile moved to B");
        assert!(err.contains("another server"), "{what}: {err}");

        let after = &api::servers_list(&s).unwrap()[0];
        assert_eq!(after.auth_kind, AuthKind::Password, "{what}");
        assert_eq!(
            (&after.host, after.port, &after.user),
            (&b.host, b.port, &b.user),
            "{what}"
        );
        assert_eq!(
            s.secrets.get(&reference).unwrap(),
            "password-of-server-b",
            "{what}: B's password was written over"
        );
    }
}

#[test]
fn the_switch_itself_is_conditional_on_the_server_the_run_started_on() {
    // The `UPDATE` holds the same condition, for a change landing outside this application's
    // lock (another copy on the same database): nothing is switched.
    let s = state();
    let id = api::server_add(&s, valid_input("Server"), SECRET).unwrap();
    let at_start = profile_now(&s, &id);
    let mut b = valid_input("Server");
    b.host = String::from("198.51.100.20");
    api::server_update(&s, &id, b, None).unwrap();

    assert!(
        !vrcast_studio_lib::store::profiles::switch_to_managed_key(&s.db, &at_start).unwrap(),
        "a profile moved to another server was switched to managed_key"
    );
    assert_eq!(
        api::servers_list(&s).unwrap()[0].auth_kind,
        AuthKind::Password
    );
}

#[test]
fn a_rename_and_other_fields_changed_during_the_run_do_not_stop_the_key_being_kept() {
    // The owner's decision 2026-09-30: only the address, port and user matter.
    let s = state();
    let id = api::server_add(&s, valid_input("Server"), SECRET).unwrap();
    let reference = SecretRef::from_stored(&api::servers_list(&s).unwrap()[0].secret_ref);
    let at_start = profile_now(&s, &id);

    let mut edited = valid_input("Renamed during the run");
    edited.domain = String::from("other.example.com");
    edited.video_dir = Some(String::from("/srv/other"));
    edited.cdn_base = Some(String::from("https://cdn.example.com"));
    api::server_update(&s, &id, edited, None).unwrap();

    vrcast_studio_lib::commands::deploy::key_keeper(&s, &at_start, MADE_KEY.to_owned())()
        .expect("the key was not kept after a rename");

    let after = &api::servers_list(&s).unwrap()[0];
    assert_eq!(after.auth_kind, AuthKind::ManagedKey);
    assert_eq!(after.name, "Renamed during the run");
    assert_eq!(after.domain, "other.example.com");
    assert_eq!(s.secrets.get(&reference).unwrap(), MADE_KEY);
}

// ---------- T638 (QA-21 №5): off the made key onto a key file with no passphrase ----------

const OWN_KEY: &str = "C:/keys/id_ed25519";

fn own_key_file(name: &str) -> ServerInput {
    let mut input = valid_input(name);
    input.auth_kind = AuthKind::Key;
    input.key_path = Some(String::from(OWN_KEY));
    input
}

#[test]
fn moving_off_the_made_key_to_a_key_file_without_a_passphrase_is_allowed_and_clears_the_key() {
    // The owner's decision 2026-09-30: `secret = ""` is "the file has no passphrase". The made
    // private key is written over — it does not stay in the store to be handed to the file as
    // its passphrase.
    let s = state();
    let (id, reference) = managed(&s);

    api::server_update(&s, &id, own_key_file("Server"), Some(""))
        .expect("a key file without a passphrase was refused");

    let after = &api::servers_list(&s).unwrap()[0];
    assert_eq!(after.auth_kind, AuthKind::Key);
    assert_eq!(after.key_path.as_deref(), Some(OWN_KEY));
    assert_eq!(
        s.secrets.get(&reference).unwrap(),
        "",
        "the made key stayed in the store"
    );
}

#[test]
fn moving_off_the_made_key_to_a_key_file_with_a_passphrase_stores_the_passphrase() {
    let s = state();
    let (id, reference) = managed(&s);

    api::server_update(&s, &id, own_key_file("Server"), Some("my-passphrase")).unwrap();

    assert_eq!(api::servers_list(&s).unwrap()[0].auth_kind, AuthKind::Key);
    assert_eq!(s.secrets.get(&reference).unwrap(), "my-passphrase");
}

#[test]
fn moving_off_the_made_key_to_a_key_file_with_no_secret_at_all_is_still_refused() {
    // `null` is "leave the store as it is" — and what it holds is the made private key, which a
    // `key` profile would take for the file's passphrase.
    let s = state();
    let (id, reference) = managed(&s);

    let err = api::server_update(&s, &id, own_key_file("Server"), None)
        .expect_err("the made key was left in the store under a key-file profile");
    assert!(err.says(DetailCode::ProfileAuthNeedsSecret), "{err}");
    assert_eq!(
        api::servers_list(&s).unwrap()[0].auth_kind,
        AuthKind::ManagedKey
    );
    assert_eq!(s.secrets.get(&reference).unwrap(), MADE_KEY);
}

#[test]
fn an_empty_secret_is_no_password_the_stale_form_is_still_refused() {
    // The exception is for the file's passphrase alone: an empty password is not a password,
    // and a stale form's `password` stays refused whichever absence it sends.
    let s = state();
    let (id, reference) = managed(&s);
    for secret in [None, Some("")] {
        let err = api::server_update(&s, &id, valid_input("Server"), secret)
            .expect_err("managed_key -> password without a password");
        assert!(err.says(DetailCode::ProfileAuthNeedsSecret), "{err}");
    }
    // Nor does it open a way onto the made key: to `managed_key` only a deployment moves.
    let other = api::server_add(&s, own_key_file("Other"), "a-passphrase").unwrap();
    let mut to_managed = valid_input("Other");
    to_managed.auth_kind = AuthKind::ManagedKey;
    let err = api::server_update(&s, &other, to_managed, Some(""))
        .expect_err("key -> managed_key with an empty secret");
    assert!(err.says(DetailCode::ProfileAuthNeedsSecret), "{err}");

    assert_eq!(s.secrets.get(&reference).unwrap(), MADE_KEY);
}
