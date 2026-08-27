//! T264 — the three states, against three real machines.
//!
//! The judging is checked without a server in `tests/unit/server_state.rs`; what is checked
//! here is the half that cannot be: that a real machine in each of the three states produces
//! the facts the judging expects. A parser tested only against strings we wrote ourselves
//! checks our idea of what a server says.
//!
//! The dangerous one is the middle test. Everything else fails loudly; a foreign server read
//! as bare fails by succeeding — the application offers to deploy, the offer looks like the
//! right one, and somebody's machine is rebuilt.

use vrcast_studio_lib::domain::server_state::{judge, Kind};
use vrcast_studio_lib::server::detect::{command, read};

use super::deploy_fixture::{DeployTarget, Flavour};
use super::fixture::TestServer;

#[test]
fn a_bare_machine_is_recognised_as_bare() {
    let target = DeployTarget::start(Flavour::Clean).expect("the bare container would not come up");
    let said = target
        .exec_inside(&command("/var/lib/vrcast/videos"))
        .expect("the machine would not answer");

    let facts = read(&said);
    assert!(facts.state_file.is_none(), "a state file on a bare machine");
    assert!(!facts.caddyfile_present);
    assert_eq!(
        facts.web_server_running, None,
        "something is serving: {said}"
    );

    let state = judge(&facts);
    assert_eq!(state.kind, Kind::Clean, "read as {:?}: {said}", state.kind);
}

#[test]
fn somebody_elses_machine_is_recognised_as_theirs() {
    // **The one that fails by succeeding.** There is no Caddyfile here and no state file — the
    // only sign is that something is listening, and a detector that looked only at the path it
    // knows would report this machine as bare and offer to deploy on it.
    let target =
        DeployTarget::start(Flavour::Foreign).expect("the foreign container would not come up");
    let said = target
        .exec_inside(&command("/var/lib/vrcast/videos"))
        .expect("the machine would not answer");

    let facts = read(&said);
    assert!(facts.state_file.is_none());
    assert!(
        !facts.caddyfile_present,
        "this machine is supposed to be foreign by its running server alone"
    );
    assert!(
        facts.web_server_running.is_some(),
        "the running web server was not noticed: {said}"
    );

    let state = judge(&facts);
    assert_eq!(
        state.kind,
        Kind::Foreign,
        "read as {:?}: {said}",
        state.kind
    );
    assert!(
        state.foreign_reason.is_some(),
        "nothing to tell the person what was found"
    );
}

#[test]
fn our_own_machine_is_recognised_as_ours() {
    // The ordinary throwaway container, which since T252a carries a state file. Without one it
    // would read as foreign — it has a Caddyfile — and every changing operation of milestone C
    // would be refused on it.
    let server = TestServer::start().expect("the container would not come up");
    let said = server
        .exec_inside(&command("/var/lib/vrcast/videos"))
        .expect("the machine would not answer");

    let facts = read(&said);
    assert!(
        matches!(facts.state_file, Some(Ok(_))),
        "the state file was not read: {said}"
    );

    let state = judge(&facts);
    assert_eq!(
        state.kind,
        Kind::Managed,
        "read as {:?}: {said}",
        state.kind
    );
    assert_eq!(
        state.server_version,
        Some(1),
        "the version was not read out of the file"
    );
}
