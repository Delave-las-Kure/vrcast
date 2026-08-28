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
use crate::tasks::process::ManagedProcess;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum ValidateError {
    #[error(transparent)]
    Ffmpeg(#[from] ffmpeg::FfmpegError),

    #[error("could not start the decoder: {0}")]
    Spawn(String),
}

pub type Result<T> = std::result::Result<T, ValidateError>;

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
pub async fn validate(path: &Path) -> Result<Validation> {
    let program = ffmpeg::locate("ffmpeg")?;

    // Both streams are decoded — audio included. The known workaround for the
    // muxer trap drops audio entirely, which would leave a silent-but-broken
    // track undetected; classifying the complaints keeps audio under test.
    let args: Vec<String> = [
        "-hide_banner",
        "-nostdin",
        "-v",
        "error",
        "-i",
        &path.to_string_lossy(),
        "-f",
        "null",
        "-",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect();

    let mut child = ManagedProcess::spawn(&program.to_string_lossy(), &args)
        .map_err(|e| ValidateError::Spawn(e.to_string()))?;

    let (_, stderr) = child.take_output();
    let mut complaints = String::new();
    if let Some(err) = stderr {
        use tokio::io::AsyncReadExt;
        let mut reader = tokio::io::BufReader::new(err);
        let _ = reader.read_to_string(&mut complaints).await;
    }
    let status = child.wait().await;

    let mut verdict = classify(&complaints);

    // **FFmpeg's own verdict, which used to be thrown away on the line above.** It can only
    // condemn here, never excuse: a zero exit is what a file full of real decoder complaints
    // gives too, because the decoder reports the damage and carries on to the end. Letting a
    // zero acquit would undo the whole of `classify`. A non-zero one is the other way round
    // — the decode did not finish — and until now a refusal that printed nothing at
    // `-v error` passed for a clean file.
    if let Ok(status) = &status {
        if !status.success() {
            verdict.problems.push(format!(
                "the decoder stopped before the end of the file ({status})"
            ));
            verdict.ok = false;
        }
    }

    Ok(verdict)
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
