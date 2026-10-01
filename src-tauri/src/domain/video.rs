//! T672 — a video in work: the rules of the pipeline, without a database, a film or a server.
//!
//! **One video goes the whole way in one place** (the owner's decision of 2026-10-01): a plan,
//! then «Start», then measuring, encoding the rungs, sending, cutting, checking and links — and
//! after a restart it carries on from the stage it was at. The work itself is not done here
//! and not done twice: it is the measurement with its chain onwards (T438) and `ladder_build`,
//! which recognises the rungs already on the server. What lives here is what can be decided
//! from facts alone — which stage a task's own stage code belongs to, which actions a problem
//! offers, what a state allows — so that every rule can be checked without running anything.

use serde::{Deserialize, Serialize};

use super::ladder::Rung;
use super::source::SourceFile;
use super::wording::DetailCode;
use crate::error::{AppError, ErrorCode};

/// Declares a string enumeration from one list, like `tasks::state` does: the enum, `ALL`,
/// `as_str` and `parse` cannot drift apart.
macro_rules! str_enum {
    (
        $(#[$outer:meta])*
        $vis:vis enum $enum_name:ident {
            $($(#[$meta:meta])* $name:ident => $code:literal),+ $(,)?
        }
    ) => {
        $(#[$outer])*
        $vis enum $enum_name {
            $($(#[$meta])* $name,)+
        }

        impl $enum_name {
            pub const ALL: &'static [$enum_name] = &[$(Self::$name),+];

            pub fn as_str(&self) -> &'static str {
                match self { $(Self::$name => $code,)+ }
            }

            pub fn parse(s: &str) -> Option<Self> {
                match s {
                    $($code => Some(Self::$name),)+
                    _ => None,
                }
            }
        }
    };
}

str_enum! {
    /// Where a video is on its way, in the order it goes.
    ///
    /// Inside the build the stages alternate per rung — encoding, sending, encoding,
    /// sending — and the stage shown is whichever the work is in now, with "rung k of n".
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum VideoStage {
        /// The plan is made (or being made) and waits for «Start».
        Planned => "planned",
        /// The quality is being measured on the film itself.
        Measuring => "measuring",
        /// A rung is being encoded here and checked to decode.
        Encoding => "encoding",
        /// A rung is on its way to the server.
        Uploading => "uploading",
        /// The server is cutting the rungs into segments.
        Cutting => "cutting",
        /// Every rung is being asked for over the address a viewer would use.
        Verifying => "verifying",
        /// Served whole; the link is ready.
        Done => "done",
    }
}

str_enum! {
    /// What is happening to a video right now — kept apart from where it is.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum VideoState {
        /// Added; its plan is being worked out.
        Planning => "planning",
        /// The plan is there; nothing runs until «Start».
        Ready => "ready",
        /// Going: a task of its current stage is queued or running.
        Working => "working",
        /// Paused by a person. Survives a restart as a pause.
        Paused => "paused",
        /// Stopped on a problem that names what to press.
        Problem => "problem",
        /// Stop was pressed and the work is still winding down (the server's cutting is
        /// confirmed stopped before anything is called cancelled — principle III).
        Cancelling => "cancelling",
        /// Stopped by a person. «Retry» carries on from the stage it was at.
        Cancelled => "cancelled",
        /// Finished: served whole, the link is ready.
        Done => "done",
    }
}

str_enum! {
    /// What a person can press about a problem.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum VideoAction {
        /// `video_retry(id)`: carry on from the stage it stopped at.
        Retry => "retry",
        /// `video_retry(id, confirmed: true)`: build although somebody is watching, or although
        /// the checker objects to the ladder — the person has seen why and accepts it.
        BuildAnyway => "build_anyway",
        /// Open the rung editor for this video (`video_set_rungs`), then `video_retry`.
        EditRungs => "edit_rungs",
        /// `video_replace(id)`: the name is taken; build into the medium that has it.
        Replace => "replace",
        /// `video_set_name(id, …)`: the name is taken; choose another, then `video_retry`.
        Rename => "rename",
    }
}

impl VideoState {
    /// Whether a task of this video may be alive right now.
    pub fn is_going(&self) -> bool {
        matches!(self, Self::Working | Self::Cancelling)
    }

    /// Whether the video still has to get somewhere — what a restart carries on.
    pub fn is_unfinished(&self) -> bool {
        !matches!(self, Self::Done | Self::Cancelled)
    }
}

/// What a person may do to a video in a given state.
///
/// One table, so that a command and a screen cannot disagree about what is offered. Each
/// command asks it before doing anything, and refuses with `VIDEO_NOT_NOW` otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    Start,
    Pause,
    Resume,
    Cancel,
    Retry,
    Remove,
    /// Change the audio track. Only before anything is encoded: a variant already on the
    /// server would be recognised as done with the old track in it.
    SetAudio,
    /// Change the name. Only before the medium exists in the library.
    SetName,
    /// Edit the rungs. Before start, and on a problem — never under a running build.
    SetRungs,
    /// Take over the medium that holds the slug.
    Replace,
}

/// Whether `act` is allowed for a video at `stage` in `state`, whose medium is or is not
/// already in the library.
pub fn allowed(act: Act, state: VideoState, stage: VideoStage, has_medium: bool) -> bool {
    use VideoState as S;
    match act {
        Act::Start => matches!(state, S::Ready | S::Planning),
        Act::Pause => state == S::Working,
        Act::Resume => state == S::Paused,
        Act::Cancel => matches!(state, S::Working | S::Paused | S::Problem | S::Planning),
        Act::Retry => matches!(state, S::Problem | S::Cancelled),
        Act::Remove => !state.is_going(),
        Act::SetAudio => stage <= VideoStage::Measuring && !state.is_going() && state != S::Done,
        Act::SetName => !has_medium && !state.is_going() && state != S::Done,
        Act::SetRungs => matches!(state, S::Planning | S::Ready | S::Problem | S::Cancelled),
        Act::Replace => state == S::Problem && !has_medium,
    }
}

/// What a video does when the application starts again (T672, the owner's decision of
/// 2026-10-01, which lifts T446's «not yet» for this path).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterRestart {
    /// Its plan was still being made: make it again.
    PlanAgain,
    /// It was going: carry on from the stage it was at, by itself.
    CarryOn,
    /// Stop had been pressed and the work was winding down: it is stopped now.
    NowCancelled,
    /// Waiting for a person — for «Start», for «Continue» after their own pause, for a
    /// choice about a problem — or finished. Nothing runs unasked.
    Leave,
}

/// The rule for one video, from its state alone.
///
/// **A pause a person pressed stays a pause.** Somebody may have paused precisely to take the
/// machine back for the evening; carrying on because the application was restarted would undo
/// their decision behind their back.
pub fn after_restart(state: VideoState) -> AfterRestart {
    match state {
        VideoState::Planning => AfterRestart::PlanAgain,
        VideoState::Working => AfterRestart::CarryOn,
        VideoState::Cancelling => AfterRestart::NowCancelled,
        VideoState::Ready
        | VideoState::Paused
        | VideoState::Problem
        | VideoState::Cancelled
        | VideoState::Done => AfterRestart::Leave,
    }
}

/// Which stage a task's own stage code belongs to (T672).
///
/// `None` for codes that say nothing about where the video is — the end of a task, a stop
/// still being confirmed — so the video keeps the stage it had.
pub fn stage_of(code: DetailCode) -> Option<VideoStage> {
    use DetailCode as D;
    match code {
        D::StagePreparingMeasurement | D::StageMeasuringQuality | D::StageCheckingLoan => {
            Some(VideoStage::Measuring)
        }
        // `StageConverting` is what an encode of one rung reports while it runs; the build
        // announces the rung with `StageBuildingLadder` and checks it with `StageValidating`.
        D::StageBuildingLadder | D::StageConverting | D::StageValidating => {
            Some(VideoStage::Encoding)
        }
        D::StageSendingVariant => Some(VideoStage::Uploading),
        D::StageCuttingSegments => Some(VideoStage::Cutting),
        D::StageVerifyingLadder => Some(VideoStage::Verifying),
        _ => None,
    }
}

/// Which rung the build is at, counting from one, out of what `StageBuildingLadder` reports.
///
/// The build reports `done / (n + 1)` as it takes up each rung (`tasks::ladder_build::run`),
/// so the number is read back out of the share rather than kept in a second place that could
/// disagree with it.
pub fn rung_at(progress: f64, rungs: usize) -> u32 {
    if rungs == 0 {
        return 0;
    }
    let done = (progress.clamp(0.0, 1.0) * (rungs as f64 + 1.0)).round() as usize;
    (done + 1).min(rungs) as u32
}

/// What a person can press about this error.
///
/// **Retry is offered almost everywhere**, because almost every stop is about something
/// outside — a connection, a full disk freed meanwhile, a viewer gone. Where pressing it would
/// only repeat the refusal, it is not offered: a taken name stays taken until a choice is made.
pub fn actions_for(error: &AppError) -> Vec<VideoAction> {
    use VideoAction as A;
    match error.code {
        ErrorCode::SlugTaken => vec![A::Replace, A::Rename],
        ErrorCode::FileInUse => vec![A::BuildAnyway, A::Retry],
        ErrorCode::LadderObjection => vec![A::BuildAnyway, A::EditRungs],
        ErrorCode::LadderNotMeasured => vec![A::EditRungs, A::Retry],
        ErrorCode::RungAboveSource | ErrorCode::BufsizeTooLarge | ErrorCode::LevelExceeded => {
            vec![A::EditRungs, A::Retry]
        }
        _ => vec![A::Retry],
    }
}

/// The title a file is offered under: its own name without the extension.
pub fn title_of(path: &str) -> String {
    let name = std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .trim();
    if name.is_empty() {
        String::from("video")
    } else {
        name.to_owned()
    }
}

/// The short name a title gets, or a made-up one when nothing usable is left of it.
///
/// Never a refusal: a film called «……» is still a film, and refusing it for its name
/// would stop it before its plan. The made-up one is visible and can be changed.
pub fn slug_for(title: &str, fallback_seed: &str) -> String {
    super::media::slugify(title).unwrap_or_else(|| {
        let tail: String = fallback_seed
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(8)
            .collect::<String>()
            .to_lowercase();
        format!("video-{tail}")
    })
}

/// The audio track offered when nobody chose: the one marked default, else the first.
pub fn default_audio(source: &SourceFile) -> usize {
    source.default_track().map(|t| t.index).unwrap_or(0)
}

/// Whether a rung is carried across without encoding (the same test `ladder_build::work_for`
/// makes — a rung that *is* the source asks for nothing).
pub fn is_copy(rung: &Rung, source: &SourceFile) -> bool {
    rung.height == source.height
        && rung.width == source.width
        && rung.bitrate_bps >= source.bitrate_bps
}

/// How fast an encoder makes pictures, when nothing has been timed on this machine yet: in
/// pixels of output a second.
///
/// **A rough model, and said to be one** (`EncodeEstimate::Model`). A graphics card makes a
/// 1080p rung at well over real time; x264 on the quality preset this project uses is near
/// half of real time at 1080p24. Every encode timed here replaces the model with what this
/// machine actually did (`EncodeEstimate::ThisMachine`).
pub const MODEL_PIXELS_PER_S_HARDWARE: f64 = 1920.0 * 1080.0 * 150.0;
pub const MODEL_PIXELS_PER_S_SOFTWARE: f64 = 1920.0 * 1080.0 * 12.0;

/// The decode check after each encode, as a share of the encode's own time.
pub const VALIDATE_SHARE: f64 = 0.15;

/// Pixels of output an encode of `rung` over `source` makes: the work an encoder's speed is
/// counted against.
pub fn pixels_of(rung: &Rung, source: &SourceFile) -> f64 {
    f64::from(rung.width)
        * f64::from(rung.height)
        * f64::from(source.fps.max(1))
        * source.duration_s.max(0.0)
}

/// How long encoding these rungs should take, in seconds, at `pixels_per_s`.
///
/// Rungs carried across untouched cost nothing here. `None` when the film's length is not
/// known — a number made up for that case would look like an estimate and be a guess.
pub fn encode_seconds(rungs: &[Rung], source: &SourceFile, pixels_per_s: f64) -> Option<u64> {
    if source.duration_s <= 0.0 || pixels_per_s <= 0.0 {
        return None;
    }
    let pixels: f64 = rungs
        .iter()
        .filter(|r| !is_copy(r, source))
        .map(|r| pixels_of(r, source))
        .sum();
    Some((pixels / pixels_per_s * (1.0 + VALIDATE_SHARE)).round() as u64)
}

/// The middle of what this machine has done, or nothing when it has done nothing.
pub fn middle_speed(mut timed: Vec<f64>) -> Option<f64> {
    timed.retain(|s| s.is_finite() && *s > 0.0);
    if timed.is_empty() {
        return None;
    }
    timed.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(timed[timed.len() / 2])
}

/// Time left in a stage at a steady pace: what has been done since the stage began, against
/// how long that took. `None` until there is something to divide by.
pub fn eta_s(elapsed_s: f64, progress_from: f64, progress_now: f64) -> Option<i64> {
    let moved = progress_now - progress_from;
    if elapsed_s < 2.0 || moved <= 0.005 {
        return None;
    }
    let left = (1.0 - progress_now).max(0.0);
    Some((elapsed_s / moved * left).round() as i64)
}
