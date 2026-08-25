//! What the core is able to say — as codes, not as sentences.
//!
//! The core names the situation; the interface owns the wording, in every language it
//! speaks (FR-105, FR-106). This module holds the vocabulary both sides agree on.
//!
//! It lives in `domain` rather than next to [`crate::commands::error::AppError`]
//! because the checks that produce these are here: a profile field, a short name, a
//! preparation plan. Putting the vocabulary a layer up would mean the domain composing
//! prose again, which is exactly what has to stop.
//!
//! Nothing here touches secrets: redaction happens where the detail is attached to an
//! error, together with the rest of that discipline (see `commands::error`).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Declares detail codes as ONE list: the enum, `ALL` and `as_str` all come from it.
///
/// The same reasoning as [`crate::commands::error::ErrorCode`]: a hand-maintained
/// list next to the enum is a hole in every check that walks it, and the compiler
/// cannot see the hole.
macro_rules! detail_codes {
    ($($(#[$meta:meta])* $name:ident => $code:literal),+ $(,)?) => {
        /// A specific thing to say about a failure, finer than the error code itself.
        ///
        /// Separate from `ErrorCode` on purpose: the code is what the interface
        /// **branches on** (offer a retry, open the fingerprint dialog), and that set
        /// has to stay small enough to reason about. The detail is what the interface
        /// **says**, and there are naturally many more of those.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(into = "String", try_from = "String")]
        pub enum DetailCode {
            $($(#[$meta])* $name,)+
        }

        impl DetailCode {
            /// Every detail code. Born of the same list as the enum.
            pub const ALL: &'static [DetailCode] = &[$(Self::$name),+];

            /// The string key the interface looks up in its catalogue.
            pub fn as_str(&self) -> &'static str {
                match self { $(Self::$name => $code,)+ }
            }

            /// Read a code back. Needed because a plan carrying a reason is stored
            /// and read again on the next run.
            pub fn parse(s: &str) -> Option<Self> {
                match s { $($code => Some(Self::$name),)+ _ => None }
            }
        }
    };
}

detail_codes! {
    // --- server profile fields ---
    ProfileIdEmpty => "PROFILE_ID_EMPTY",
    ProfileNameEmpty => "PROFILE_NAME_EMPTY",
    /// `max` — the limit, in characters.
    ProfileNameTooLong => "PROFILE_NAME_TOO_LONG",
    /// `name` — the one already in use.
    ProfileNameTaken => "PROFILE_NAME_TAKEN",
    ProfileHostEmpty => "PROFILE_HOST_EMPTY",
    ProfileHostNotBare => "PROFILE_HOST_NOT_BARE",
    ProfilePortRange => "PROFILE_PORT_RANGE",
    ProfileUserEmpty => "PROFILE_USER_EMPTY",
    ProfileUserHasSpaces => "PROFILE_USER_HAS_SPACES",
    ProfileSecretRefEmpty => "PROFILE_SECRET_REF_EMPTY",
    ProfileKeyPathRequired => "PROFILE_KEY_PATH_REQUIRED",
    ProfileKeyPathUnused => "PROFILE_KEY_PATH_UNUSED",
    ProfileNotFound => "PROFILE_NOT_FOUND",
    FingerprintEmpty => "FINGERPRINT_EMPTY",

    // --- domain field ---
    DomainEmpty => "DOMAIN_EMPTY",
    DomainHasSpaces => "DOMAIN_HAS_SPACES",
    DomainHasPath => "DOMAIN_HAS_PATH",
    DomainHasUserOrPort => "DOMAIN_HAS_USER_OR_PORT",
    DomainBadDots => "DOMAIN_BAD_DOTS",
    DomainNoDot => "DOMAIN_NO_DOT",
    DomainBadChars => "DOMAIN_BAD_CHARS",

    // --- video directory field ---
    VideoDirEmpty => "VIDEO_DIR_EMPTY",
    VideoDirNotAbsolute => "VIDEO_DIR_NOT_ABSOLUTE",
    VideoDirHasDotDot => "VIDEO_DIR_HAS_DOTDOT",
    VideoDirHasNewline => "VIDEO_DIR_HAS_NEWLINE",
    VideoDirAtRoot => "VIDEO_DIR_AT_ROOT",

    // --- CDN address field ---
    CdnBaseNoScheme => "CDN_BASE_NO_SCHEME",
    CdnBaseHasSpaces => "CDN_BASE_HAS_SPACES",
    CdnBaseIncomplete => "CDN_BASE_INCOMPLETE",

    // --- short name (slug) ---
    SlugEmpty => "SLUG_EMPTY",
    /// `len`, `max` — actual and allowed length, in bytes.
    SlugTooLong => "SLUG_TOO_LONG",
    /// `char` — the first character that is not allowed.
    SlugBadChar => "SLUG_BAD_CHAR",
    SlugReserved => "SLUG_RESERVED",
    SlugUnmakeable => "SLUG_UNMAKEABLE",

    // --- library ---
    MediaTitleEmpty => "MEDIA_TITLE_EMPTY",
    MediaNothingToChange => "MEDIA_NOTHING_TO_CHANGE",
    MediaNotFound => "MEDIA_NOT_FOUND",
    MediaIsServiceEntry => "MEDIA_IS_SERVICE_ENTRY",
    /// `old`, `new` — the names on the server.
    RenameFailed => "RENAME_FAILED",
    DeleteFilesFailed => "DELETE_FILES_FAILED",
    ManifestMalformed => "MANIFEST_MALFORMED",
    /// `what`, `files`, `bytes` — what is about to be removed.
    ConfirmDelete => "CONFIRM_DELETE",
    /// `connections` — how many are open right now. What a deletion would do to them.
    ViewersActiveDelete => "VIEWERS_ACTIVE_DELETE",

    // --- preparing files ---
    FfmpegSelfBroken => "FFMPEG_SELF_BROKEN",
    FfmpegNoX264 => "FFMPEG_NO_X264",
    ProbeNoVideo => "PROBE_NO_VIDEO",
    ProbeUnreadable => "PROBE_UNREADABLE",
    ConvertNoOutPath => "CONVERT_NO_OUT_PATH",
    ConvertOutOverwritesSource => "CONVERT_OUT_OVERWRITES_SOURCE",
    ConvertValidateNoFfmpeg => "CONVERT_VALIDATE_NO_FFMPEG",
    ConvertNoEncoder => "CONVERT_NO_ENCODER",
    PlanNoAudioTracks => "PLAN_NO_AUDIO_TRACKS",
    /// `number` — as a human counts, from one; `available` — how many there are.
    PlanNoSuchTrack => "PLAN_NO_SUCH_TRACK",
    PlanHeightZero => "PLAN_HEIGHT_ZERO",
    /// `asked`, `source` — heights, in lines.
    PlanHeightAboveSource => "PLAN_HEIGHT_ABOVE_SOURCE",
    PlanBitrateZero => "PLAN_BITRATE_ZERO",
    /// `asked_kbps`, `source_kbps`.
    PlanBitrateAboveSource => "PLAN_BITRATE_ABOVE_SOURCE",

    // --- how a long task can end badly ---
    /// `out_path` — where the file was left. `problems` — the decoder's own words.
    ConvertValidationFailed => "CONVERT_VALIDATION_FAILED",
    /// `sent`, `total` — bytes.
    UploadShort => "UPLOAD_SHORT",
    UploadChecksumMismatch => "UPLOAD_CHECKSUM_MISMATCH",
    /// The source on disk is no longer the one the transfer started with.
    UploadSourceChanged => "UPLOAD_SOURCE_CHANGED",
    /// The connection kept breaking; `attempts` — how many times it was tried.
    UploadTooManyBreaks => "UPLOAD_TOO_MANY_BREAKS",
    /// `path` — the source that could not be read.
    UploadSourceUnreadable => "UPLOAD_SOURCE_UNREADABLE",

    // --- stages of a long task, as shown beside its progress ---
    StageConverting => "STAGE_CONVERTING",
    StageValidating => "STAGE_VALIDATING",
    StageChecksum => "STAGE_CHECKSUM",
    StageDone => "STAGE_DONE",

    // --- what closing the application would do to a task (FR-086) ---
    /// `percent` — how far it got.
    OnCloseResumesFrom => "ON_CLOSE_RESUMES_FROM",
    /// `percent` — how much work would be thrown away.
    OnCloseRestartsLosing => "ON_CLOSE_RESTARTS_LOSING",
    OnCloseNotStartedYet => "ON_CLOSE_NOT_STARTED_YET",
    OnCloseMustRunAgain => "ON_CLOSE_MUST_RUN_AGAIN",

    // --- steps of the connection check (FR-003) ---
    /// `banner` — what the server introduced itself as.
    StepNetBanner => "STEP_NET_BANNER",
    /// `seconds` — how long we waited.
    StepNetTimeout => "STEP_NET_TIMEOUT",
    StepNetClosed => "STEP_NET_SILENT_CLOSED",
    StepNetSilent => "STEP_NET_SILENT",
    /// `port`, `got` — where we knocked and what answered.
    StepNetNotSsh => "STEP_NET_NOT_SSH",
    StepLoginFingerprintUnconfirmed => "STEP_LOGIN_FINGERPRINT_UNCONFIRMED",
    /// `user` — who we came in as.
    StepLoginOk => "STEP_LOGIN_OK",
    /// `dir` — the directory that is readable and writable.
    StepVideoDirOk => "STEP_VIDEO_DIR_OK",
    /// `dir`, `user`.
    StepVideoDirMissingOrDenied => "STEP_VIDEO_DIR_MISSING_OR_DENIED",
    /// `domain`, `code` — answered, but there was no file to check serving with.
    StepDomainOkNoFiles => "STEP_DOMAIN_OK_NO_FILES",
    /// `url`, `code`.
    StepDomainFileNotServed => "STEP_DOMAIN_FILE_NOT_SERVED",
    /// `url` — what was fetched to prove it.
    StepDomainOk => "STEP_DOMAIN_OK",
    /// `url`, `code`.
    StepDomainEmptyBody => "STEP_DOMAIN_EMPTY_BODY",
    /// `domain`, `seconds`.
    StepDomainTimeout => "STEP_DOMAIN_TIMEOUT",
    /// `domain`.
    StepDomainNoConnection => "STEP_DOMAIN_NO_CONNECTION",

    /// A complaint from a library, in its own words. `text` — what it said.
    ///
    /// Kept verbatim rather than translated: it can be searched for, and a rephrased
    /// version of it cannot. Every use of this is a place where we do not know enough
    /// to say anything better.
    SystemError => "SYSTEM_ERROR",

    // --- why a stream cannot simply be carried across (FR-022) ---
    /// `codec` — what the source is in.
    ReasonVideoNotH264 => "REASON_VIDEO_NOT_H264",
    /// `pix_fmt` — the source pixel format.
    ReasonVideoPixFmt => "REASON_VIDEO_PIX_FMT",
    ReasonTonemap => "REASON_TONEMAP",
    ReasonResize => "REASON_RESIZE",
    ReasonTargetBitrate => "REASON_TARGET_BITRATE",
    /// `codec` — what the track is in.
    ReasonAudioNotAac => "REASON_AUDIO_NOT_AAC",
    /// `channels` — how many the track has.
    ReasonAudioChannels => "REASON_AUDIO_CHANNELS",
    ReasonAudioTooFat => "REASON_AUDIO_TOO_FAT",

    // --- what to say about the choice of encoder (FR-026) ---
    NoticeNoHardwareFound => "NOTICE_NO_HARDWARE_FOUND",
    NoticeSoftwareAsAsked => "NOTICE_SOFTWARE_AS_ASKED",
    /// `encoder` — the ffmpeg name of the one that failed, e.g. `h264_nvenc`.
    NoticeHardwareFailed => "NOTICE_HARDWARE_FAILED",

    // --- transfer ---
    UploadFileUnreadable => "UPLOAD_FILE_UNREADABLE",
    UploadNotAFile => "UPLOAD_NOT_A_FILE",
    UploadNameEmpty => "UPLOAD_NAME_EMPTY",
    /// `name` — the file already on its way to this server.
    UploadAlreadyRunning => "UPLOAD_ALREADY_RUNNING",
    UploadNameReserved => "UPLOAD_NAME_RESERVED",
    /// `short_by`, `needed`, `free` — bytes.
    NotEnoughSpace => "NOT_ENOUGH_SPACE",
    /// `name` — the file that will be replaced.
    NameWillBeReplaced => "NAME_WILL_BE_REPLACED",
    CdnKeepsOldCopy => "CDN_KEEPS_OLD_COPY",
    /// `connections` — how many are open right now. What an upload would do to them.
    ViewersActiveUpload => "VIEWERS_ACTIVE_UPLOAD",
}

impl TryFrom<String> for DetailCode {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::parse(&s).ok_or_else(|| format!("unknown detail code: {s}"))
    }
}

impl From<DetailCode> for String {
    fn from(c: DetailCode) -> Self {
        c.as_str().to_owned()
    }
}

impl std::fmt::Display for DetailCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One thing to say, with the values to put into it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Detail {
    pub key: DetailCode,
    /// Substitutions, by name. Ordered, so the same situation always serialises the
    /// same way — otherwise comparing two errors would pass or fail at random.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, serde_json::Value>,
}

impl Detail {
    pub fn new(key: DetailCode) -> Self {
        Self {
            key,
            params: BTreeMap::new(),
        }
    }

    /// Add a substitution.
    ///
    /// Numbers go in raw, not pre-formatted: one language writes 22.0 GB, another writes
    /// the same number with a comma and its own unit. Which of the two to write is the
    /// interface's choice, and it cannot make that choice once the number is a string.
    pub fn with(mut self, name: &str, value: impl Into<serde_json::Value>) -> Self {
        self.params.insert(name.to_owned(), value.into());
        self
    }
}
