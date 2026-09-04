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

// ---------- the gate is the door, and there is no second one (T488) ----------
//
// ⚠ **The rule above is checked; the way to it was not.** Everything in this file judges
// `allowed(state, intent)` — the decision. But a decision is only worth what reaching it is
// worth, and reaching it is not a rule of the type system: it is four call sites of
// `Connection::connect` that somebody chose to write one way rather than another. A fifth,
// added in the ordinary course of work, would open a session on a machine the application
// never asked itself about, and not one of this project's checks would notice.
//
// **What the guard is written against was chosen after looking.** The obvious rule —
// "`connect_raw` has one caller" — is itself the failure it is meant to prevent: it guards a
// door with three others standing open beside it. `connect_raw` is not where a connection is
// made; `Connection::connect` is, and it is reached from three places besides the gate. So
// the list below is of every place in the core that reaches a server at all, with why each
// may do so without asking first.

/// Every place in the core that opens a session, how many times, and why it may do so before
/// — or without — the gate's decision.
///
/// **A count, not just a file.** A new `Connection::connect` beside an allowed one is exactly
/// the case this exists for: the file is already on the list, so naming files alone would
/// wave it through.
const REACHES_A_SERVER: &[(&str, usize, &str)] = &[
    (
        "server/mod.rs",
        1,
        "`connect_raw` itself — the one door, which `gate::open` opens after it has decided.",
    ),
    (
        "commands/servers.rs",
        1,
        "The step-by-step connection check (T041). It exists precisely to find out what is at \
         the other end, so it cannot be made to wait for an answer that comes from connecting. \
         It reads and reports; it changes nothing.",
    ),
    (
        "commands/deploy.rs",
        2,
        "The two proofs after the hardening (T274): that the key still works, and that a \
         password is now refused. Each needs a session of its own — the one being held would \
         go on working whatever the settings became, which is what makes it the wrong witness. \
         Both run on a machine the gate has already judged ours.",
    ),
];

/// Every `.rs` file under a directory, path first.
fn core_files(dir: &std::path::Path, into: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            core_files(&path, into);
        } else if path.extension().is_some_and(|e| e == "rs") {
            into.push(path);
        }
    }
}

/// The file's code with its line comments taken out, so a mention in prose is not a call.
fn without_comments(body: &str) -> String {
    body.lines()
        .map(|l| match l.find("//") {
            Some(at) => &l[..at],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Where the core opens sessions, counted per file and named the way the list names them.
fn where_a_server_is_reached(needle: &str) -> std::collections::BTreeMap<String, usize> {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    core_files(&src, &mut files);

    let mut found = std::collections::BTreeMap::new();
    for path in files {
        let Ok(body) = std::fs::read_to_string(&path) else {
            continue;
        };
        let times = without_comments(&body).matches(needle).count();
        if times > 0 {
            let rel = path
                .strip_prefix(&src)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            *found.entry(rel).or_insert(0) += times;
        }
    }
    found
}

#[test]
fn nothing_reaches_a_server_except_the_places_that_say_why() {
    let found = where_a_server_is_reached("Connection::connect(");
    let mut wrong = Vec::new();

    for (where_, times) in &found {
        match REACHES_A_SERVER.iter().find(|(f, _, _)| f == where_) {
            None => wrong.push(format!(
                "{where_} opens a session {times} time(s) and is on no list. Either it must go \
                 through `gate::open`, or it must be added here with why it may not."
            )),
            Some((_, allowed, _)) if allowed != times => wrong.push(format!(
                "{where_} opens a session {times} time(s); the list allows {allowed}. A new one \
                 beside an allowed one is still a new way past the gate."
            )),
            Some(_) => {}
        }
    }
    for (where_, allowed, _) in REACHES_A_SERVER {
        if !found.contains_key(*where_) {
            wrong.push(format!(
                "{where_} is allowed {allowed} session(s) and opens none. The list has outlived \
                 the code — take the entry out."
            ));
        }
    }

    assert!(
        wrong.is_empty(),
        "principle I rests on the gate being the only way to a server:\n  {}",
        wrong.join("\n  ")
    );
}

#[test]
fn the_undecided_door_is_opened_only_by_the_gate() {
    // `connect_raw` is the connection made *without* asking whether the machine may be
    // touched — its own doc says so. `pub(crate)` keeps it inside the core; nothing keeps a
    // second place inside the core from calling it.
    let found = where_a_server_is_reached("connect_raw(");
    let callers: Vec<&String> = found
        .keys()
        .filter(|f| f.as_str() != "server/mod.rs")
        .collect();
    assert_eq!(
        callers,
        vec![&String::from("server/gate.rs")],
        "`connect_raw` reaches a server without asking whether it may be touched, so only the \
         gate may call it; these do: {callers:?}"
    );
}

#[test]
fn every_place_that_may_skip_the_gate_carries_its_reason() {
    for (where_, _, why) in REACHES_A_SERVER {
        assert!(
            why.len() > 40,
            "{where_} may reach a server ungated with no reason written down"
        );
    }
}
