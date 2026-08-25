//! T117, T118 — running the actual conversion (FR-022, FR-023, FR-024, FR-028).
//!
//! Two halves, deliberately separated.
//!
//! [`build_args`] is pure: a plan goes in, an FFmpeg command line comes out.
//! Every flag that matters was bought with a bug in this project, and a pure
//! function is the only way to keep them under test without encoding a video
//! on every run.
//!
//! [`run`] does the encoding. It spawns FFmpeg in its own process group so the
//! whole tree dies with us (constitution, principle III): an orphaned encoder
//! keeps writing into the output file long after the task is gone, and the file
//! looks finished while being garbage.

use super::ffmpeg;
use crate::domain::convert_plan::{AudioAction, ConvertPlan, VideoAction};
use crate::domain::source::SourceFile;
use crate::media::encoders::Encoder;
use crate::tasks::engine::TaskContext;
use crate::tasks::process::ManagedProcess;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum ConvertError {
    #[error(transparent)]
    Ffmpeg(#[from] ffmpeg::FfmpegError),

    #[error("could not start the encoder: {0}")]
    Spawn(String),

    /// The encoder ran and refused. Its own complaint is carried along: it is
    /// cryptic, but it can be searched for, and "encoding failed" cannot.
    #[error("encoding failed: {0}")]
    Failed(String),

    #[error("cancelled")]
    Cancelled,
}

pub type Result<T> = std::result::Result<T, ConvertError>;

/// Everything needed to build the command line.
pub struct ConvertJob<'a> {
    pub source: &'a SourceFile,
    pub plan: &'a ConvertPlan,
    pub encoder: &'a Encoder,
    pub out_path: &'a str,
}

/// Append one argument.
///
/// A free function rather than a closure: a closure capturing the vector holds a
/// mutable borrow for as long as it lives, and nothing else can touch the vector
/// while it does — which is exactly what the per-stream builders need to do.
fn push(args: &mut Vec<String>, value: &str) {
    args.push(value.to_owned());
}

/// Build the FFmpeg command line.
///
/// Ordering follows FFmpeg's own grammar: global flags, input, stream mapping,
/// then per-stream codec options, then output. Getting this order wrong does not
/// fail loudly — FFmpeg silently applies an option to the wrong stream.
pub fn build_args(job: &ConvertJob<'_>) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    let a = &mut args;

    // Overwrite without asking, and never read stdin. Without `-nostdin` FFmpeg
    // grabs the terminal's input and a background job can hang forever waiting
    // for a keypress nobody will ever make.
    push(a, "-hide_banner");
    push(a, "-nostdin");
    push(a, "-y");

    // Machine-readable progress on stdout, human noise off. Parsing the pretty
    // status line instead would tie us to a format that changes between releases.
    push(a, "-progress");
    push(a, "pipe:1");
    push(a, "-nostats");

    push(a, "-i");
    push(a, job.source.path.as_str());

    // Exactly one video and one audio stream, chosen explicitly. Letting FFmpeg
    // pick means it picks differently on files with several video streams
    // (cover art counts as one), and the result is a still image with sound.
    push(a, "-map");
    push(a, "0:v:0");
    push(a, "-map");
    push(a, &format!("0:a:{}", job.plan.audio_track));
    // Subtitles and data streams are dropped: MP4 for the VRChat player carries
    // video and audio, and an unexpected stream can make the muxer refuse.
    push(a, "-map_metadata");
    push(a, "-1");

    if let Some(filter) = video_filter(job) {
        push(a, "-vf");
        push(a, &filter);
    }

    video_args(job, a);
    audio_args(job, a);

    // Moov atom at the front (FR-023). Without it the player has to download the
    // tail before it can start, and seeking is impossible over plain HTTP.
    push(a, "-movflags");
    push(a, "+faststart");
    push(a, "-f");
    push(a, "mp4");
    push(a, job.out_path);

    args
}

/// The `-vf` chain, if any filtering is needed at all.
///
/// Kept as one chain because FFmpeg accepts only one `-vf`; a second one silently
/// replaces the first rather than adding to it.
fn video_filter(job: &ConvertJob<'_>) -> Option<String> {
    let mut steps: Vec<String> = Vec::new();

    if job.plan.tonemap {
        // Software tonemapping through zscale. The GPU path (libplacebo/Vulkan)
        // is faster but needs a working Vulkan stack, and on a machine without
        // one it fails at startup rather than falling back. Correct-everywhere
        // beats fast-sometimes for a step that runs once per file.
        steps.push(String::from(
            "zscale=transfer=linear:npl=100,tonemap=tonemap=hable:desat=0,\
             zscale=primaries=bt709:transfer=bt709:matrix=bt709",
        ));
    }

    if let Some(height) = target_height(job) {
        // `-2` keeps the aspect ratio and rounds the width to an even number.
        // H.264 in yuv420p cannot encode odd dimensions at all, and `-1` produces
        // them on plenty of real sources.
        steps.push(format!("scale=-2:{height}"));
    }

    // The pixel format goes last, after everything that could have changed it.
    if !steps.is_empty() {
        steps.push(String::from("format=yuv420p"));
    }

    (!steps.is_empty()).then(|| steps.join(","))
}

/// Requested frame height, when it differs from the source.
fn target_height(job: &ConvertJob<'_>) -> Option<u32> {
    match &job.plan.video {
        VideoAction::Copy => None,
        _ => job
            .plan
            .requested_height
            .filter(|h| *h != job.source.height),
    }
}

fn video_args(job: &ConvertJob<'_>, args: &mut Vec<String>) {
    match &job.plan.video {
        VideoAction::Copy => {
            push(args, "-c:v");
            push(args, "copy");
        }
        VideoAction::Reencode { level, .. } => {
            push(args, "-c:v");
            push(args, job.encoder.ffmpeg_name());
            // Visually lossless. There is no target bitrate here, so quality is
            // pinned instead and the file lands wherever it lands.
            if matches!(job.encoder, Encoder::Software) {
                push(args, "-crf");
                push(args, "16");
                push(args, "-preset");
                push(args, "slow");
            } else {
                push(args, "-cq");
                push(args, "16");
            }
            common_video_args(job, level, args);
        }
        VideoAction::ReencodeCapped {
            level,
            target_kbps,
            maxrate_kbps,
            bufsize_kbps,
            ..
        } => {
            push(args, "-c:v");
            push(args, job.encoder.ffmpeg_name());
            push(args, "-b:v");
            push(args, &format!("{target_kbps}k"));
            push(args, "-maxrate");
            push(args, &format!("{maxrate_kbps}k"));
            // Buffer equals the ceiling on purpose. A larger buffer lets bursts
            // run above the ceiling, and that is what froze viewers: a ceiling of
            // 45 with a buffer of 60 produced 54 Mbit/s peaks.
            push(args, "-bufsize");
            push(args, &format!("{bufsize_kbps}k"));
            // No `-minrate`: a floor forces easy scenes to be padded with bits
            // they do not need, which is constant-bitrate behaviour, and constant
            // bitrate lost the measurement it was compared against.
            if matches!(job.encoder, Encoder::Software) {
                push(args, "-preset");
                push(args, "slow");
            }
            common_video_args(job, level, args);
        }
    }
}

/// Options shared by both re-encoding paths.
fn common_video_args(job: &ConvertJob<'_>, level: &str, args: &mut Vec<String>) {
    push(args, "-profile:v");
    push(args, "high");
    // Level is computed from both of H.264's limits — per frame and per second.
    // A level that is too low is something a strict decoder may reject outright;
    // too high is always safe.
    push(args, "-level");
    push(args, level);
    push(args, "-pix_fmt");
    push(args, "yuv420p");
    // One keyframe per second at whatever the frame rate is. A constant here
    // would be wrong: 48 was written for 48 fps material and meant "once a
    // second", but on 24 fps it means once every two.
    push(args, "-g");
    push(args, &job.plan.gop.to_string());
    push(args, "-keyint_min");
    push(args, &job.plan.gop.to_string());
}

fn audio_args(job: &ConvertJob<'_>, args: &mut Vec<String>) {
    match &job.plan.audio {
        AudioAction::Copy => {
            push(args, "-c:a");
            push(args, "copy");
        }
        AudioAction::Reencode {
            bitrate_kbps,
            resample_fix,
            ..
        } => {
            push(args, "-c:a");
            push(args, "aac");
            push(args, "-b:a");
            push(args, &format!("{bitrate_kbps}k"));
            push(args, "-ar");
            push(args, "48000");
            push(args, "-ac");
            push(args, "2");
            if *resample_fix {
                // FR-024. AAC records its priming samples through an edit list,
                // and the VRChat player ignores edit lists — so without this the
                // audio drifts away from the picture. Nothing about the file
                // looks wrong; you only hear it.
                push(args, "-af");
                push(args, "aresample=async=1:first_pts=0");
            }
        }
    }
}

// ---------- running it ----------

/// Encode the file.
///
/// Progress is reported through `ctx`; cancellation kills the whole process tree
/// and removes the half-written output, because a leftover file that looks like
/// a result is worse than no file at all.
pub async fn run(job: &ConvertJob<'_>, ctx: &TaskContext) -> Result<()> {
    let program = ffmpeg::locate("ffmpeg")?;
    let args = build_args(job);

    let mut child = ManagedProcess::spawn(&program.to_string_lossy(), &args)
        .map_err(|e| ConvertError::Spawn(e.to_string()))?;

    let (stdout, stderr) = child.take_output();
    let duration_us = (job.source.duration_s * 1_000_000.0).max(1.0);

    // FFmpeg's own complaints go to stderr and are the only useful thing to show
    // when it refuses. Collected in the background so a full pipe buffer cannot
    // deadlock the encoder — which it will, on any file that produces warnings.
    let complaints = tokio::spawn(async move {
        use tokio::io::AsyncBufReadExt;
        let mut tail: Vec<String> = Vec::new();
        if let Some(err) = stderr {
            let mut lines = tokio::io::BufReader::new(err).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tail.push(line);
                // Only the last few lines matter: the reason is at the end, and
                // keeping all of them turns a broken file into a memory problem.
                if tail.len() > 20 {
                    tail.remove(0);
                }
            }
        }
        tail
    });

    if let Some(out) = stdout {
        use tokio::io::AsyncBufReadExt;
        let mut lines = tokio::io::BufReader::new(out).lines();

        while let Ok(Some(line)) = lines.next_line().await {
            if ctx.is_cancelled() {
                let _ = child.kill_tree().await;
                cleanup(job.out_path);
                return Err(ConvertError::Cancelled);
            }
            if let Some(done_us) = progress_position(&line) {
                let fraction = (done_us as f64 / duration_us).clamp(0.0, 1.0);
                ctx.report(fraction, "converting");
                ctx.save_progress(fraction);
            }
        }
    }

    let status = child
        .wait()
        .await
        .map_err(|e| ConvertError::Spawn(e.to_string()))?;

    if ctx.is_cancelled() {
        cleanup(job.out_path);
        return Err(ConvertError::Cancelled);
    }

    if !status.success() {
        cleanup(job.out_path);
        let tail = complaints.await.unwrap_or_default();
        return Err(ConvertError::Failed(explain(&tail, status)));
    }

    Ok(())
}

/// Position within the file, in microseconds, from a `-progress` line.
///
/// FFmpeg prints `out_time_us=N`, and on some builds `out_time_ms` holding the
/// very same microseconds — the name is a long-standing mistake upstream, not a
/// different unit. Reading it as milliseconds puts progress a thousand times off.
pub fn progress_position(line: &str) -> Option<u64> {
    let (key, value) = line.split_once('=')?;
    match key.trim() {
        "out_time_us" | "out_time_ms" => value.trim().parse::<u64>().ok(),
        _ => None,
    }
}

/// Turn FFmpeg's last words into something worth showing.
fn explain(tail: &[String], status: std::process::ExitStatus) -> String {
    let last = tail
        .iter()
        .rev()
        .find(|l| !l.trim().is_empty())
        .cloned()
        .unwrap_or_default();

    if last.is_empty() {
        format!("the encoder exited with {status} and said nothing")
    } else {
        last
    }
}

/// Remove a half-written result.
///
/// A leftover file is worse than none: it has the right name and a plausible
/// size, and the next step would happily upload it.
fn cleanup(out_path: &str) {
    if Path::new(out_path).exists() {
        let _ = std::fs::remove_file(out_path);
    }
}
