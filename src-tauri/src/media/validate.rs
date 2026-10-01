//! T120 — proving the finished file actually plays (FR-027).
//!
//! The file is decoded from end to end. Nothing cheaper is enough: a broken
//! encode opens fine, reports the right duration and the right frame count, and
//! falls apart only where someone is watching it.
//!
//! **Cost.** A full decode of a 20 GB file takes around eight minutes. That is
//! not a reason to skip it — an unplayable file reaching viewers costs far more —
//! but it does belong in whatever estimate is shown to the person waiting.
//!
//! ## The trap this module exists to avoid
//!
//! The obvious command, `ffmpeg -v error -i out.mp4 -f null -`, produces false
//! failures on a large class of perfectly good files:
//!
//! ```text
//! [null @ 0x...] Application provided invalid, non monotonically increasing dts to muxer
//! ```
//!
//! That complaint comes from the **muxer**, not the decoder. The null muxer
//! insists on monotonic timestamps; plenty of real sources have duplicate DTS and
//! decode perfectly. Every source from one supplier used by this project has that
//! defect, and it survives being re-encoded — so treating the message as failure
//! would reject an entire library of working files.
//!
//! A decoder complaint carries the decoder's name instead — `[h264 @ ...]`,
//! `[aac @ ...]` — and that is the class the rule was written for: "Invalid NAL
//! unit size" from an encoder that was orphaned mid-write.
//!
//! So the output is classified by who is complaining rather than whether anything
//! complained at all.
//!
//! ## The second family, found the same way
//!
//! ```text
//! [in#0 @ 0x...] Referenced QT chapter track not found
//! ```
//!
//! The container reader, about a track reference — not the decoder, about data. FFmpeg
//! prints it and exits **0**. This project made those files itself: chapters were copied
//! into the MP4 while the chapter track was not (fixed in `convert.rs`), and then this
//! module called the result broken. The cause is gone; the rule stays for the files already
//! made, which hold hours of work and play perfectly.
//!
//! **Both rules are shaped the same way and neither is a wildcard**: a known component
//! saying a known thing. Anything else still counts against the file — an unrecognised
//! complaint is a complaint (`unknown_complaints_are_treated_as_problems`).

use super::ffmpeg;
use crate::tasks::engine::TaskContext;
use crate::tasks::process::ManagedProcess;
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Debug, thiserror::Error)]
pub enum ValidateError {
    #[error(transparent)]
    Ffmpeg(#[from] ffmpeg::FfmpegError),

    #[error("could not start the decoder: {0}")]
    Spawn(String),

    /// The task was cancelled while the file was being decoded (T663). The decoder's whole
    /// process tree was stopped and waited for before this came back.
    #[error("the playback check was cancelled")]
    Cancelled,

    /// The decoder went this many seconds without decoding a single new frame, while nobody
    /// had paused it (T663). It was stopped; nothing was learned about the file.
    #[error("the decoder made no progress for {0} s and was stopped")]
    Stalled(u64),
}

pub type Result<T> = std::result::Result<T, ValidateError>;

/// How long the decoder may go without a single new frame before it is taken for hung (T663).
///
/// **A limit on silence, not on length.** A whole-film decode takes minutes to tens of
/// minutes, and any fixed total would be wrong for some film; what a hung decoder has that a
/// slow one does not is a position that stops moving. Three minutes is far past anything a
/// working decoder does between two frames — even an 8K HEVC in software moves every few
/// seconds — and short enough that a hung one does not hold the compute lane for the night.
/// Time spent paused is not silence and is not counted.
pub const STALL_LIMIT: Duration = Duration::from_secs(180);

/// How often to look at cancel and pause while the decoder is quiet — the same fifth of a
/// second `media::convert` uses.
const POLL: Duration = Duration::from_millis(200);

/// Who watches one decode.
///
/// Without a task (`convert_validate`, asked from a screen) only the stall limit applies;
/// inside one, the task's cancel stops it, its pause freezes it, and how far it got is
/// reported under `STAGE_VALIDATING`.
pub struct Watch<'a> {
    pub ctx: Option<&'a TaskContext>,
    /// The film's length, for the share decoded. Zero or less: no share is reported.
    pub duration_s: f64,
    pub stall: Duration,
}

impl Watch<'_> {
    /// Nobody to answer to but the stall limit.
    pub fn unattended() -> Watch<'static> {
        Watch {
            ctx: None,
            duration_s: 0.0,
            stall: STALL_LIMIT,
        }
    }
}

/// What a watched decoder left behind.
pub struct Decoded {
    /// Everything it said on stderr.
    pub complaints: String,
    /// Its exit, when it could be read.
    pub status: Option<std::process::ExitStatus>,
}

/// What the decode showed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Validation {
    /// Whether the file may be offered for upload.
    pub ok: bool,
    /// Decoder complaints, in the decoder's own words. Empty when clean.
    ///
    /// Kept verbatim: "Invalid NAL unit size" is cryptic but searchable, and
    /// "the file is broken" is neither.
    pub problems: Vec<String>,
    /// Muxer complaints that were deliberately not counted against the file.
    ///
    /// Shown rather than hidden: when someone later wonders why a file with
    /// warnings was accepted, the answer should be in front of them.
    pub ignored: Vec<String>,
}

/// Decode the whole file and report what happened.
///
/// Unattended: for the check a screen asks for, outside any task. Still bounded by
/// [`STALL_LIMIT`] — a hung decoder must not hold a call open for ever either.
pub async fn validate(path: &Path) -> Result<Validation> {
    validate_watched(path, &Watch::unattended()).await
}

/// The same, inside a task (T663): cancel stops the decoder's whole tree and waits for it,
/// pause freezes it in place, and the share decoded goes out as the task's progress under
/// `STAGE_VALIDATING` — a stage of its own, from zero, rather than a bar parked at 98 %.
///
/// **Pause, decided (T663).** The decoder is frozen where it is, exactly as the encoder is
/// during encoding (FR-083a), and carries on from the same frame on resume. Starting the
/// check over would throw away up to minutes of decoding; refusing to pause would make this
/// one phase of a preparation behave unlike all the others. The time spent frozen is not
/// counted against [`STALL_LIMIT`].
pub async fn validate_in_task(
    path: &Path,
    ctx: &TaskContext,
    duration_s: f64,
) -> Result<Validation> {
    validate_watched(
        path,
        &Watch {
            ctx: Some(ctx),
            duration_s,
            stall: STALL_LIMIT,
        },
    )
    .await
}

/// Decode the whole file under `watch`.
pub async fn validate_watched(path: &Path, watch: &Watch<'_>) -> Result<Validation> {
    let program = ffmpeg::locate("ffmpeg")?;

    // Both streams are decoded — audio included. The known workaround for the
    // muxer trap drops audio entirely, which would leave a silent-but-broken
    // track undetected; classifying the complaints keeps audio under test.
    //
    // `-progress pipe:1` (T663): the decoder's position on stdout, which is what tells a slow
    // decode from a hung one, and what the task's progress is made of. The complaints stay
    // on stderr, untouched, for `classify`.
    let args: Vec<String> = [
        "-hide_banner",
        "-nostdin",
        "-v",
        "error",
        "-progress",
        "pipe:1",
        "-nostats",
        "-i",
        &path.to_string_lossy(),
        "-f",
        "null",
        "-",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect();

    let decoded = watch_decoder(&program.to_string_lossy(), &args, watch).await?;
    let mut verdict = classify(&decoded.complaints);

    // **FFmpeg's own verdict, which used to be thrown away on the line above.** It can only
    // condemn here, never excuse: a zero exit is what a file full of real decoder complaints
    // gives too, because the decoder reports the damage and carries on to the end. Letting a
    // zero acquit would undo the whole of `classify`. A non-zero one is the other way round
    // — the decode did not finish — and until now a refusal that printed nothing at
    // `-v error` passed for a clean file.
    if let Some(status) = &decoded.status {
        if !status.success() {
            verdict.problems.push(format!(
                "the decoder stopped before the end of the file ({status})"
            ));
            verdict.ok = false;
        }
    }

    Ok(verdict)
}

/// Run a decoder to its end under `watch` (T663).
///
/// Public, and taking the program rather than a file, so that what matters here — a cancel
/// that stops it, a pause that freezes it, a silence that ends it — can be checked against
/// a program that is deliberately mute, which no real file can be made to produce honestly.
///
/// The decoder is expected to write FFmpeg's `-progress` lines on stdout; a program that
/// writes none is, to this, a decoder that never moves.
pub async fn watch_decoder(program: &str, args: &[String], watch: &Watch<'_>) -> Result<Decoded> {
    let mut child =
        ManagedProcess::spawn(program, args).map_err(|e| ValidateError::Spawn(e.to_string()))?;

    let (stdout, stderr) = child.take_output();

    // Collected in the background, as the encoder's are: a full stderr pipe would stop the
    // decoder dead, and a file with damage in every frame fills one quickly.
    let complaints = tokio::spawn(async move {
        let mut text = String::new();
        if let Some(err) = stderr {
            use tokio::io::AsyncReadExt;
            let mut reader = tokio::io::BufReader::new(err);
            let _ = reader.read_to_string(&mut text).await;
        }
        text
    });

    let duration_us = watch.duration_s * 1_000_000.0;
    let mut position: Option<u64> = None;
    let mut moved_at = Instant::now();

    if let Some(out) = stdout {
        use tokio::io::AsyncBufReadExt;
        let mut lines = tokio::io::BufReader::new(out).lines();

        loop {
            // Never waiting on the next line alone: a hung decoder prints nothing, or prints
            // the same position for ever, and neither would ever let the checks below run.
            let line = tokio::select! {
                got = lines.next_line() => match got {
                    Ok(Some(line)) => Some(line),
                    _ => break,
                },
                _ = tokio::time::sleep(POLL) => None,
            };

            if let Some(ctx) = watch.ctx {
                if ctx.is_cancelled() {
                    let _ = child.kill_tree().await;
                    complaints.abort();
                    return Err(ValidateError::Cancelled);
                }
                freeze_to_match(&mut child, ctx);
                if child.is_suspended() {
                    // Paused is not hung: the clock starts again when it is let go.
                    moved_at = Instant::now();
                    continue;
                }
            }

            if let Some(at) = line.as_deref().and_then(super::convert::progress_position) {
                if position.is_none_or(|was| at > was) {
                    position = Some(at);
                    moved_at = Instant::now();
                    if let (Some(ctx), true) = (watch.ctx, duration_us > 0.0) {
                        let share = (at as f64 / duration_us).clamp(0.0, 1.0);
                        ctx.report(share, crate::domain::wording::DetailCode::StageValidating);
                    }
                }
            }

            if moved_at.elapsed() >= watch.stall {
                let _ = child.kill_tree().await;
                complaints.abort();
                return Err(ValidateError::Stalled(watch.stall.as_secs()));
            }
        }

        // A frozen process is never reaped: waiting for it below would hang.
        let _ = child.resume();
    }

    // Its output is closed, so it is ending — but "ending" is not waited for blindly either:
    // a decoder that closed its output and then hung would otherwise hold this for ever, and
    // a cancel that came in the last moment is honoured rather than reported as a check.
    let waited_from = Instant::now();
    let status = loop {
        match tokio::time::timeout(POLL, child.wait()).await {
            Ok(status) => break status.ok(),
            Err(_) => {
                if watch.ctx.is_some_and(|c| c.is_cancelled()) {
                    let _ = child.kill_tree().await;
                    complaints.abort();
                    return Err(ValidateError::Cancelled);
                }
                if waited_from.elapsed() >= watch.stall {
                    let _ = child.kill_tree().await;
                    complaints.abort();
                    return Err(ValidateError::Stalled(watch.stall.as_secs()));
                }
            }
        }
    };
    if watch.ctx.is_some_and(|c| c.is_cancelled()) {
        complaints.abort();
        return Err(ValidateError::Cancelled);
    }
    let complaints = complaints.await.unwrap_or_default();
    Ok(Decoded { complaints, status })
}

/// Freeze or release the decoder to match the task's pause.
fn freeze_to_match(child: &mut ManagedProcess, ctx: &TaskContext) {
    let should = ctx.is_paused();
    if should == child.is_suspended() {
        return;
    }
    let outcome = if should {
        child.suspend()
    } else {
        child.resume()
    };
    if let Err(e) = outcome {
        tracing::warn!(error = %e, paused = should, "could not change the decoder's state");
    }
}

/// Sort decoder complaints from muxer noise.
///
/// Pure on purpose: this is the whole decision, and it must be testable against
/// the exact messages seen in practice rather than against a live decode.
pub fn classify(stderr: &str) -> Validation {
    let mut problems = Vec::new();
    let mut ignored = Vec::new();

    for line in stderr.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if is_muxer_timestamp_noise(line) || is_container_note(line) {
            ignored.push(line.to_owned());
        } else {
            problems.push(line.to_owned());
        }
    }

    Validation {
        ok: problems.is_empty(),
        problems,
        ignored,
    }
}

/// Is this the container reader noting a reference it could not follow?
///
/// Held to the same two-part shape as the rule below, and for the same reason: the wording
/// alone would excuse a decoder saying something similar, and the component alone would
/// excuse everything the container reader ever says — including the complaints that do mean
/// a file is unusable.
fn is_container_note(line: &str) -> bool {
    // `[in#0 @ ...]` when the file is an input, `[mov,mp4,m4a,3gp,3g2,mj2 @ ...]` when it is
    // being examined. Both were seen on 2026-08-28 from the same file.
    let from_the_container = line.starts_with("[in#") || line.starts_with("[mov,");
    let about_a_chapter_reference = line.contains("Referenced QT chapter track not found");
    from_the_container && about_a_chapter_reference
}

/// Is this the muxer complaining about timestamps rather than the decoder about data?
fn is_muxer_timestamp_noise(line: &str) -> bool {
    // Two conditions, both required. The component name alone is not enough: the
    // null muxer could in principle report something that does matter. And the
    // wording alone is not enough either — a decoder saying something similar
    // would be a genuine problem.
    let from_null_muxer = line.starts_with("[null @") || line.starts_with("[out#");
    let about_timestamps = line.contains("non monotonically increasing dts")
        || line.contains("Non-monotonic DTS")
        || line.contains("non-monotonic DTS");
    from_null_muxer && about_timestamps
}
