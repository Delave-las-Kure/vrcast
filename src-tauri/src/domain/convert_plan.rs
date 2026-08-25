//! T109, T110 — what to do with a source: carry it across or re-encode it (FR-022,
//! FR-025, FR-029).
//!
//! There is one target format and it is not up for discussion: MP4, H.264 video in yuv420p,
//! AAC-LC stereo audio, the housekeeping data at the front of the file. It was not chosen
//! out of taste — it is what the VRChat player accepts, and everything else fails to play
//! at all for some viewers.
//!
//! **Why H.264 and not HEVC.** HEVC saves 35–45 % of the bitrate but needs a decoder on the
//! viewer's side, and Windows 10/11 has no system HEVC: a separate package from the store is
//! needed, and the player goes through Media Foundation, so without that package nothing
//! plays no matter what the graphics card can do. Tested in the field on 2026-07-30: **four
//! viewers out of eight could not watch**. So an HEVC source is re-encoded here rather than
//! carried across — even though formally "the video is already compressed" and copying
//! would have been cheaper.
//!
//! The rules below were carried over from `vrcast-convert` unchanged: every one of them was
//! bought with a mistake, and reinventing them would repeat it for certain (R-13).

use super::source::{AudioTrack, SourceFile};
use super::wording::{Detail, DetailCode};
use serde::{Deserialize, Serialize};

/// The default target audio bitrate, in kilobits per second.
pub const AUDIO_KBPS: u32 = 256;

/// The allowance on the audio budget.
///
/// Real AAC runs a little over its nominal size, consistently: a "128k" track weighs
/// 128,634 bits. Without the allowance it would go off to be re-encoded, losing a
/// generation for nothing.
const AUDIO_TOLERANCE_PERCENT: u64 = 10;

/// How far above the target the bitrate ceiling sits.
///
/// It used to be +30 % — and gave a peak 1.36 times above the target: behind a rung marked
/// "35 Mbit/s" hid a demand for almost 50. Measured on 2026-08-02: dropping to +10 % cost
/// about 0.5 dB and took 15 % off what a viewer's connection has to carry.
const MAXRATE_PERCENT: u32 = 110;

/// How much higher the real peak runs than the ceiling that was set.
///
/// The ceiling limits not the instantaneous bitrate but the average over the buffer's
/// verification window, and the peak comes out 5–6 % higher, consistently. The number is in
/// hundredths so integers suffice: to want a peak of P, set the ceiling to P/1.06.
const PEAK_OVER_MAXRATE: u32 = 106;

/// What to do with the video stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VideoAction {
    /// Carry it across as it is — no loss at all, and minutes instead of hours.
    Copy,
    /// Re-encode with no bitrate given: "visually lossless".
    Reencode {
        /// Why it could not simply be carried across. Shown to a person: re-encoding
        /// takes hours, and they are entitled to know what they are paying for.
        reason: Detail,
        level: String,
    },
    /// Re-encode to a given bitrate with the peaks held down.
    ReencodeCapped {
        reason: Detail,
        level: String,
        target_kbps: u32,
        maxrate_kbps: u32,
        bufsize_kbps: u32,
    },
}

/// What to do with the audio.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AudioAction {
    Copy,
    Reencode {
        reason: Detail,
        bitrate_kbps: u32,
        /// Lining the audio up against the picture.
        ///
        /// Required whenever it is re-encoded: AAC writes its priming samples through an
        /// edit list, and the VRChat player does not read one — so the sound drifts. That is
        /// FR-024, and without this field the plan would be incomplete.
        resample_fix: bool,
    },
}

/// What stood in the way of making a plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlanProblem {
    /// There is no audio at all — nothing to choose from.
    NoAudioTracks,
    /// The file has no such track.
    NoSuchTrack { index: usize, available: usize },
    /// A frame height of zero was given.
    HeightZero,
    /// More lines are asked for than the source has.
    HeightAboveSource { asked: u32, source: u32 },
    /// A bitrate of zero was given.
    BitrateZero,
    /// A bitrate noticeably higher than the source's is asked for.
    BitrateAboveSource { asked_kbps: u32, source_kbps: u64 },
}

impl PlanProblem {
    /// What to say about it. The wording belongs to the interface (FR-105, FR-106).
    ///
    /// Contradictions used to carry a ready sentence built where they were detected,
    /// which meant the same complaint could be worded two ways depending on which
    /// check raised it. A code cannot drift like that.
    pub fn detail(&self) -> Detail {
        match self {
            Self::NoAudioTracks => Detail::new(DetailCode::PlanNoAudioTracks),
            // Tracks are counted from one for a person and from zero for ffmpeg. The
            // conversion happens here, once, instead of in each catalogue entry.
            Self::NoSuchTrack { index, available } => Detail::new(DetailCode::PlanNoSuchTrack)
                .with("number", index + 1)
                .with("available", *available),
            Self::HeightZero => Detail::new(DetailCode::PlanHeightZero),
            Self::HeightAboveSource { asked, source } => {
                Detail::new(DetailCode::PlanHeightAboveSource)
                    .with("asked", *asked)
                    .with("source", *source)
            }
            Self::BitrateZero => Detail::new(DetailCode::PlanBitrateZero),
            Self::BitrateAboveSource {
                asked_kbps,
                source_kbps,
            } => Detail::new(DetailCode::PlanBitrateAboveSource)
                .with("asked_kbps", *asked_kbps)
                .with("source_kbps", *source_kbps),
        }
    }
}

/// What a person wants out of the preparation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConvertRequest {
    /// Which audio track to take.
    pub audio_track: usize,
    /// The target video bitrate, in kilobits. Empty means do not set one and compress so
    /// that the eye sees no loss.
    pub target_kbps: Option<u32>,
    /// The target frame height. Empty means leave it alone.
    pub height: Option<u32>,
}

/// A finished preparation plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConvertPlan {
    pub video: VideoAction,
    pub audio: AudioAction,
    /// The index of the chosen track.
    pub audio_track: usize,
    /// Frames between keyframes.
    ///
    /// A keyframe once a second at any frame rate. A constant here would be a mistake: 48
    /// was written for 48-frame video and meant "once a second", while on 24-frame material
    /// it gave one every two.
    pub gop: u32,
    /// Whether to bring high dynamic range down to the ordinary one.
    pub tonemap: bool,
    /// Requested frame height, as asked for. Kept even when it equals the source
    /// height: the command builder needs to tell "not asked" from "asked for the
    /// same", and only the former may skip the scaling filter.
    pub requested_height: Option<u32>,
    /// The housekeeping data at the front of the file — otherwise a viewer waits for the
    /// tail to download (FR-023).
    pub faststart: bool,
}

impl ConvertPlan {
    /// Whether the quality will be left untouched.
    pub fn lossless(&self) -> bool {
        self.video == VideoAction::Copy && self.audio == AudioAction::Copy
    }
}

/// The H.264 compatibility level, by **two** limits at once.
///
/// Checking only the frame size is not enough, and that is a recorded mistake: 1922×1082 at
/// 48 frames is 8228 macroblocks per frame (which almost fits 4.1) and 394,944 per second
/// against the 4.1 limit of 245,760 — an excess of 1.6 times. A strict decoder is entitled
/// to refuse an understated level; an overstated one is always safe.
pub fn h264_level(width: u32, height: u32, fps: u32) -> &'static str {
    // A macroblock is 16×16, and a partial one counts too: 1922 gives 121 columns, not 120.
    let mb = u64::from(width.div_ceil(16)) * u64::from(height.div_ceil(16));
    let mbps = mb * u64::from(fps.max(1));

    match () {
        _ if mb <= 8_192 && mbps <= 245_760 => "4.1",
        _ if mb <= 8_704 && mbps <= 522_240 => "4.2",
        _ if mb <= 22_080 && mbps <= 589_824 => "5.0",
        _ if mb <= 36_864 && mbps <= 983_040 => "5.1",
        _ => "5.2",
    }
}

/// The ceiling and the buffer for a given target bitrate.
///
/// It returns kilobits. Counting in megabits will not do, and that is a recorded mistake of
/// its own: at a target of 8 Mbit/s the integer `8 * 11 / 10` gives exactly 8 — the ceiling
/// equals the target, there is no buffer at all, and out comes constant-bitrate mode, which
/// lost in the measurements. At the old +30 % this never showed (`8*13/10 = 10`); at +10 %
/// it broke quietly.
///
/// The buffer equals the ceiling deliberately: a large buffer allows a surge above the
/// ceiling, and that is what froze viewers — it used to be `ceiling 45 / buffer 60`, with
/// peaks of 54 Mbit/s.
pub fn peak_control(target_kbps: u32) -> (u32, u32) {
    let maxrate = target_kbps.saturating_mul(MAXRATE_PERCENT) / 100;
    // The ceiling must be strictly above the target: equality is the very constant
    // bitrate all this arithmetic exists to get away from.
    let maxrate = maxrate.max(target_kbps.saturating_add(1));
    (maxrate, maxrate)
}

/// What ceiling to set so that the real peak does not exceed a given one.
///
/// The inverse of [`peak_control`]: a viewer's connection is sized for the peak, not for
/// the average.
pub fn maxrate_for_peak(peak_kbps: u32) -> u32 {
    peak_kbps.saturating_mul(100) / PEAK_OVER_MAXRATE
}

/// Make a plan.
///
/// It returns **every** objection at once rather than the first: there are often several,
/// and a person needs to see the whole list rather than deal with one per round.
pub fn plan(
    source: &SourceFile,
    request: &ConvertRequest,
) -> Result<ConvertPlan, Vec<PlanProblem>> {
    let mut problems = Vec::new();

    if source.audio_tracks.is_empty() {
        problems.push(PlanProblem::NoAudioTracks);
    } else if source.track(request.audio_track).is_none() {
        problems.push(PlanProblem::NoSuchTrack {
            index: request.audio_track,
            available: source.audio_tracks.len(),
        });
    }

    if let Some(h) = request.height {
        if h == 0 {
            problems.push(PlanProblem::HeightZero);
        } else if h > source.height {
            // There is nothing to stretch: detail the source does not have will not
            // appear, and the file swells. This is exactly the case FR-029 means by "do
            // not allow it quietly".
            problems.push(PlanProblem::HeightAboveSource {
                asked: h,
                source: source.height,
            });
        }
    }

    if let Some(kbps) = request.target_kbps {
        if kbps == 0 {
            problems.push(PlanProblem::BitrateZero);
        } else if u64::from(kbps) * 1000 > source.bitrate_bps.saturating_mul(2) {
            problems.push(PlanProblem::BitrateAboveSource {
                asked_kbps: kbps,
                source_kbps: source.bitrate_bps / 1000,
            });
        }
    }

    if !problems.is_empty() {
        return Err(problems);
    }

    let track = source
        .track(request.audio_track)
        .expect("the track was checked above");

    let tonemap = source.is_hdr();
    let downscale = request.height.is_some_and(|h| h != source.height);
    let level = h264_level(
        source.width,
        request.height.unwrap_or(source.height),
        source.fps,
    );

    Ok(ConvertPlan {
        video: video_action(source, request, level, tonemap, downscale),
        audio: audio_action(track),
        audio_track: request.audio_track,
        // A keyframe once a second at any frame rate.
        gop: source.fps.max(1),
        tonemap,
        requested_height: request.height,
        faststart: true,
    })
}

fn video_action(
    source: &SourceFile,
    request: &ConvertRequest,
    level: &str,
    tonemap: bool,
    downscale: bool,
) -> VideoAction {
    // Carrying across without re-encoding is possible only when the stream need not be
    // touched at all: any change to the picture requires decoding it, and once decoded it
    // can no longer be put back the way it was.
    let reason = if !source.video_codec.eq_ignore_ascii_case("h264") {
        Some(Detail::new(DetailCode::ReasonVideoNotH264).with("codec", source.video_codec.clone()))
    } else if !source.pix_fmt.eq_ignore_ascii_case("yuv420p") {
        // Ten-bit H.264 is formally the same codec, but a strict decoder refuses it.
        Some(Detail::new(DetailCode::ReasonVideoPixFmt).with("pix_fmt", source.pix_fmt.clone()))
    } else if tonemap {
        Some(Detail::new(DetailCode::ReasonTonemap))
    } else if downscale {
        Some(Detail::new(DetailCode::ReasonResize))
    } else {
        None
    };

    match (reason, request.target_kbps) {
        (None, None) => VideoAction::Copy,
        // A bitrate was given — re-encoding is unavoidable even when the stream is
        // compatible: otherwise the request goes unmet while a person believes it was
        // honoured.
        (reason, Some(kbps)) => {
            let (maxrate_kbps, bufsize_kbps) = peak_control(kbps);
            VideoAction::ReencodeCapped {
                reason: reason.unwrap_or_else(|| Detail::new(DetailCode::ReasonTargetBitrate)),
                level: level.to_owned(),
                target_kbps: kbps,
                maxrate_kbps,
                bufsize_kbps,
            }
        }
        (Some(reason), None) => VideoAction::Reencode {
            reason,
            level: level.to_owned(),
        },
    }
}

fn audio_action(track: &AudioTrack) -> AudioAction {
    // All three conditions are required, and that is a recorded mistake: checking only the
    // codec let a six-channel track through against the target format — given AAC 5.1 on
    // the way in, the file went out with six channels.
    let codec_fits = track.codec.eq_ignore_ascii_case("aac");
    let is_stereo = track.channels == 2;
    let budget = u64::from(AUDIO_KBPS) * 1000 * (100 + AUDIO_TOLERANCE_PERCENT) / 100;
    let within_budget = track.bitrate_bps.is_none_or(|b| b <= budget);

    if codec_fits && is_stereo && within_budget {
        return AudioAction::Copy;
    }

    let reason = if !codec_fits {
        Detail::new(DetailCode::ReasonAudioNotAac).with("codec", track.codec.clone())
    } else if !is_stereo {
        Detail::new(DetailCode::ReasonAudioChannels).with("channels", track.channels)
    } else {
        Detail::new(DetailCode::ReasonAudioTooFat)
    };

    AudioAction::Reencode {
        reason,
        bitrate_kbps: AUDIO_KBPS,
        resample_fix: true,
    }
}
