//! T356–T358 — removing everything, and saying what that is first (FR-114).
//!
//! What is checked here is the half that can be checked without installing anything: the
//! list shown before the decision, the order the removal happens in, and the warning about
//! the one loss that cannot be undone.
//!
//! The uninstaller's own checkbox is a different mechanism on a different platform, and it is
//! checked where it lives — see `src-tauri/uninstall.nsh` and scenario 10 of the quickstart.

use vrcast_studio_lib::commands::forget::api;
use vrcast_studio_lib::commands::servers::api as servers;
use vrcast_studio_lib::domain::server_profile::AuthKind;
use vrcast_studio_lib::error::ErrorCode;

use super::support::{state, valid_input};

#[test]
fn nothing_is_removed_without_saying_so() {
    // The same rule as everywhere else that cannot be undone: no `confirmed`, no action, and
    // the refusal comes before anything is touched.
    let state = state();
    let refused = api::forget_everything(&state, false);
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

    let went = api::forget_everything(&state, true).expect("removal failed");
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
    let went = api::forget_everything(&state, true).expect("removal failed");
    assert_eq!(went.secrets_removed, 1);
}
