//! T289 — the single door, checked on every combination.
//!
//! This is the rule that keeps the application from being able to break somebody else's
//! machine, and it is exactly the kind that fails by succeeding: a hole here does not throw,
//! it lets the work through. So every combination of what a server is and what it is being
//! asked for is walked, rather than the interesting ones.

use vrcast_studio_lib::domain::server_state::{
    unreachable, Compat, ForeignReason, Kind, ServerState, APP_EXPECTS, APP_MIN_SUPPORTED,
};
use vrcast_studio_lib::server::gate::{allowed, Intent, Refusal};

fn server(kind: Kind, compat: Compat) -> ServerState {
    ServerState {
        kind,
        server_version: Some(APP_EXPECTS),
        app_expects: APP_EXPECTS,
        app_min_supported: APP_MIN_SUPPORTED,
        compat,
        upgrade_available: false,
        foreign_reason: match kind {
            Kind::Foreign => Some(ForeignReason::ConfigWithoutState),
            _ => None,
        },
    }
}

#[test]
fn nothing_may_change_a_server_that_is_not_ours() {
    // FR-132, and the reason this module exists. Reading is allowed — that is how a person
    // finds out the machine is somebody else's in the first place.
    let theirs = server(Kind::Foreign, Compat::Unknown);
    assert!(allowed(&theirs, Intent::Read).is_ok());
    assert!(matches!(
        allowed(&theirs, Intent::Change),
        Err(Refusal::Foreign { .. })
    ));
    assert!(matches!(
        allowed(&theirs, Intent::Setup),
        Err(Refusal::Foreign { .. })
    ));
}

#[test]
fn a_newer_server_side_is_read_and_not_written() {
    // FR-130. The dangerous reading is the other one: a newer server side may keep things
    // elsewhere, and an older application writing to it by its own idea of the layout is how
    // a working server is quietly broken.
    let newer = ServerState {
        server_version: Some(APP_EXPECTS + 1),
        ..server(Kind::Managed, Compat::TooNew)
    };
    assert!(allowed(&newer, Intent::Read).is_ok());
    match allowed(&newer, Intent::Change) {
        Err(Refusal::TooNew {
            server,
            app_expects,
        }) => {
            assert_eq!(server, APP_EXPECTS + 1);
            assert_eq!(app_expects, APP_EXPECTS);
        }
        other => panic!("a newer server was written to: {other:?}"),
    }
    // Nor is it offered an upgrade: this application does not know what it would be
    // upgrading to.
    assert!(allowed(&newer, Intent::Setup).is_err());
}

#[test]
fn an_older_server_side_is_offered_the_upgrade_and_not_written_to() {
    let older = server(Kind::Managed, Compat::NeedsUpgrade);
    assert!(allowed(&older, Intent::Read).is_ok());
    assert!(matches!(
        allowed(&older, Intent::Change),
        Err(Refusal::NeedsUpgrade { .. })
    ));
    assert!(
        allowed(&older, Intent::Setup).is_ok(),
        "the way forward has to be open, or the person is stuck"
    );
}

#[test]
fn a_bare_machine_is_for_deploying_and_for_looking_at() {
    let bare = server(Kind::Clean, Compat::NotDeployed);
    assert!(allowed(&bare, Intent::Setup).is_ok());
    assert!(matches!(
        allowed(&bare, Intent::Change),
        Err(Refusal::NotDeployed)
    ));
    // **And it may be looked at.** Found on the real stand: refusing this turned the one
    // command whose job is to say "this machine is bare" into a command that refuses bare
    // machines. There is nothing to read there, which is not the same as being forbidden
    // to look.
    assert!(
        allowed(&bare, Intent::Read).is_ok(),
        "a bare machine could not be looked at, so nothing could ever discover it is bare"
    );
}

#[test]
fn our_own_server_is_open_to_everything_but_being_deployed_again() {
    let ours = server(Kind::Managed, Compat::Ok);
    assert!(allowed(&ours, Intent::Read).is_ok());
    assert!(allowed(&ours, Intent::Change).is_ok());
    assert!(
        allowed(&ours, Intent::Setup).is_err(),
        "a working server was offered a deployment over the top of itself"
    );
}

#[test]
fn a_server_that_will_not_answer_is_touched_by_nothing() {
    let gone = unreachable();
    assert!(allowed(&gone, Intent::Change).is_err());
    assert!(
        allowed(&gone, Intent::Setup).is_err(),
        "deploying to a server that is not answering is how a half-deployed machine is made"
    );
}

#[test]
fn every_combination_holds() {
    // The whole product, because a hole in this table is a permission granted by accident and
    // nothing anywhere complains about it.
    let kinds = [Kind::Clean, Kind::Managed, Kind::Foreign, Kind::Unreachable];
    let compats = [
        Compat::Ok,
        Compat::NeedsUpgrade,
        Compat::TooNew,
        Compat::NotDeployed,
        Compat::Unknown,
    ];
    for kind in kinds {
        for compat in compats {
            let state = server(kind, compat);
            let at = format!("{kind:?}/{compat:?}");

            // Exactly one state may be changed.
            assert_eq!(
                allowed(&state, Intent::Change).is_ok(),
                kind == Kind::Managed && compat == Compat::Ok,
                "{at}: the wrong answer about changing"
            );
            // Setting up is for a bare machine and for one that is behind.
            assert_eq!(
                allowed(&state, Intent::Setup).is_ok(),
                kind == Kind::Clean || (kind == Kind::Managed && compat == Compat::NeedsUpgrade),
                "{at}: the wrong answer about setting up"
            );
            // Looking is allowed everywhere, and that is the rule rather than an
            // oversight: a person has to be able to find out what a machine is before
            // anything can tell them what may be done with it.
            assert!(
                allowed(&state, Intent::Read).is_ok(),
                "{at}: looking was refused"
            );
        }
    }
}
