//! T289 — the one place that decides whether this application may touch this server.
//!
//! **A prohibition spread across the commands is a prohibition somebody will forget in one of
//! them**, and a forgotten one looks exactly like a working program — right up to the day
//! somebody points the application at a server that is not theirs. So there is a single door:
//! every operation that changes anything on a server comes through here, says what it is for,
//! and is refused if this machine is not ours to change (FR-007, FR-130, FR-132).
//!
//! The recognising happens on connecting rather than being read from a cache. It costs one
//! command on a connection that was being opened anyway, and the alternative — believing what
//! the server looked like the last time — is how a machine that changed hands quietly gets
//! written to.

use crate::domain::server_profile::ServerProfile;
use crate::domain::server_state::{self, Compat, ForeignReason, Kind, ServerState};
use crate::ssh::{Connection, SshError};
use crate::store::secrets::SecretStore;

/// What a session is being opened for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    /// Only look. Allowed on anything that answers, including somebody else's server —
    /// looking is how a person finds out it *is* somebody else's.
    Read,
    /// Change the serving: send files, build sets, cap a viewer.
    Change,
    /// Set the server up, or bring it up to date.
    Setup,
}

/// Why the door stayed shut.
///
/// Codes with what is needed to explain them; the wordings live in the interface's
/// dictionaries. `NotDeployed` and `Foreign` are told apart deliberately: one is an offer to
/// deploy and the other is a refusal to.
#[derive(Debug)]
pub enum Refusal {
    /// Somebody else's machine. Names what was recognised (FR-132).
    Foreign {
        reason: Option<ForeignReason>,
    },
    /// The server side is newer than this application understands. Reading is fine; writing
    /// would be an older application putting files where a newer layout does not keep them
    /// (FR-130).
    TooNew {
        server: u32,
        app_expects: u32,
    },
    /// Too old to be written to until it is brought up to date.
    NeedsUpgrade {
        server: u32,
        app_min: u32,
    },
    /// Nothing is deployed here yet, so there is nothing to change.
    NotDeployed,
    Ssh(SshError),
}

impl From<SshError> for Refusal {
    fn from(e: SshError) -> Self {
        Self::Ssh(e)
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Foreign { reason } => write!(f, "somebody else's server: {reason:?}"),
            Self::TooNew {
                server,
                app_expects,
            } => write!(
                f,
                "the server side is version {server} and this application knows {app_expects}"
            ),
            Self::NeedsUpgrade { server, app_min } => write!(
                f,
                "the server side is version {server} and at least {app_min} is needed"
            ),
            Self::NotDeployed => f.write_str("nothing is deployed on this server"),
            Self::Ssh(e) => write!(f, "{e}"),
        }
    }
}

/// A connection, and what was recognised about the server behind it.
pub struct Opened {
    pub conn: Connection,
    pub state: ServerState,
}

/// Open a session for a purpose, or refuse.
pub async fn open(
    secrets: &dyn SecretStore,
    profile: &ServerProfile,
    intent: Intent,
) -> Result<Opened, Refusal> {
    let conn = super::connect(secrets, profile).await?;
    let state = super::detect::detect(&conn, &profile.video_dir).await?;

    if let Err(refusal) = allowed(&state, intent) {
        // The connection is closed here rather than left to be dropped: a refused session
        // holds a channel on a server it has no business being on, and on somebody else's
        // machine that is a login sitting open in their logs.
        conn.close().await;
        return Err(refusal);
    }
    Ok(Opened { conn, state })
}

/// The decision itself, without a server.
///
/// Split out so every combination can be checked without one — this is the rule the whole
/// protection rests on, and it is exactly the kind that fails by succeeding.
pub fn allowed(state: &ServerState, intent: Intent) -> Result<(), Refusal> {
    let may = server_state::allowed(state);
    let ok = match intent {
        Intent::Read => may.read,
        Intent::Change => may.change_serving,
        Intent::Setup => !matches!(may.setup, server_state::Setup::Nothing),
    };
    if ok {
        return Ok(());
    }

    // Refused. Which refusal it is decides what the person is offered next, so it is worked
    // out from what was recognised rather than being one flat "not allowed".
    Err(match (state.kind, state.compat) {
        (Kind::Foreign, _) => Refusal::Foreign {
            reason: state.foreign_reason.clone(),
        },
        (Kind::Managed, Compat::TooNew) => Refusal::TooNew {
            server: state.server_version.unwrap_or_default(),
            app_expects: state.app_expects,
        },
        (Kind::Managed, Compat::NeedsUpgrade) => Refusal::NeedsUpgrade {
            server: state.server_version.unwrap_or_default(),
            app_min: state.app_min_supported,
        },
        _ => Refusal::NotDeployed,
    })
}
