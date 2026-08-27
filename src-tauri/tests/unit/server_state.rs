//! T261 — recognising a server, comparing versions, and what may be done to it.
//!
//! The table these check against is R-11's and the contract's, and the point of checking
//! every row is that this is code whose mistakes do not fail — they succeed at the wrong
//! thing. A detector that answers "clean" for somebody else's machine does not throw; it
//! offers to deploy, and the offer looks exactly like the right one.

use vrcast_studio_lib::domain::server_state::{
    allowed, compat_of, judge, parse_state_file, unreachable, Compat, Facts, ForeignReason, Kind,
    Setup, StateFile, StateFileProblem, APP_EXPECTS, APP_MIN_SUPPORTED,
};

fn state_file(version: u32) -> StateFile {
    StateFile {
        vrcast_server_version: version,
        deployed_at: String::from("2026-08-27T00:00:00Z"),
        deployed_by_app: String::from("0.1.0"),
        steps_applied: vec![String::from("user-dirs")],
        video_dir: String::from("/var/lib/vrcast/videos"),
        domain: String::from("stream.example.com"),
    }
}

// ---------- the five rows of the table ----------

#[test]
fn nothing_of_ours_and_nothing_of_anybody_elses_is_a_bare_machine() {
    let state = judge(&Facts::default());
    assert_eq!(state.kind, Kind::Clean);
    assert_eq!(state.compat, Compat::NotDeployed);
    assert!(state.server_version.is_none());
    assert!(state.foreign_reason.is_none());
}

#[test]
fn our_state_file_within_range_is_our_server() {
    let state = judge(&Facts {
        state_file: Some(Ok(state_file(APP_EXPECTS))),
        caddyfile_present: true,
        web_server_running: Some(String::from("caddy")),
        video_dir_present: true,
        our_own_marks: true,
    });
    assert_eq!(state.kind, Kind::Managed);
    assert_eq!(state.compat, Compat::Ok);
    assert_eq!(state.server_version, Some(APP_EXPECTS));
    assert!(!state.upgrade_available);
}

#[test]
fn a_version_newer_than_we_understand_is_read_only() {
    // FR-130. The dangerous reading is the opposite one: a newer server side may have moved
    // files or changed the layout, and writing to it with an older application's idea of
    // where things are is how a working server is quietly broken.
    let state = judge(&Facts {
        state_file: Some(Ok(state_file(APP_EXPECTS + 1))),
        ..Facts::default()
    });
    assert_eq!(state.kind, Kind::Managed);
    assert_eq!(state.compat, Compat::TooNew);
    assert!(!allowed(&state).change_serving);
    assert!(allowed(&state).read);
    assert_eq!(allowed(&state).setup, Setup::Nothing);
}

#[test]
fn a_version_below_the_oldest_supported_waits_for_an_upgrade() {
    // Written against the constants rather than against the number 0, so that it goes on
    // meaning what it says when the oldest supported version rises. Today the two constants
    // are both 1 and this branch is dormant — which is precisely why it is checked now,
    // while somebody still remembers what it is for.
    assert_eq!(compat_of(APP_MIN_SUPPORTED - 1), Compat::NeedsUpgrade);

    let state = judge(&Facts {
        state_file: Some(Ok(state_file(APP_MIN_SUPPORTED - 1))),
        ..Facts::default()
    });
    assert_eq!(state.compat, Compat::NeedsUpgrade);
    let may = allowed(&state);
    assert!(may.read, "an old server is still readable");
    assert!(!may.change_serving, "an old server must not be written to");
    assert_eq!(may.setup, Setup::Upgrade, "and the way forward is named");
}

#[test]
fn no_state_file_but_something_is_serving_means_somebody_else_was_first() {
    // **The row that costs the most to get wrong** (FR-132). And it is checked with nginx
    // rather than with a Caddyfile on purpose: a detector that looks only at
    // /etc/caddy/Caddyfile — the path it knows — walks straight past a machine running
    // anything else and reports it clean.
    let state = judge(&Facts {
        web_server_running: Some(String::from("nginx")),
        ..Facts::default()
    });
    assert_eq!(state.kind, Kind::Foreign);
    assert_eq!(
        state.foreign_reason,
        Some(ForeignReason::WebServerRunning {
            name: String::from("nginx")
        }),
        "the refusal has to name what was found: a person can act on \"an nginx is running\""
    );
}

#[test]
fn a_configuration_with_no_state_file_is_also_somebody_elses() {
    // The other branch: a machine set up by hand, or by an older tool, that looks like ours
    // and is not.
    let state = judge(&Facts {
        caddyfile_present: true,
        ..Facts::default()
    });
    assert_eq!(state.kind, Kind::Foreign);
    assert_eq!(
        state.foreign_reason,
        Some(ForeignReason::ConfigWithoutState)
    );
}

// ---------- the state file itself ----------

#[test]
fn a_half_written_state_file_is_not_an_absent_one() {
    // Treating it as absent means deploying over our own server: every field of a real state
    // file — the serving directory, the domain — would be replaced by whatever the new
    // deployment was told, and nothing would look wrong.
    let broken = parse_state_file("{\"vrcast_server_version\": 1, \"doma");
    assert!(matches!(broken, Err(StateFileProblem::Unreadable { .. })));

    let state = judge(&Facts {
        state_file: Some(broken),
        caddyfile_present: true,
        ..Facts::default()
    });
    assert_eq!(
        state.kind,
        Kind::Foreign,
        "a marker we cannot read means we do not know what this machine is"
    );
    assert!(matches!(
        state.foreign_reason,
        Some(ForeignReason::StateFileUnreadable { .. })
    ));
    assert!(!allowed(&state).change_serving);
}

#[test]
fn a_state_file_without_a_version_is_refused() {
    // Versions start at one. A zero means something wrote the file without knowing what it
    // was writing, and believing it would make the version comparison meaningless.
    assert_eq!(
        parse_state_file("{\"vrcast_server_version\": 0}"),
        Err(StateFileProblem::NoVersion)
    );
}

#[test]
fn a_state_file_with_only_a_version_still_reads() {
    // The other side of the same coin: the fields around the version are filled in as the
    // server side grows, and an older file missing the newer ones must not become
    // unreadable — that would turn every upgrade into a machine the application refuses to
    // touch.
    let file = parse_state_file("{\"vrcast_server_version\": 1}").expect("a bare version failed");
    assert_eq!(file.vrcast_server_version, 1);
    assert!(file.steps_applied.is_empty());
}

// ---------- the matrix, on every combination ----------

#[test]
fn what_is_allowed_holds_on_every_combination() {
    // Every combination rather than the interesting ones: this matrix is dangerous exactly
    // where nobody looked, and a forgotten cell hands out permission by accident.
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
            let state = vrcast_studio_lib::domain::server_state::ServerState {
                kind,
                server_version: None,
                app_expects: APP_EXPECTS,
                app_min_supported: APP_MIN_SUPPORTED,
                compat,
                upgrade_available: false,
                foreign_reason: None,
            };
            let may = allowed(&state);
            let at = format!("{kind:?}/{compat:?}");

            // Changing implies reading. A state that lets the serving be changed but not
            // read would be one where the application writes blind.
            if may.change_serving {
                assert!(may.read, "{at}: changing without reading");
            }
            // Exactly one state may be written to.
            assert_eq!(
                may.change_serving,
                kind == Kind::Managed && compat == Compat::Ok,
                "{at}: the wrong answer about changing the serving"
            );
            // Deploying is offered only on a bare machine. Offering it anywhere else is the
            // failure this whole module exists to prevent.
            assert_eq!(
                may.setup == Setup::Deploy,
                kind == Kind::Clean,
                "{at}: the wrong answer about deploying"
            );
            assert_eq!(
                may.setup == Setup::Upgrade,
                kind == Kind::Managed && compat == Compat::NeedsUpgrade,
                "{at}: the wrong answer about upgrading"
            );
            if kind == Kind::Foreign {
                assert!(!may.change_serving, "{at}: a foreign server was written to");
                assert_eq!(
                    may.setup,
                    Setup::Nothing,
                    "{at}: a foreign server was set up"
                );
            }
        }
    }
}

#[test]
fn a_server_that_will_not_answer_shows_what_was_known_and_is_touched_by_nothing() {
    let state = unreachable();
    let may = allowed(&state);
    assert!(
        may.read,
        "the last known picture is shown, with a stale mark"
    );
    assert!(!may.change_serving);
    assert_eq!(
        may.setup,
        Setup::Nothing,
        "deploying to a server that is not answering is how a half-deployed machine is made"
    );
}

#[test]
fn a_deployment_that_did_not_finish_is_ours_and_not_a_stranger_s() {
    // **Found on the real stand** (2026-08-27), and it is the difference between SC-015 being
    // true and being impossible. A deployment stopped part-way leaves a running web server,
    // our directories and our rules file — and no state file, because the state file is
    // written last on purpose. Read by the plain rule, that is a stranger's machine, and the
    // application then refuses to finish its own work. Every interrupted deployment would be
    // unrecoverable by the thing that started it.
    let half_done = Facts {
        state_file: None,
        caddyfile_present: true,
        web_server_running: Some(String::from("caddy")),
        video_dir_present: true,
        our_own_marks: true,
    };
    let state = judge(&half_done);
    assert_eq!(state.kind, Kind::Unfinished, "read as {:?}", state.kind);

    let may = allowed(&state);
    assert_eq!(
        may.setup,
        Setup::Deploy,
        "there is no way to finish what was started"
    );
    assert!(
        !may.change_serving,
        "a half-configured server was opened for serving from"
    );
    assert!(may.read, "and it must still be possible to look at it");
}

#[test]
fn a_strangers_machine_is_still_a_strangers() {
    // The other side of the same rule, and the one that must not be lost by adding the first.
    // A machine with somebody's web server and none of our own marks is theirs, whatever else
    // is true of it.
    let theirs = Facts {
        state_file: None,
        caddyfile_present: false,
        web_server_running: Some(String::from("nginx")),
        video_dir_present: false,
        our_own_marks: false,
    };
    assert_eq!(judge(&theirs).kind, Kind::Foreign);
}
