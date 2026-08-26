//! T233, T234 — measuring what one point of the grid is actually worth.
//!
//! One point is one bitrate at one height. Measuring it means encoding the three reference
//! chunks the way the application would really encode them, and comparing each against the
//! source it came from.
//!
//! **The comparison is made after stretching the result back to the source's own size.**
//! Without that, a rung encoded at 1080 would be compared against a 1080 reference and
//! score beautifully — it would be judged against itself rather than against the film, and
//! every low rung would win. What a viewer sees is a small picture stretched over their
//! whole screen, and that is what is scored here.
//!
//! Carried over from `.claude/skills/vrcast-convert/scripts/measure-ladder.sh` without
//! changing the arithmetic (constitution VI).

use std::path::{Path, PathBuf};

use tokio_util::sync::CancellationToken;

use super::encoders::Encoder;
use super::ffmpeg;
use crate::domain::measure_grid::Cell;
use crate::domain::measured_ladder::Point;
use crate::tasks::process::ManagedProcess;

/// How many threads the scoring may use.
pub const VMAF_THREADS: u32 = 8;

/// The distance between keyframes while measuring.
///
/// Fixed rather than taken from the material: every point of the grid has to be encoded the
/// same way, or the scores compare encodes rather than bitrates.
pub const KEYFRAME_EVERY: u32 = 48;

#[derive(Debug, thiserror::Error)]
pub enum VmafError {
    /// The bundled FFmpeg cannot measure quality.
    #[error("this build of FFmpeg has no libvmaf, so quality cannot be measured")]
    Unavailable,

    #[error(transparent)]
    Ffmpeg(#[from] ffmpeg::FfmpegError),

    #[error("could not run the measurement: {0}")]
    NotRunnable(String),

    /// Every chunk of this point failed. The point has no answer, not a bad answer.
    #[error("nothing could be measured at {bitrate_mbps} Mbit/s and {height}p")]
    NothingMeasured { bitrate_mbps: u64, height: u32 },

    #[error("the measurement was cancelled")]
    Cancelled,
}

/// Whether the bundled build can measure quality at all.
pub async fn available() -> Result<bool, ffmpeg::FfmpegError> {
    Ok(ffmpeg::probe_self().await?.has_libvmaf)
}

/// The peak allowed above a rung's target, in the script's integer arithmetic.
///
/// `MR=$(( BR * 11 / 10 )); [[ "$MR" -le "$BR" ]] && MR=$((BR+1))`. Below ten megabits the
/// tenth disappears in the integer division, and the guard keeps the ceiling above the
/// target rather than equal to it — a ceiling equal to the target is a constant bitrate,
/// which is not what any of this was measured with.
pub fn ceiling_mbps(bitrate_mbps: u64) -> u64 {
    let ceiling = bitrate_mbps * 11 / 10;
    if ceiling <= bitrate_mbps {
        bitrate_mbps + 1
    } else {
        ceiling
    }
}

/// Measure one point of the grid on the given chunks.
///
/// The result is the average over the chunks that encoded. **A chunk that fails is skipped
/// rather than fatal**: two chunks out of three still say something about the material,
/// while refusing the point would leave a hole in the grid and the hull would step over it
/// as if the bitrate had never been tried.
#[allow(clippy::too_many_arguments)]
pub async fn measure_point(
    source: &Path,
    source_width: u32,
    source_height: u32,
    chunk_starts: &[u64],
    chunk_s: u64,
    cell: Cell,
    encoder: &Encoder,
    cancel: &CancellationToken,
) -> Result<Point, VmafError> {
    let ffmpeg_bin = ffmpeg::locate("ffmpeg")?;
    let work = Workspace::make(cell)?;

    let mut scores: Vec<f64> = Vec::new();
    let mut weights: Vec<u64> = Vec::new();

    for at_s in chunk_starts {
        if cancel.is_cancelled() {
            return Err(VmafError::Cancelled);
        }

        match encode_chunk(&ffmpeg_bin, &work, source, *at_s, chunk_s, cell, encoder).await {
            Ok(()) => {}
            Err(e) => {
                tracing::debug!(at_s, ?cell, error = %e, "a chunk of this point would not encode");
                continue;
            }
        }

        if cancel.is_cancelled() {
            return Err(VmafError::Cancelled);
        }

        let score = score_chunk(
            &ffmpeg_bin,
            &work,
            source,
            *at_s,
            chunk_s,
            source_width,
            source_height,
        )
        .await;

        match score {
            Ok(vmaf) => {
                scores.push(vmaf);
                weights.push(
                    ffmpeg::bitrate_of(&work.encoded)
                        .await
                        .unwrap_or(cell.bitrate_mbps * 1_000_000),
                );
            }
            Err(e) => {
                tracing::debug!(at_s, ?cell, error = %e, "a chunk of this point would not score")
            }
        }
        let _ = tokio::fs::remove_file(&work.encoded).await;
    }

    if scores.is_empty() {
        return Err(VmafError::NothingMeasured {
            bitrate_mbps: cell.bitrate_mbps,
            height: cell.height,
        });
    }

    Ok(Point {
        bitrate_mbps: cell.bitrate_mbps,
        height: cell.height,
        actual_bps: weights.iter().sum::<u64>() / weights.len() as u64,
        vmaf: scores.iter().sum::<f64>() / scores.len() as f64,
    })
}

/// A directory of our own, with the two working files named relatively inside it.
///
/// See [`ManagedProcess::spawn_in`] for why the names have to be relative.
struct Workspace {
    dir: PathBuf,
    encoded: PathBuf,
}

impl Workspace {
    const ENCODED: &'static str = "point.mp4";
    const SCORE: &'static str = "score.json";

    fn make(cell: Cell) -> Result<Self, VmafError> {
        let dir = std::env::temp_dir().join(format!(
            "vrcast-vmaf-{}-{}-{}",
            std::process::id(),
            cell.bitrate_mbps,
            cell.height
        ));
        std::fs::create_dir_all(&dir).map_err(|e| VmafError::NotRunnable(e.to_string()))?;
        let encoded = dir.join(Self::ENCODED);
        Ok(Self { dir, encoded })
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Encode one chunk at this point of the grid.
async fn encode_chunk(
    ffmpeg_bin: &Path,
    work: &Workspace,
    source: &Path,
    at_s: u64,
    chunk_s: u64,
    cell: Cell,
    encoder: &Encoder,
) -> Result<(), VmafError> {
    let ceiling = ceiling_mbps(cell.bitrate_mbps);
    let mut args: Vec<String> = vec![
        "-nostdin".into(),
        "-y".into(),
        "-v".into(),
        "error".into(),
        // Seeking before the input: FFmpeg jumps rather than decoding its way there.
        "-ss".into(),
        at_s.to_string(),
        "-t".into(),
        chunk_s.to_string(),
        "-i".into(),
        source.to_string_lossy().into_owned(),
        "-map".into(),
        "0:v:0".into(),
        // `-2` rather than a width of our own: the width follows the height and stays
        // divisible by two, which keeps the aspect of anamorphic and side-by-side material.
        "-vf".into(),
        format!("scale=-2:{}", cell.height),
        "-c:v".into(),
        encoder.ffmpeg_name().to_owned(),
    ];

    // The production profile. Measuring through a different one would answer a question
    // nobody asked: the whole premise is that what is measured here is what will be made.
    let family = super::encoder_args::family_of(encoder);
    args.extend(super::encoder_args::quality_preset(family));
    args.extend(super::encoder_args::bitrate_capped(
        (cell.bitrate_mbps * 1000) as u32,
        (ceiling * 1000) as u32,
        // Buffer equal to the ceiling. A larger one lets bursts through above it, which is
        // what froze viewers once: ceiling 45, buffer 60, peaks at 54.
        (ceiling * 1000) as u32,
    ));

    for a in [
        "-g",
        &KEYFRAME_EVERY.to_string(),
        "-keyint_min",
        &KEYFRAME_EVERY.to_string(),
        "-an",
        "-f",
        "mp4",
        Workspace::ENCODED,
    ] {
        args.push(a.to_owned());
    }

    run_in(ffmpeg_bin, &work.dir, &args).await
}

/// Score one encoded chunk against the source it came from.
async fn score_chunk(
    ffmpeg_bin: &Path,
    work: &Workspace,
    source: &Path,
    at_s: u64,
    chunk_s: u64,
    source_width: u32,
    source_height: u32,
) -> Result<f64, VmafError> {
    // The reference is the source's own frames; the distorted one is stretched back up to
    // meet it. `setpts` on both puts them on the same clock — without it the two inputs
    // start at different timestamps and the filter compares frame 0 against frame 240.
    let graph = format!(
        "[0:v]setpts=PTS-STARTPTS[r];\
         [1:v]scale={source_width}:{source_height}:flags=bicubic,setpts=PTS-STARTPTS[d];\
         [d][r]libvmaf=n_threads={VMAF_THREADS}:log_fmt=json:log_path={}",
        Workspace::SCORE
    );

    let args: Vec<String> = vec![
        "-nostdin".into(),
        "-v".into(),
        "error".into(),
        "-ss".into(),
        at_s.to_string(),
        "-t".into(),
        chunk_s.to_string(),
        "-i".into(),
        source.to_string_lossy().into_owned(),
        "-i".into(),
        Workspace::ENCODED.into(),
        "-lavfi".into(),
        graph,
        "-f".into(),
        "null".into(),
        "-".into(),
    ];

    run_in(ffmpeg_bin, &work.dir, &args).await?;

    let json = tokio::fs::read_to_string(work.dir.join(Workspace::SCORE))
        .await
        .map_err(|e| VmafError::NotRunnable(e.to_string()))?;
    let score = pooled_mean(&json)?;
    let _ = tokio::fs::remove_file(work.dir.join(Workspace::SCORE)).await;
    Ok(score)
}

/// The one number out of libvmaf's report.
///
/// Kept apart from the running so that it can be checked without an encoder — continuous
/// integration has neither a graphics card nor a film.
pub fn pooled_mean(json: &str) -> Result<f64, VmafError> {
    let parsed: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| VmafError::Ffmpeg(ffmpeg::FfmpegError::Unexpected(e.to_string())))?;
    parsed
        .get("pooled_metrics")
        .and_then(|m| m.get("vmaf"))
        .and_then(|v| v.get("mean"))
        .and_then(|m| m.as_f64())
        .ok_or_else(|| {
            VmafError::Ffmpeg(ffmpeg::FfmpegError::Unexpected(String::from(
                "the quality report has no pooled mean in it",
            )))
        })
}

/// Run FFmpeg in the workspace and wait for it.
async fn run_in(ffmpeg_bin: &Path, dir: &Path, args: &[String]) -> Result<(), VmafError> {
    let mut child = ManagedProcess::spawn_in(Some(dir), &ffmpeg_bin.to_string_lossy(), args)
        .map_err(|e| VmafError::NotRunnable(e.to_string()))?;
    let (_stdout, stderr) = child.take_output();

    let complaints = tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut text = String::new();
        if let Some(mut stderr) = stderr {
            let _ = stderr.read_to_string(&mut text).await;
        }
        text
    });

    let status = child
        .wait()
        .await
        .map_err(|e| VmafError::NotRunnable(e.to_string()))?;
    let said = complaints.await.unwrap_or_default();

    if status.success() {
        Ok(())
    } else {
        Err(VmafError::Ffmpeg(ffmpeg::FfmpegError::Unexpected(
            said.trim().to_owned(),
        )))
    }
}
