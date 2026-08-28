//! T012 — an error is an object, not a sentence.
//!
//! Contract of the command layer (`contracts/ipc-commands.md`, rule 2): an error is
//! `{ code, details, cause? }`. The core names **what happened**; the interface owns
//! the wording, in every language it speaks (FR-105, FR-106).
//!
//! Why not a ready-made sentence, as it used to be. Prose composed here can exist in
//! only one language at a time, and an error already written into a task record would
//! stay in the language of the moment forever — switching the interface to English
//! would leave yesterday's failures in Russian. Codes re-render.
//!
//! Why not a bare code either. One code often has to say several things at once
//! ("the name is taken" *and* "the CDN will keep serving the old copy"), and it has to
//! name numbers: how much is missing, how many connections are open. Hence `details`:
//! an ordered list of what to say, each with its own substitutions.
//!
//! Every code and every detail must have a wording in **both** languages. That is
//! enforced by the type of the catalogue (`src/shared/i18n/`), not by attentiveness:
//! the record is keyed by the union of codes, so a missing one fails the build.

use crate::domain::wording::Detail;
use serde::{Deserialize, Serialize};

// The vocabulary of what can be said lives in the domain layer, beside the checks
// that produce it. Re-exported here so callers get an error and its details from
// one place.
pub use crate::domain::wording::DetailCode;

/// Declares error codes as ONE list: the enum, `ALL` and `as_str` all come from it.
///
/// `ALL` used to be maintained by hand alongside the enum, and that was the hole in
/// the whole checking system: a code added to the enum but forgotten in `ALL` dropped
/// out of every check at once. The compiler stayed quiet — it demands match arms, not
/// a complete hand-written list. Now there is nowhere to forget it.
macro_rules! error_codes {
    ($($(#[$meta:meta])* $name:ident => $code:literal),+ $(,)?) => {
        /// Error code. The list is fixed by `contracts/ipc-commands.md`.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(into = "String", try_from = "String")]
        pub enum ErrorCode {
            $($(#[$meta])* $name,)+
        }

        impl ErrorCode {
            /// Every code. Born of the same list as the enum — they cannot diverge.
            pub const ALL: &'static [ErrorCode] = &[$(Self::$name),+];

            /// The string code that goes to the interface.
            pub fn as_str(&self) -> &'static str {
                match self { $(Self::$name => $code,)+ }
            }

            /// Read a code back — a failed task keeps its error across a restart.
            pub fn parse(s: &str) -> Option<Self> {
                match s { $($code => Some(Self::$name),)+ _ => None }
            }
        }
    };
}

error_codes! {
    // --- reaching the server ---
    SshAuthFailed => "SSH_AUTH_FAILED",
    SshUnreachable => "SSH_UNREACHABLE",
    HostKeyChanged => "HOST_KEY_CHANGED",
    HostKeyUnconfirmed => "HOST_KEY_UNCONFIRMED",
    HostKeyIsCertificate => "HOST_KEY_IS_CERTIFICATE",
    KeyNeedsPassphrase => "KEY_NEEDS_PASSPHRASE",
    KeyUnreadable => "KEY_UNREADABLE",
    VideoDirDenied => "VIDEO_DIR_DENIED",

    // --- domain ---
    DomainNotServing => "DOMAIN_NOT_SERVING",
    DomainNotPointed => "DOMAIN_NOT_POINTED",
    DomainPointsElsewhere => "DOMAIN_POINTS_ELSEWHERE",
    Ipv6Mismatch => "IPV6_MISMATCH",

    // --- server state and deployment ---
    ServerForeign => "SERVER_FOREIGN",
    ServerTooNew => "SERVER_TOO_NEW",
    /// The server side is older than this application can work with. Not a fault —
    /// an offer, and the screen turns it into one (FR-129).
    ServerNeedsUpgrade => "SERVER_NEEDS_UPGRADE",
    DeployStepFailed => "DEPLOY_STEP_FAILED",
    SwapFailed => "SWAP_FAILED",

    // --- library ---
    SlugTaken => "SLUG_TAKEN",
    ManifestConflict => "MANIFEST_CONFLICT",
    FileMissingOnServer => "FILE_MISSING_ON_SERVER",
    FileInUse => "FILE_IN_USE",

    // --- preparing files ---
    FfmpegBroken => "FFMPEG_BROKEN",
    NoAudioTracks => "NO_AUDIO_TRACKS",
    DecodeValidationFailed => "DECODE_VALIDATION_FAILED",
    NoHwEncoder => "NO_HW_ENCODER",
    LocalDiskFull => "LOCAL_DISK_FULL",

    // --- transfer ---
    RemoteDiskFull => "REMOTE_DISK_FULL",
    ChecksumMismatch => "CHECKSUM_MISMATCH",
    ViewersActive => "VIEWERS_ACTIVE",
    NameExists => "NAME_EXISTS",

    // --- quality ladders ---
    RungAboveSource => "RUNG_ABOVE_SOURCE",
    BufsizeTooLarge => "BUFSIZE_TOO_LARGE",
    LevelExceeded => "LEVEL_EXCEEDED",
    LadderIncomplete => "LADDER_INCOMPLETE",
    NoLadderForMedia => "NO_LADDER_FOR_MEDIA",
    /// This build of FFmpeg cannot measure quality, so no ladder can be built.
    VmafUnavailable => "VMAF_UNAVAILABLE",
    /// The ladder for this material has not been measured, and a ladder is not built
    /// from a formula (R-21).
    LadderNotMeasured => "LADDER_NOT_MEASURED",
    /// A batch stopped rather than build a ladder the checker objects to (T439).
    ///
    /// Its own code and not `InvalidInput`: nothing about the input was wrong. The work
    /// was done, the answer came out, and the answer is one this application will not
    /// send to a server unasked.
    LadderObjection => "LADDER_OBJECTION",
    MeasurementNotFound => "MEASUREMENT_NOT_FOUND",
    /// The measurement asked for was taken on material of another kind entirely.
    MeasurementDifferentMaterial => "MEASUREMENT_DIFFERENT_MATERIAL",

    // --- web server configuration ---
    CaddyValidateFailed => "CADDY_VALIDATE_FAILED",
    CaddyReloadFailed => "CADDY_RELOAD_FAILED",

    // --- tasks ---
    TaskCancelled => "TASK_CANCELLED",
    TaskNotFound => "TASK_NOT_FOUND",
    TaskBadTransition => "TASK_BAD_TRANSITION",
    TaskNotPausable => "TASK_NOT_PAUSABLE",

    // --- input and confirmation ---
    InvalidInput => "INVALID_INPUT",
    ConfirmationRequired => "CONFIRMATION_REQUIRED",

    // --- updating the application itself ---
    UpdateCheckFailed => "UPDATE_CHECK_FAILED",
    UpdateInstallFailed => "UPDATE_INSTALL_FAILED",

    // --- everything else ---
    StorageFailed => "STORAGE_FAILED",
    Internal => "INTERNAL",
}

impl TryFrom<String> for ErrorCode {
    type Error = String;

    fn try_from(s: String) -> std::result::Result<Self, String> {
        Self::parse(&s).ok_or_else(|| format!("unknown error code: {s}"))
    }
}

impl From<ErrorCode> for String {
    fn from(c: ErrorCode) -> Self {
        c.as_str().to_owned()
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An error on its way to the interface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppError {
    pub code: ErrorCode,
    /// What exactly to say, in order. Empty means the code's own wording says it all.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<Detail>,
    /// The particulars: which file, which step, which address. Always redacted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
}

impl AppError {
    pub fn new(code: ErrorCode) -> Self {
        Self {
            code,
            details: Vec::new(),
            cause: None,
        }
    }

    /// Say something specific, beyond what the code alone says.
    pub fn detail(mut self, key: DetailCode) -> Self {
        self.details.push(Detail::new(key));
        self
    }

    /// Say something specific that carries values.
    pub fn with_detail(mut self, detail: Detail) -> Self {
        self.details.push(redacted(detail));
        self
    }

    /// Say all of these, in order.
    pub fn with_details(mut self, details: impl IntoIterator<Item = Detail>) -> Self {
        self.details.extend(details.into_iter().map(redacted));
        self
    }

    /// Add the particulars. Redacted: they often come from a foreign library
    /// (constitution, principle IV).
    pub fn with_cause(mut self, cause: impl std::fmt::Display) -> Self {
        self.cause = Some(crate::store::redact::safe_display(&cause));
        self
    }

    /// Is this among the things being said? For branching and for tests.
    pub fn says(&self, key: DetailCode) -> bool {
        self.details.iter().any(|d| d.key == key)
    }
}

/// Sweep secrets out of a detail on its way into an error.
///
/// Done here rather than in [`Detail::with`] so that the domain layer stays free of
/// storage concerns, and so that all redaction sits at the same boundary as the one
/// applied to `cause` (constitution, principle IV). Substituted text is often a path
/// or a name that arrived from a library knowing nothing of our rules; numbers cannot
/// carry a secret and are left alone.
fn redacted(mut detail: Detail) -> Detail {
    for value in detail.params.values_mut() {
        if let serde_json::Value::String(s) = value {
            *value = serde_json::Value::String(crate::store::redact::redact(s).into_owned());
        }
    }
    detail
}

/// Developer-facing, for logs. Deliberately not prose: prose belongs to the interface
/// now, and inventing a second set of it here is how the two drift apart.
impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}]", self.code)?;
        for d in &self.details {
            write!(f, " {}", d.key)?;
            if !d.params.is_empty() {
                let pairs: Vec<String> = d.params.iter().map(|(k, v)| format!("{k}={v}")).collect();
                write!(f, "({})", pairs.join(", "))?;
            }
        }
        if let Some(cause) = &self.cause {
            write!(f, ": {cause}")?;
        }
        Ok(())
    }
}

impl std::error::Error for AppError {}

pub type Result<T> = std::result::Result<T, AppError>;

// --- turning lower-layer failures into contract codes ---

/// The single door's refusals, as contract codes.
///
/// Each of the four sends a person somewhere different: buy nothing and look
/// elsewhere, upgrade the application, upgrade the server side, or deploy. One flat
/// "not allowed" would send them nowhere.
impl From<crate::server::gate::Refusal> for AppError {
    fn from(e: crate::server::gate::Refusal) -> Self {
        use crate::server::gate::Refusal as R;
        match e {
            R::Ssh(inner) => inner.into(),
            R::Foreign { reason } => {
                AppError::new(ErrorCode::ServerForeign).with_cause(format!("{reason:?}"))
            }
            R::TooNew {
                server,
                app_expects,
            } => AppError::new(ErrorCode::ServerTooNew)
                .with_cause(format!("server {server}, application {app_expects}")),
            R::NeedsUpgrade { server, app_min } => AppError::new(ErrorCode::ServerNeedsUpgrade)
                .with_cause(format!("server {server}, at least {app_min}")),
            R::NotDeployed => AppError::new(ErrorCode::ServerForeign)
                .with_cause("nothing is deployed on this server"),
            // Not a failure of anything. The screen turns it into "already set up",
            // and until it does, at least the words are the right way round.
            R::AlreadyDeployed => AppError::new(ErrorCode::InvalidInput)
                .with_cause("this server is already deployed and up to date"),
        }
    }
}

impl From<crate::ssh::SshError> for AppError {
    fn from(e: crate::ssh::SshError) -> Self {
        use crate::ssh::SftpFailure as F;
        use crate::ssh::SshError as S;
        let code = match &e {
            S::Unreachable { .. } => ErrorCode::SshUnreachable,
            S::HostKeyChanged { .. } => ErrorCode::HostKeyChanged,
            S::HostKeyUnconfirmed { .. } => ErrorCode::HostKeyUnconfirmed,
            S::HostKeyIsCertificate => ErrorCode::HostKeyIsCertificate,
            S::AuthFailed { .. } => ErrorCode::SshAuthFailed,
            S::KeyNeedsPassphrase { .. } => ErrorCode::KeyNeedsPassphrase,
            S::KeyUnreadable { .. } => ErrorCode::KeyUnreadable,
            S::Exec(_) | S::Protocol(_) => ErrorCode::Internal,
            // A file failure sends a person in DIFFERENT directions depending on the
            // cause. Every one of them used to be reported as a permission problem
            // with the hint "check who owns the directory" — on a full disk that sent
            // people to fix what was not broken (debt T071).
            S::Sftp { kind, .. } => match kind {
                F::NoSpace => ErrorCode::RemoteDiskFull,
                F::Denied => ErrorCode::VideoDirDenied,
                F::Missing => ErrorCode::FileMissingOnServer,
                // A dropped connection is not a broken server, it is a reason to try
                // again. Showing it as "no access" would send someone to fix what works.
                F::Interrupted => ErrorCode::SshUnreachable,
                // The unfamiliar is not guessed at: a wrong guess is worse than an
                // honest "unknown" because it leads to fixing the wrong thing. The text
                // itself is kept.
                F::Other => ErrorCode::Internal,
            },
        };
        // The lower layer's particulars are kept: they name the specifics — which
        // address, which authentication methods the server offered, which key file.
        AppError::new(code).with_cause(e)
    }
}

impl From<crate::server::manifest_io::ManifestIoError> for AppError {
    fn from(e: crate::server::manifest_io::ManifestIoError) -> Self {
        use crate::server::manifest_io::ManifestIoError as M;
        match e {
            // The only case where a refusal is normal work rather than a fault:
            // another copy of the application is working with this server.
            M::Conflict { .. } => AppError::new(ErrorCode::ManifestConflict).with_cause(e),
            M::Malformed(_) => AppError::new(ErrorCode::Internal)
                .detail(DetailCode::ManifestMalformed)
                .with_cause(e),
            M::Ssh(inner) => AppError::from(inner),
        }
    }
}

impl From<crate::store::db::DbError> for AppError {
    fn from(e: crate::store::db::DbError) -> Self {
        AppError::new(ErrorCode::StorageFailed).with_cause(e)
    }
}

impl From<crate::store::secrets::SecretError> for AppError {
    fn from(e: crate::store::secrets::SecretError) -> Self {
        AppError::new(ErrorCode::StorageFailed).with_cause(e)
    }
}
