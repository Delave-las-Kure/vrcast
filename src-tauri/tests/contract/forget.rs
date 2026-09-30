//! T356–T358 — removing everything, and saying what that is first (FR-114).
//!
//! What is checked here is the half that can be checked without installing anything: the
//! list shown before the decision, the order the removal happens in, and the warning about
//! the one loss that cannot be undone.
//!
//! The uninstaller's own checkbox is a different mechanism on a different platform, and it is
//! checked where it lives — see `src-tauri/uninstall.nsh` and scenario 10 of the quickstart.

use vrcast_studio_lib::commands::forget::{api, ForgetSeen};
use vrcast_studio_lib::commands::servers::api as servers;
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::domain::server_profile::AuthKind;
use vrcast_studio_lib::error::ErrorCode;

use super::support::{state, valid_input};

/// What the screen would hand back: the list as it stands right now (T648). The tests that are
/// not about a stale list agree to the current one, exactly as a person looking at it would.
pub fn seen(state: &AppState) -> ForgetSeen {
    ForgetSeen::from(&api::forget_preview(state).expect("no preview"))
}

#[test]
fn nothing_is_removed_without_saying_so() {
    // The same rule as everywhere else that cannot be undone: no `confirmed`, no action, and
    // the refusal comes before anything is touched.
    let state = state();
    let refused = api::forget_everything(&state, false, &seen(&state));
    assert_eq!(refused.unwrap_err().code, ErrorCode::ConfirmationRequired);
}

#[test]
fn the_list_names_what_would_go() {
    // "Delete my data" without a list is read differently by everybody who reads it, and the
    // person deciding is the one who cannot check afterwards.
    let state = state();
    servers::server_add(&state, valid_input("первый"), "секрет-1")
        .expect("the profile would not be created");
    servers::server_add(&state, valid_input("второй"), "секрет-2")
        .expect("the profile would not be created");

    let would = api::forget_preview(&state).expect("no preview");
    assert_eq!(would.servers.len(), 2);
    assert!(would.servers.contains(&String::from("первый")));
    assert_eq!(would.secrets, 2);
    // The directory is deliberately absent here: this state was never given one, and that is
    // what keeps a test from deleting somebody's real profiles. Where the naming of it is
    // checked is on a state that has one — which is the running application, and scenario 10.
}

#[test]
fn the_servers_that_would_be_lost_for_good_are_named_apart() {
    // **The one loss that cannot be undone.** A server this application deployed refuses
    // passwords, and the only key for it is the one in the operating system's store. Erasing
    // that without a copy means the hosting provider's console and a reinstall.
    //
    // Ordinary profiles must NOT be counted among them: a warning that fires for everybody is
    // a warning nobody reads.
    let state = state();
    servers::server_add(&state, valid_input("по паролю"), "пароль")
        .expect("the profile would not be created");

    let mut managed = valid_input("свой ключ");
    managed.auth_kind = AuthKind::ManagedKey;
    servers::server_add(&state, managed, "ключ").expect("the profile would not be created");

    let would = api::forget_preview(&state).expect("no preview");
    assert_eq!(
        would.locked_out,
        vec![String::from("свой ключ")],
        "the warning has to name the servers that would become unreachable, and only those"
    );
}

#[test]
fn the_secrets_go_and_are_counted() {
    // **Secrets before the directory, and that order is the whole of it.** They are reachable
    // only through the profiles, and the profiles live in the database. Remove the directory
    // first and the entries in the operating system's store are orphans: nothing left knows
    // their names, and after the application is gone there is nobody to clear them.
    let state = state();
    for name in ["первый", "второй", "третий"] {
        servers::server_add(&state, valid_input(name), "секрет")
            .expect("the profile would not be created");
    }

    let went = api::forget_everything(&state, true, &seen(&state)).expect("removal failed");
    assert_eq!(went.secrets_removed, 3);
    assert!(
        went.secrets_left.is_empty(),
        "secrets were left behind: {:?}",
        went.secrets_left
    );
}

#[test]
fn a_test_can_never_reach_a_real_directory() {
    // **The check that exists because the mistake was made.** The first version worked the
    // directory out from the environment, so this very file — running on an in-memory
    // database — deleted the developer's own profiles and both place tables. 133 megabytes,
    // restored from a copy made beforehand.
    //
    // Now the directory is carried on the state and handed over in exactly one place. A run
    // that was not given one removes nothing, and that is settled at construction rather than
    // by whoever calls the removal.
    let state = state();
    assert!(
        state.data_dir.is_none(),
        "a test state has been given a real directory, and removal would delete it"
    );

    servers::server_add(&state, valid_input("останется"), "секрет")
        .expect("the profile would not be created");
    let would = api::forget_preview(&state).expect("no preview");
    assert!(would.data_dir.is_none());
    assert_eq!(would.bytes, 0);

    // And removal still does its other half: the secrets go.
    let went = api::forget_everything(&state, true, &seen(&state)).expect("removal failed");
    assert_eq!(went.secrets_removed, 1);
}

// ---------- T648 (QA-23 №2): only what the person agreed to goes ----------

const MADE_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nmade-by-the-run\n";

#[test]
fn the_qa_path_a_key_made_after_the_look_refuses_the_old_agreement() {
    // The finding: the screen was opened while a deployment had not yet made its key — nothing
    // named as lost; the run finished, the profile turned to `managed_key`, and the removal went
    // on the old agreement, taking the only key to that server with it.
    let state = state();
    let id = servers::server_add(&state, valid_input("свежий"), "пароль")
        .expect("the profile would not be created");
    let looked = seen(&state);
    assert!(looked.locked_out.is_empty());

    let profile = vrcast_studio_lib::store::profiles::get(&state.db, &id)
        .unwrap()
        .unwrap();
    vrcast_studio_lib::commands::deploy::key_keeper(&state, &profile, MADE_KEY.into())()
        .expect("the run could not keep its key");

    let err = api::forget_everything(&state, true, &looked)
        .expect_err("removed the only key on an agreement that never named it");
    assert_eq!(err.code, ErrorCode::ForgetPreviewStale);
    let reference = vrcast_studio_lib::store::secrets::SecretRef::from_stored(
        vrcast_studio_lib::store::profiles::get(&state.db, &id)
            .unwrap()
            .unwrap()
            .secret_ref,
    );
    assert_eq!(
        state.secrets.get(&reference).ok().as_deref(),
        Some(MADE_KEY),
        "the key went although the removal was refused"
    );
    assert!(
        !state.tasks.is_closed_for_forgetting(),
        "a refused removal left the engine closed"
    );

    // Looked at again — now naming the server — the removal goes through.
    let again = seen(&state);
    assert_eq!(again.locked_out, vec![String::from("свежий")]);
    let went = api::forget_everything(&state, true, &again).expect("removal failed");
    assert_eq!(went.secrets_removed, 1);
    assert!(state.secrets.get(&reference).is_err());
}

#[test]
fn a_different_set_of_servers_refuses_and_the_order_does_not() {
    let state = state();
    for name in ["а", "б"] {
        servers::server_add(&state, valid_input(name), "секрет").unwrap();
    }
    let looked = seen(&state);

    // A profile added after the look is one the person never agreed to lose.
    servers::server_add(&state, valid_input("в"), "секрет").unwrap();
    assert_eq!(
        api::forget_everything(&state, true, &looked)
            .unwrap_err()
            .code,
        ErrorCode::ForgetPreviewStale
    );
    assert_eq!(
        vrcast_studio_lib::store::profiles::list(&state.db)
            .unwrap()
            .len(),
        3,
        "profiles went although the removal was refused"
    );

    // Named in another order is still the same list.
    let mut shuffled = seen(&state);
    shuffled.servers.reverse();
    let went = api::forget_everything(&state, true, &shuffled).expect("removal failed");
    assert_eq!(went.secrets_removed, 3);
}

#[test]
fn a_lock_out_the_person_was_warned_of_but_which_is_gone_also_refuses() {
    // The other direction: warned of a loss that is no longer there is also not what is.
    // Stricter than needed for safety, but a list that disagrees with the database is one the
    // screen has to read again either way.
    let state = state();
    servers::server_add(&state, valid_input("пароль"), "секрет").unwrap();
    let mut looked = seen(&state);
    looked.locked_out.push(String::from("пароль"));
    assert_eq!(
        api::forget_everything(&state, true, &looked)
            .unwrap_err()
            .code,
        ErrorCode::ForgetPreviewStale
    );
}

#[test]
fn no_confirmation_is_refused_before_the_list_is_looked_at() {
    let state = state();
    servers::server_add(&state, valid_input("один"), "секрет").unwrap();
    let stale = ForgetSeen::default();
    assert_eq!(
        api::forget_everything(&state, false, &stale)
            .unwrap_err()
            .code,
        ErrorCode::ConfirmationRequired
    );
}

#[test]
fn a_task_alive_is_named_before_a_stale_list() {
    // While a task is going its end may change the list anyway: "stop the tasks first" is the
    // useful answer, not "look again".
    let state = state();
    servers::server_add(&state, valid_input("один"), "секрет").unwrap();
    let claim = state.tasks.claim("deploy:z").unwrap();
    assert_eq!(
        api::forget_everything(&state, true, &ForgetSeen::default())
            .unwrap_err()
            .code,
        ErrorCode::ForgetTasksRunning
    );
    drop(claim);
}

#[test]
fn what_the_screen_sends_is_read_by_the_core() {
    // The shape `ipc.forgetEverything` puts on the wire, read as the command's argument.
    let seen: ForgetSeen =
        serde_json::from_value(serde_json::json!({ "servers": ["а"], "locked_out": [] }))
            .expect("the core cannot read what the screen sends");
    assert_eq!(seen.servers, vec![String::from("а")]);
}
