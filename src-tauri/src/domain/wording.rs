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
    StageMeasuringQuality => "STAGE_MEASURING_QUALITY",
    StageBuildingLadder => "STAGE_BUILDING_LADDER",
    StageCuttingSegments => "STAGE_CUTTING_SEGMENTS",
    StageVerifyingLadder => "STAGE_VERIFYING_LADDER",
    /// A deployment is under way. Which step it is on goes out as its own event: the
    /// screen shows the whole list with their states, and a single stage code could not.
    StageDeploying => "STAGE_DEPLOYING",
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
    /// The source's own keyframes do not fall where the segments will be cut.
    ReasonKeyframesUnaligned => "REASON_KEYFRAMES_UNALIGNED",
    /// `codec` — what the track is in.
    ReasonAudioNotAac => "REASON_AUDIO_NOT_AAC",
    /// `channels` — how many the track has.
    ReasonAudioChannels => "REASON_AUDIO_CHANNELS",
    ReasonAudioTooFat => "REASON_AUDIO_TOO_FAT",

    // --- what to say about the choice of encoder (FR-026) ---
    NoticeNoHardwareFound => "NOTICE_NO_HARDWARE_FOUND",
    NoticeSoftwareAsAsked => "NOTICE_SOFTWARE_AS_ASKED",
    /// The probe ran on an encoder the quality setting was never calibrated against.
    NoticeProbeUncalibrated => "NOTICE_PROBE_UNCALIBRATED",
    /// The probe could not run at all, and the ladder rests on the old constant.
    NoticeProbeFailed => "NOTICE_PROBE_FAILED",
    /// `from` — the file the measurement was really taken on. These rungs were not
    /// measured on this material.
    NoticeMeasurementBorrowed => "NOTICE_MEASUREMENT_BORROWED",
    /// `measured`, `total` — how much of the grid answered. Some points would not encode.
    NoticeMeasurementPartial => "NOTICE_MEASUREMENT_PARTIAL",
    /// A rung that needed no change of quality is being re-encoded anyway, to put its
    /// keyframes where the other rungs have theirs.
    NoticeReencodedForKeyframes => "NOTICE_REENCODED_FOR_KEYFRAMES",
    /// `count` — variants that were already on the server and were not made again.
    NoticeVariantsReused => "NOTICE_VARIANTS_REUSED",

    // --- what to know before capping somebody's quality (FR-066) ---
    /// A limit is put on an address, and an address is not a person.
    WarnLimitFollowsTheAddress => "WARN_LIMIT_FOLLOWS_THE_ADDRESS",
    /// `count` — how many viewers are behind this address right now.
    WarnAddressShared => "WARN_ADDRESS_SHARED",
    /// `lightest_bps` — the lightest rung there is.
    WarnCapBelowLightest => "WARN_CAP_BELOW_LIGHTEST",
    /// `encoder` — the ffmpeg name of the one that failed, e.g. `h264_nvenc`.
    NoticeHardwareFailed => "NOTICE_HARDWARE_FAILED",

    // --- what to do about a domain that does not point here (FR-140) ---
    //
    // Not one code saying "the domain is wrong" but four saying what to do, because the
    // four are different afternoons: create a record, correct one, remove one, or remove
    // one that was never about this machine. A person who is told only that something is
    // wrong goes to their registrar and guesses.
    /// `record` — A or AAAA, `name` — the exact name, `value` — what to put in it.
    DomainAddRecord => "DOMAIN_ADD_RECORD",
    /// `record`, `name`, `to` — where it leads now, `value` — where it must lead.
    DomainFixRecord => "DOMAIN_FIX_RECORD",
    /// `record`, `name`, `to`. IPv6 is being turned off and the record still promises it.
    DomainRemoveRecord => "DOMAIN_REMOVE_RECORD",
    /// `name`, `to`. The machine has no IPv6 address at all, so whatever this leads to is
    /// not it — the likeliest shape of a record left from the domain's previous life.
    DomainServerHasNoIpv6 => "DOMAIN_SERVER_HAS_NO_IPV6",

    // --- transfer ---
    UploadFileUnreadable => "UPLOAD_FILE_UNREADABLE",
    UploadNotAFile => "UPLOAD_NOT_A_FILE",
    UploadNameEmpty => "UPLOAD_NAME_EMPTY",
    /// `name` — the file already on its way to this server.
    UploadAlreadyRunning => "UPLOAD_ALREADY_RUNNING",
    UploadNameReserved => "UPLOAD_NAME_RESERVED",
    /// `short_by`, `needed`, `free` — bytes.
    NotEnoughSpace => "NOT_ENOUGH_SPACE",
    /// `short_by`, `needed`, `free` — bytes; `rungs` — how many variants were counted.
    ///
    /// **Not `NotEnoughSpace`**, and the difference is the word "about". For a transfer the
    /// size is the file's, known to the byte. For a set it is a reckoning made before
    /// anything is encoded — and somebody told "25 GB are needed" who then frees exactly
    /// 25 GB has been misled by a number that was never that precise. The count of rungs is
    /// there because it is what a person can actually act on: drop one and it fits.
    LadderNotEnoughSpace => "LADDER_NOT_ENOUGH_SPACE",
    /// The room could not be worked out — the source's length is unknown, or the server
    /// would not say what is free.
    ///
    /// Said rather than swallowed: a check that cannot run must not look like one that ran
    /// and was content.
    LadderSpaceUnknown => "LADDER_SPACE_UNKNOWN",
    LadderNoRoomHere => "LADDER_NO_ROOM_HERE",

    // What the checker objects to in a ladder (T444). Codes rather than sentences for the
    // usual reason — and here for a second one: until now these lived only as phrases on the
    // ladder screen, so a task that stopped on an objection had nothing to say it with but a
    // fresh set of words about the same five things. Two sets of phrases about one fact drift,
    // and the day they disagreed nobody would know which was the rule.
    ObjectionRungAboveSource => "OBJECTION_RUNG_ABOVE_SOURCE",
    ObjectionBufsizeTooLarge => "OBJECTION_BUFSIZE_TOO_LARGE",
    ObjectionLevelExceeded => "OBJECTION_LEVEL_EXCEEDED",
    ObjectionOutOfOrder => "OBJECTION_OUT_OF_ORDER",
    ObjectionBadStep => "OBJECTION_BAD_STEP",
    /// The batch stopped here rather than building something it objects to (T439).
    ChainStoppedByObjection => "CHAIN_STOPPED_BY_OBJECTION",

    // Why one film's measurement cannot be lent to another (T431). Which field it was, so
    // that a person is not left comparing two files by eye against a list of eight things
    // this application looked at.
    NoticeVariantsStranded => "NOTICE_VARIANTS_STRANDED",
    NoticeMaterialApart => "NOTICE_MATERIAL_APART",
    NoticeMeasurementThin => "NOTICE_MEASUREMENT_THIN",
    NoticeCheckPointHeld => "NOTICE_CHECK_POINT_HELD",
    CheckPointApart => "CHECK_POINT_APART",
    CheckPointNotComparable => "CHECK_POINT_NOT_COMPARABLE",
    NoticeCheckPointRunning => "NOTICE_CHECK_POINT_RUNNING",
    StageCheckingLoan => "STAGE_CHECKING_LOAN",

    LendFrameDiffers => "LEND_FRAME_DIFFERS",
    LendFpsDiffers => "LEND_FPS_DIFFERS",
    LendNativeHeightDiffers => "LEND_NATIVE_HEIGHT_DIFFERS",
    LendCodecDiffers => "LEND_CODEC_DIFFERS",
    LendPixelFormatDiffers => "LEND_PIXEL_FORMAT_DIFFERS",
    LendColourTransferDiffers => "LEND_COLOUR_TRANSFER_DIFFERS",
    LendTooShort => "LEND_TOO_SHORT",
    LendMaterialNotKnown => "LEND_MATERIAL_NOT_KNOWN",
    /// `name` — the file that will be replaced.
    NameWillBeReplaced => "NAME_WILL_BE_REPLACED",
    CdnKeepsOldCopy => "CDN_KEEPS_OLD_COPY",
    /// `connections` — how many are open right now. What an upload would do to them.
    ViewersActiveUpload => "VIEWERS_ACTIVE_UPLOAD",

    // --- the state of the server (FR-070) ---
    //
    // Every one of these carries the numbers it rests on, and that is not decoration: a
    // reading shown as a bare word is a reading nobody can check, and these are read by
    // somebody deciding whether to touch their server at all.
    /// Could not be found out. Never shown as "fine" — see `domain::health`.
    HealthNotEstablished => "HEALTH_NOT_ESTABLISHED",
    /// Cannot be found out **in a container** (T246): kernel settings and a real disk.
    HealthNotInContainer => "HEALTH_NOT_IN_CONTAINER",
    HealthServingRunning => "HEALTH_SERVING_RUNNING",
    /// `service` — which one, `state` — what the machine called it. Named, not implied.
    HealthServingStopped => "HEALTH_SERVING_STOPPED",
    /// `status` — 206, which is what a range request deserves.
    HealthDeliveryOk => "HEALTH_DELIVERY_OK",
    /// `status`. It served the whole file instead of the range: playing works, seeking does not.
    HealthDeliveryNoRanges => "HEALTH_DELIVERY_NO_RANGES",
    /// `status` — 4xx or 5xx to our own request.
    HealthDeliveryRefused => "HEALTH_DELIVERY_REFUSED",
    HealthDeliverySilent => "HEALTH_DELIVERY_SILENT",
    /// No video on the server yet, so nothing was asked for. Not a fault.
    HealthNothingToServe => "HEALTH_NOTHING_TO_SERVE",
    HealthFirewallOn => "HEALTH_FIREWALL_ON",
    /// `status` — what `ufw status` said. Read from there and never from `is-active`.
    HealthFirewallOff => "HEALTH_FIREWALL_OFF",
    /// `count`, `ports`. Listed for the person to read; not judged.
    HealthOpenPorts => "HEALTH_OPEN_PORTS",
    /// `total_mb`, `used_mb`.
    HealthMemory => "HEALTH_MEMORY",
    /// `cache_mb`. Nobody watching, so nothing is cached — which is not news.
    HealthCacheIdle => "HEALTH_CACHE_IDLE",
    /// `cache_mb`, `total_mb`, `watching`. Small **while somebody is watching**.
    HealthCacheSmall => "HEALTH_CACHE_SMALL",
    /// `cache_mb`, `watching`.
    HealthCacheOk => "HEALTH_CACHE_OK",
    /// `total_mb` — the memory the machine has, since that is why swap was wanted.
    HealthNoSwap => "HEALTH_NO_SWAP",
    /// `used_mb`, `total_mb`.
    HealthSwapInUse => "HEALTH_SWAP_IN_USE",
    /// `used_mb`, `total_mb`.
    HealthSwapOk => "HEALTH_SWAP_OK",
    /// `free_mb`, `total_mb`.
    HealthDisk => "HEALTH_DISK",
    /// `congestion`.
    HealthNetworkTuned => "HEALTH_NETWORK_TUNED",
    /// `congestion`, `qdisc`, `wanted_congestion`, `wanted_qdisc`.
    HealthNetworkUntuned => "HEALTH_NETWORK_UNTUNED",
    /// `kb`.
    HealthReadaheadOk => "HEALTH_READAHEAD_OK",
    /// `kb`, `wanted_kb`.
    HealthReadaheadSmall => "HEALTH_READAHEAD_SMALL",
    HealthNoAutoRestart => "HEALTH_NO_AUTO_RESTART",
    /// `mode`.
    HealthAutoRestart => "HEALTH_AUTO_RESTART",

    // --- why a viewer's picture stops (FR-072) ---
    //
    // A cause and the figures behind it, because this conclusion is sometimes wrong and a
    // conclusion nobody can check is a conclusion nobody can argue with.
    /// `seconds` — too short a stretch to work anything out from.
    StallsTooShort => "STALLS_TOO_SHORT",
    /// `ratio`, `mbit_s`. They are keeping up; the gaps between their requests are a full
    /// buffer, not a stall.
    StallsKeepingUp => "STALLS_KEEPING_UP",
    /// `out_mbit_s`, `capacity_mbit_s`.
    StallsServerLink => "STALLS_SERVER_LINK",
    /// `disk_read_mb_s`, `ratio`.
    StallsDisk => "STALLS_DISK",
    /// `mbit_s`, `average_mbit`, `peak_10s_mbit`.
    StallsFilePeaks => "STALLS_FILE_PEAKS",
    /// `ratio`, `mbit_s`, `in_download_mbit_s`, `skipped`, `restarts`.
    StallsViewerLink => "STALLS_VIEWER_LINK",
    StallsThePlayer => "STALLS_THE_PLAYER",
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
