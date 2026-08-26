//! T192 — asking the material how many bits it wants.
//!
//! **Why this exists at all.** The top of a ladder used to be a constant — 35 Mbit/s,
//! capped by the source. A constant knows nothing about the material: on a 4K upscale of
//! animation it called for 35 where a VMAF measurement showed 14 was enough, and on dense
//! live action it understated. So the material is asked instead: three pieces are encoded
//! with the quality pinned and the bitrate left free, and however many bits they take is
//! the answer.
//!
//! Carried over from `.claude/skills/vrcast-convert/scripts/plan-ladder.sh` without
//! changing the arithmetic (constitution, principle VI).
//!
//! **Where it is honest about itself.** This finds where the material stops asking, not
//! where quality stops improving. Only a VMAF measurement finds the latter, and the
//! project's own policy is to run one for every film. This is the fast path.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::encoders::Encoder;
use super::ffmpeg;

/// The quality the probe pins.
///
/// **Calibrated against NVIDIA's encoder**, so that the anchor lands a little above the
/// point of saturation. The calibration table is in `measure-ladder.sh`. A quantiser of 26
/// does not mean the same thing to AMD's encoder, to Intel's, or to x264 — the intent
/// carries over, the number does not, and an answer taken with any of the others is marked
/// as uncalibrated rather than passed off as a measurement.
const PROBE_CQ: u32 = 26;

/// How long each piece is, and where in the film they are taken from.
const PIECE_SECONDS: u64 = 10;
const AT_PERCENT: [u64; 3] = [25, 50, 75];

/// How far from the ends to stay.
///
/// Titles and credits are not the film: they are flat, they compress to nothing, and a
/// piece taken from them would say the whole film is easy.
const EDGE_SECONDS: u64 = 180;
const TAIL_SECONDS: u64 = 200;
const SHORT_FILM_SECONDS: u64 = 400;

/// What the probe found.
///
/// **What to do when it found nothing is not here.** Falling back on the old constant means
/// holding that constant down to what the source allows, and that allowance carries the
/// heavier-codec multiplier with it — a rule, and one that belongs with the other rules
/// where it can be checked without an encoder. It lived here once and took the source's own
/// bitrate instead, which cut every ladder over an HEVC master by a third.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Probed {
    /// How many bits the material asked for, in bits per second. `None` means the probe
    /// could not run at all.
    pub measured_bps: Option<u64>,
    /// How many pieces actually encoded. Fewer than three still gives an answer.
    pub pieces: usize,
    /// What did the encoding.
    pub encoder: Encoder,
    /// What has to be said about this answer, if anything.
    pub notice: Option<crate::domain::wording::Detail>,
}

/// Ask the material.
///
/// `duration_s` is the film's length; the pieces are placed inside it.
pub async fn probe(path: &Path, duration_s: f64, encoder: &Encoder) -> Probed {
    let total = duration_s.max(0.0) as u64;
    let mut taken: Vec<u64> = Vec::new();

    for percent in AT_PERCENT {
        let at = piece_start(total, percent);
        match encode_piece(path, at, encoder).await {
            Ok(bps) if bps > 0 => taken.push(bps),
            Ok(_) => {}
            Err(e) => {
                // One piece failing is not a failure: the other two still answer. Only all
                // three failing falls back.
                tracing::debug!(at, error = %e, "a piece of the probe would not encode");
            }
        }
    }

    let calibrated = super::encoder_args::family_of(encoder).is_calibrated();
    if taken.is_empty() {
        return Probed {
            measured_bps: None,
            pieces: 0,
            encoder: encoder.clone(),
            notice: Some(crate::domain::wording::Detail::new(
                crate::domain::wording::DetailCode::NoticeProbeFailed,
            )),
        };
    }

    Probed {
        measured_bps: Some(taken.iter().sum::<u64>() / taken.len() as u64),
        pieces: taken.len(),
        encoder: encoder.clone(),
        // **The one thing that must be said out loud.** The quality the probe pins was
        // calibrated against NVIDIA's encoder. Every other one — AMD's, Intel's, x264 —
        // lands somewhere else at the same setting, so the anchor is shifted. The ladder
        // still comes out, and a person should know its top rests on a number taken with a
        // different ruler.
        notice: (!calibrated).then(|| {
            crate::domain::wording::Detail::new(
                crate::domain::wording::DetailCode::NoticeProbeUncalibrated,
            )
        }),
    }
}

/// Where a piece starts, keeping clear of titles and credits.
///
/// A short film has no room to keep clear in, and is taken as it comes: a piece from the
/// middle of a twenty-minute episode is the episode.
pub fn piece_start(total_s: u64, percent: u64) -> u64 {
    let mut at = total_s * percent / 100;
    if total_s > SHORT_FILM_SECONDS {
        at = at.min(total_s.saturating_sub(TAIL_SECONDS));
        at = at.max(EDGE_SECONDS);
    }
    at
}

/// Encode one piece and see what it weighed.
async fn encode_piece(
    path: &Path,
    at_s: u64,
    encoder: &Encoder,
) -> Result<u64, ffmpeg::FfmpegError> {
    let ffmpeg_bin = ffmpeg::locate("ffmpeg")?;
    let out = std::env::temp_dir().join(format!("vrcast-probe-{at_s}-{}.mp4", std::process::id()));

    let mut args: Vec<String> = vec![
        "-nostdin".into(),
        "-y".into(),
        "-v".into(),
        "error".into(),
        // Seeking before the input is what makes this quick: ffmpeg jumps rather than
        // decoding its way there. On a two-hour film the difference is minutes.
        "-ss".into(),
        at_s.to_string(),
        "-t".into(),
        PIECE_SECONDS.to_string(),
        "-i".into(),
        path.to_string_lossy().into_owned(),
        "-map".into(),
        "0:v:0".into(),
        "-c:v".into(),
        encoder.ffmpeg_name().to_owned(),
    ];

    // The production profile, so that what the probe measures is what the encoding will
    // later produce. A different profile would measure a different encoder's appetite.
    //
    // Through the dialect module: `-cq` is NVIDIA's own option and AMD's encoder refuses to
    // start when handed it. Written by hand here, the probe would have failed on every
    // piece on an AMD machine and quietly fallen back to the constant — which is the
    // failure that looks least like one.
    let family = super::encoder_args::family_of(encoder);
    args.extend(super::encoder_args::quality_pinned(family, PROBE_CQ));
    args.extend(super::encoder_args::quality_preset(family));

    for a in [
        "-profile:v",
        "high",
        "-b:v",
        "0",
        "-maxrate",
        "0",
        "-bufsize",
        "0",
        "-g",
        "48",
        "-keyint_min",
        "48",
        "-an",
        "-f",
        "mp4",
    ] {
        args.push(a.to_owned());
    }
    args.push(out.to_string_lossy().into_owned());

    let status = tokio::process::Command::new(&ffmpeg_bin)
        .args(&args)
        .output()
        .await
        .map_err(|e| ffmpeg::FfmpegError::NotRunnable(e.to_string()));

    let bitrate = match status {
        Ok(o) if o.status.success() => ffmpeg::bitrate_of(&out).await.unwrap_or(0),
        Ok(o) => {
            let _ = tokio::fs::remove_file(&out).await;
            return Err(ffmpeg::FfmpegError::Unexpected(
                String::from_utf8_lossy(&o.stderr).trim().to_owned(),
            ));
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&out).await;
            return Err(e);
        }
    };

    let _ = tokio::fs::remove_file(&out).await;
    Ok(bitrate)
}
