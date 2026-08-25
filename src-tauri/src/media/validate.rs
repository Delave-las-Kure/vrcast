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
    let _ = child.wait().await;

    Ok(classify(&complaints))
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
        if is_muxer_timestamp_noise(line) {
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

impl Validation {
    /// A sentence for the person waiting.
    pub fn summary(&self) -> String {
        if self.ok && self.ignored.is_empty() {
            return String::from("Файл декодируется целиком без единой жалобы.");
        }
        if self.ok {
            return format!(
                "Файл декодируется целиком. Замечаний от упаковщика: {} — они про метки \
                 времени в исходнике и на воспроизведение не влияют.",
                self.ignored.len()
            );
        }
        format!(
            "Файл не проходит проверку воспроизведения: жалоб от декодера — {}. \
             Заливать его нельзя: у зрителя он развалится там же, где развалился здесь.",
            self.problems.len()
        )
    }
}
