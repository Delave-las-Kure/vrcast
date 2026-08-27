//! T253, T254, T255, T260 — what this server is, and what may be done to it.
//!
//! Four questions, and the order they are asked in decides everything (FR-120, R-11,
//! `contracts/server-contract.md`):
//!
//! 1. Is there a state file of ours, and does it read?
//! 2. If not — is there anything here that serves? Then somebody else was first.
//! 3. Neither — the machine is bare.
//! 4. Given the answer, what is this application allowed to do?
//!
//! **The order is not a matter of taste.** Asking "is it bare" before "is somebody else
//! serving" makes the application deploy over a stranger's machine, and it would do it
//! quietly and successfully. That is why the check for a foreign server comes first and why
//! it is written down in the contract rather than left to whoever writes the detector.
//!
//! Nothing here reaches for a server: it is handed the facts and answers from them, so all
//! of it is checkable without one.

use serde::{Deserialize, Serialize};

/// The version of the server side this application deploys.
///
/// A whole number that grows by one on any change to the composition. Not a semantic
/// version: the server side is not a public library, and it has exactly one consumer (R-11).
pub const APP_EXPECTS: u32 = 1;

/// The oldest version this application can still work with.
///
/// Below it, changing the serving is refused until the server side is upgraded — not out of
/// strictness, but because the application would be writing files in a layout the server
/// does not have.
pub const APP_MIN_SUPPORTED: u32 = 1;

/// The file the application leaves on a server it deployed: `/etc/vrcast/state.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateFile {
    pub vrcast_server_version: u32,
    #[serde(default)]
    pub deployed_at: String,
    #[serde(default)]
    pub deployed_by_app: String,
    #[serde(default)]
    pub steps_applied: Vec<String>,
    #[serde(default)]
    pub video_dir: String,
    #[serde(default)]
    pub domain: String,
}

/// Why a state file could not be believed.
///
/// A code with what is needed to explain it, not a sentence: the wordings live in the
/// interface's dictionaries, one per language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StateFileProblem {
    /// Not JSON at all, or JSON of the wrong shape. Carries what the parser said, for the
    /// report — a person fixing this by hand needs to know where it broke.
    Unreadable { detail: String },
    /// Read, but the version is zero. Versions start at one; a zero means the file was
    /// written by something that did not know what it was writing.
    NoVersion,
}

/// Read the state file.
///
/// **A half-written file is not an absent one.** Treating it as absent would mean deploying
/// over our own server — and every field of a real state file (the serving directory, the
/// domain) would be replaced by whatever the new deployment was told, silently.
pub fn parse_state_file(text: &str) -> Result<StateFile, StateFileProblem> {
    let file: StateFile = serde_json::from_str(text).map_err(|e| StateFileProblem::Unreadable {
        detail: e.to_string(),
    })?;
    if file.vrcast_server_version == 0 {
        return Err(StateFileProblem::NoVersion);
    }
    Ok(file)
}

/// What was found on the machine. Gathered by the access layer, judged here.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Facts {
    /// The state file's contents, if there was one to read at all.
    ///
    /// `None` means the file is not there. `Some(Err(..))` means it is there and cannot be
    /// believed — a different answer, and the difference decides what happens next.
    pub state_file: Option<Result<StateFile, StateFileProblem>>,
    /// Is there a main web-server configuration at `/etc/caddy/Caddyfile`?
    pub caddyfile_present: bool,
    /// Is something actually serving? The name is carried so the refusal can say what was
    /// found: "an nginx is running here" is something a person can act on, "a foreign
    /// configuration" is not.
    pub web_server_running: Option<String>,
    /// Is the serving directory there?
    pub video_dir_present: bool,
    /// Is there something here that **only this application** puts on a server?
    ///
    /// The rules file it owns outright, or its serving directory. Nobody else creates
    /// either, and both appear well before the state file — which is written last, on
    /// purpose (T281).
    pub our_own_marks: bool,
}

/// What the machine is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    /// Nothing of ours and nothing of anybody else's.
    Clean,
    /// Deployed by this application.
    Managed,
    /// Somebody else was here first — or our own marker cannot be read, which comes to the
    /// same thing: we do not know what this machine is.
    Foreign,
    /// **A deployment of ours that did not reach its end** (found on the real stand,
    /// 2026-08-27).
    ///
    /// Our directories and our rules file are here and the state file is not, because
    /// the state file is written last. Without this case such a machine reads as
    /// *foreign* — a web server is running and there is no marker — and the application
    /// then refuses to touch its own half-finished work. A deployment interrupted at
    /// any step would be unrecoverable by the thing that started it, which is the exact
    /// opposite of what FR-124 and SC-015 promise.
    ///
    /// It is not `Clean`: the machine is half-configured, and telling a person it is
    /// bare would be untrue. It is not `Managed`: nothing here may be served from until
    /// the deployment is finished.
    Unfinished,
    /// Could not be reached at all. Kept as a state rather than an error so that the last
    /// known picture can be shown with a stale mark instead of an empty screen.
    Unreachable,
}

/// How the server's version sits against this application's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Compat {
    /// Within the range this application works with.
    Ok,
    /// Older than the oldest supported. Changing the serving waits for the upgrade.
    NeedsUpgrade,
    /// Newer than this application understands. Read only (FR-130).
    TooNew,
    /// Nothing is deployed, so there is nothing to compare.
    NotDeployed,
    /// Not ours, or not reachable.
    Unknown,
}

/// Why a machine was judged foreign. A code, with what is needed to explain it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ForeignReason {
    /// Something is serving, and it is not ours. Names it.
    WebServerRunning { name: String },
    /// A main web-server configuration with no state file beside it. This is the case that
    /// looks most like ours and is not — a server set up by hand, or by an older tool.
    ConfigWithoutState,
    /// Our own marker is there and unreadable. Refused deliberately: the one thing that must
    /// never happen is changing a machine we do not understand, and this is exactly that.
    StateFileUnreadable { problem: StateFileProblem },
}

/// The whole judgement. Not stored — worked out on connecting, and the last one cached in
/// the profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerState {
    pub kind: Kind,
    /// Only when `Managed`.
    pub server_version: Option<u32>,
    pub app_expects: u32,
    pub app_min_supported: u32,
    pub compat: Compat,
    /// Whether a newer server side exists than the one deployed. Separate from `compat` on
    /// purpose: "you could upgrade" (FR-129) and "you must upgrade before changing anything"
    /// are different sentences and lead to different screens. With one version in existence
    /// this is always false, and it is written now rather than when version 2 appears —
    /// which is when nobody would remember the difference.
    pub upgrade_available: bool,
    pub foreign_reason: Option<ForeignReason>,
}

/// Judge the machine from what was found.
///
/// The order of the branches is the contract's, and reordering them is the one edit here
/// that breaks something without failing anything: a server of somebody else's would come
/// back as clean, and the next thing the application does is offer to deploy on it.
pub fn judge(facts: &Facts) -> ServerState {
    let plain = |kind: Kind, compat: Compat, reason: Option<ForeignReason>| ServerState {
        kind,
        server_version: None,
        app_expects: APP_EXPECTS,
        app_min_supported: APP_MIN_SUPPORTED,
        compat,
        upgrade_available: false,
        foreign_reason: reason,
    };

    match &facts.state_file {
        // 1. Ours, and readable.
        Some(Ok(file)) => {
            let version = file.vrcast_server_version;
            let compat = compat_of(version);
            ServerState {
                kind: Kind::Managed,
                server_version: Some(version),
                app_expects: APP_EXPECTS,
                app_min_supported: APP_MIN_SUPPORTED,
                compat,
                upgrade_available: version < APP_EXPECTS,
                foreign_reason: None,
            }
        }
        // 1a. Ours, and not readable. Foreign, and said so plainly: the marker is the one
        // thing that tells us this machine is ours, and an unreadable marker means we do not
        // know. Refusing costs a person one manual step; guessing costs them their serving.
        Some(Err(problem)) => plain(
            Kind::Foreign,
            Compat::Unknown,
            Some(ForeignReason::StateFileUnreadable {
                problem: problem.clone(),
            }),
        ),
        None => {
            // 2. Nothing of ours. Is anybody else serving?
            //
            // The running service is looked at BEFORE the configuration file. Both mean
            // foreign, but a detector that only checks the path it knows — /etc/caddy —
            // walks straight past a machine running anything else, and that is the mistake
            // this order exists to make impossible.
            // 2a. **Ours, and not finished.** Asked before either foreign branch: a
            // deployment stopped part-way leaves a running web server and no state
            // file, which is exactly what a stranger's machine looks like. Told apart
            // by the things only this application puts there.
            if facts.our_own_marks {
                return plain(Kind::Unfinished, Compat::NotDeployed, None);
            }
            if let Some(name) = &facts.web_server_running {
                return plain(
                    Kind::Foreign,
                    Compat::Unknown,
                    Some(ForeignReason::WebServerRunning { name: name.clone() }),
                );
            }
            if facts.caddyfile_present {
                return plain(
                    Kind::Foreign,
                    Compat::Unknown,
                    Some(ForeignReason::ConfigWithoutState),
                );
            }
            // 3. Bare.
            //
            // A serving directory on its own does not make it ours: an empty
            // /var/lib/vrcast left over from a removal is not a deployment, and refusing to
            // deploy over it would leave a person stuck with no way forward.
            plain(Kind::Clean, Compat::NotDeployed, None)
        }
    }
}

/// The version comparison on its own (FR-127…FR-130).
pub fn compat_of(version: u32) -> Compat {
    if version > APP_EXPECTS {
        Compat::TooNew
    } else if version < APP_MIN_SUPPORTED {
        Compat::NeedsUpgrade
    } else {
        Compat::Ok
    }
}

/// The state of a server that would not answer.
pub fn unreachable() -> ServerState {
    ServerState {
        kind: Kind::Unreachable,
        server_version: None,
        app_expects: APP_EXPECTS,
        app_min_supported: APP_MIN_SUPPORTED,
        compat: Compat::Unknown,
        upgrade_available: false,
        foreign_reason: None,
    }
}

/// What setting-up, if any, this machine is open to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Setup {
    /// Deploy from bare.
    Deploy,
    /// Upgrade the server side that is already there.
    Upgrade,
    /// Neither.
    Nothing,
}

/// What the application may do to this machine.
///
/// **The single place where that is decided** ([data-model.md](../../../../specs/001-vrcast-studio/data-model.md),
/// section 2). Every changing command asks here before it works. Spread across the commands,
/// the rule gets forgotten in one of them — and a forgotten prohibition looks exactly like a
/// working program until the day somebody points the application at a server that is not
/// theirs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Allowed {
    /// Read what is on the server: the catalogue, the viewers, the state.
    pub read: bool,
    /// Change the serving: send files, build sets, set limits.
    pub change_serving: bool,
    pub setup: Setup,
}

/// Apply the matrix.
pub fn allowed(state: &ServerState) -> Allowed {
    match (state.kind, state.compat) {
        // Bare: there is nothing to read and nothing to change, and everything to set up.
        (Kind::Clean, _) => Allowed {
            read: false,
            change_serving: false,
            setup: Setup::Deploy,
        },
        // Half-configured: it may be looked at and it may be finished. Serving from it
        // is refused — the deployment has not said it is ready, and it is the one that
        // knows.
        (Kind::Unfinished, _) => Allowed {
            read: true,
            change_serving: false,
            setup: Setup::Deploy,
        },
        (Kind::Managed, Compat::Ok) => Allowed {
            read: true,
            change_serving: true,
            setup: Setup::Nothing,
        },
        // Too old to write to, but perfectly readable — and the way forward is named rather
        // than left to the person to work out.
        (Kind::Managed, Compat::NeedsUpgrade) => Allowed {
            read: true,
            change_serving: false,
            setup: Setup::Upgrade,
        },
        // Newer than we understand: read only (FR-130). Upgrading is not offered either —
        // this application does not know what it would be upgrading to.
        (Kind::Managed, Compat::TooNew) => Allowed {
            read: true,
            change_serving: false,
            setup: Setup::Nothing,
        },
        // Somebody else's: readable, untouchable (FR-132).
        (Kind::Foreign, _) => Allowed {
            read: true,
            change_serving: false,
            setup: Setup::Nothing,
        },
        // Unreachable: the cached picture, marked stale. Nothing may be done to a server
        // that is not answering — including deploying, which is how a half-deployed machine
        // is made.
        (Kind::Unreachable, _) => Allowed {
            read: true,
            change_serving: false,
            setup: Setup::Nothing,
        },
        // A managed server whose version says nothing. Not reachable in practice — `judge`
        // always sets a comparison for a managed one — but written out rather than left to
        // a catch-all: the safe answer is "nothing", and a future variant that fell through
        // here would otherwise inherit permission by accident.
        (Kind::Managed, Compat::NotDeployed | Compat::Unknown) => Allowed {
            read: true,
            change_serving: false,
            setup: Setup::Nothing,
        },
    }
}
