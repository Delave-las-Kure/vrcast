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
    /// Only look.
    ///
    /// **Allowed on anything that answers**, bare machines included. Found on the real
    /// stand (2026-08-27): the first version of this asked `server_state::allowed`,
    /// whose `read` field means "there is serving to read" and is false for a bare
    /// machine — so the one command whose whole job is to find out that a machine is
    /// bare was refused for being pointed at a bare machine.
    ///
    /// The two questions are different and only one of them belongs here. What may be
    /// **changed** is a protection; what there is to **show** is a screen's business.
    Read,
    /// Change the serving: send files, build sets, cap a viewer.
    Change,
    /// Set the server up, or bring it up to date.
    Setup,
    /// Put the last upgrade's copies back (FR-133) — `server_rollback` and nothing else (T601).
    ///
    /// ⚠ **This used to go through as `Read`, and a rollback is not a look.** It copies
    /// `state.json`, the Caddyfile and the rest back over the live ones and reloads the
    /// services. Opened as `Read`, it went through on anything that answered: an older
    /// application on a server a newer one had already upgraded past what it knows (FR-130)
    /// put back a layout it does not understand, and on somebody else's machine (FR-132) it
    /// copied whatever lay under `/etc/vrcast/backup/latest` over their configuration.
    ///
    /// **Neither `Change` nor `Setup` fits, and that is why this is a variant of its own.**
    /// `Change` refuses a server that `NeedsUpgrade` — and an upgrade of an older server that
    /// broke half-way leaves exactly that behind, because `state.json` is written last: the
    /// one moment a rollback is most wanted. `Setup` refuses a current server of ours
    /// (`AlreadyDeployed`) — which is what an upgrade that *finished* and turned out badly
    /// leaves. So: ours, at a version this application either knows or is behind.
    Restore,
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
    /// It is already deployed, and at a version this application is happy with — so
    /// there is nothing to set up (found on the real stand, 2026-08-27).
    ///
    /// Kept apart from `NotDeployed` because they are opposites and were arriving with
    /// the same words: a person asking to deploy a working server was told that nothing
    /// was deployed on it.
    AlreadyDeployed,
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
            Self::AlreadyDeployed => f.write_str("this server is already deployed and up to date"),
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
    let conn = super::connect_raw(secrets, profile, None).await?;
    admit(conn, &profile.video_dir, intent, None).await
}

/// Open a session to stop what a deployment or upgrade left running (T609, T615).
///
/// Through the gate like everything else — stopping a process on the server is an action on
/// it, not a read (the lesson of T601) — with the intent the run itself used, `Setup`, and
/// `Change` **only** when `Setup` is refused as [`Refusal::AlreadyDeployed`]: a run stopped
/// during its last step (the state file) leaves a server that is ours and current, and
/// without the second door the stop would be retried for ever against a server that will
/// never say yes. Nothing else is let through.
///
/// `made_key` (T615): the private key the run made and put on the server. A run that broke
/// off after it turned password logins off leaves a profile still saying "password" (it is
/// switched only once the run is over, and the run is over only once its stop is confirmed)
/// — signing in with it would fail on every attempt, for ever. When given, the made key is
/// tried first and the profile's own credentials only if it will not sign in.
pub async fn open_to_stop(
    secrets: &dyn SecretStore,
    profile: &ServerProfile,
    made_key: Option<&str>,
) -> Result<Opened, Refusal> {
    let conn = match made_key {
        Some(key) => match super::connect_raw(secrets, profile, Some(key)).await {
            Ok(conn) => conn,
            Err(by_key) => {
                tracing::warn!(error = %by_key, "the made key would not sign in; trying the profile's own way in");
                super::connect_raw(secrets, profile, None).await?
            }
        },
        None => super::connect_raw(secrets, profile, None).await?,
    };
    admit(
        conn,
        &profile.video_dir,
        Intent::Setup,
        Some(Intent::Change),
    )
    .await
}

/// Let an open connection through for `intent`, or refuse — closing it.
///
/// `fallback` is a second intent tried **only** when `intent` is refused as
/// [`Refusal::AlreadyDeployed`] — asked on one detection rather than on two connections.
async fn admit(
    conn: Connection,
    video_dir: &str,
    intent: Intent,
    fallback: Option<Intent>,
) -> Result<Opened, Refusal> {
    let state = super::detect::detect(&conn, video_dir).await?;
    let verdict = match (allowed(&state, intent), fallback) {
        (Err(Refusal::AlreadyDeployed), Some(second)) => allowed(&state, second),
        (first, _) => first,
    };
    if let Err(refusal) = verdict {
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
        // Looking never harms, and refusing to look is how a person is left unable to
        // find out what they are looking at.
        Intent::Read => true,
        Intent::Change => may.change_serving,
        Intent::Setup => !matches!(may.setup, server_state::Setup::Nothing),
        // Ours, and not newer than this application understands. Everything else is refused
        // by the same reasoning as a change: TooNew by FR-130, Foreign (an unreadable state
        // file included) by FR-132, and a bare or half-deployed machine has no upgrade of
        // ours to roll back.
        Intent::Restore => {
            state.kind == Kind::Managed && matches!(state.compat, Compat::Ok | Compat::NeedsUpgrade)
        }
    };
    if ok {
        return Ok(());
    }

    // Refused. Which refusal it is decides what the person is offered next, so it is worked
    // out from what was recognised rather than being one flat "not allowed".
    Err(match (state.kind, state.compat) {
        // Ours and half-finished: nothing to serve from yet, and the way forward is to
        // finish it — which `Setup` already allows. Said as "not deployed" because that is
        // what it is; calling it foreign is the mistake this whole case exists to undo.
        (Kind::Unfinished, _) => Refusal::NotDeployed,
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
        // A working server asked to be set up again. Not a fault — an answer.
        (Kind::Managed, Compat::Ok) => Refusal::AlreadyDeployed,
        _ => Refusal::NotDeployed,
    })
}
