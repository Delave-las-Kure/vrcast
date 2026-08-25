//! T116 — examining a source: what file we have been given (FR-020, FR-021).
//!
//! `ffprobe` from the same bundled build as everything else is asked. There is no
//! container parsing of our own here and there will not be: there are dozens of them,
//! each with its own oddities, and writing it better than it has already been written
//! is not going to happen.
//!
//! Parsing the answer is deliberately separated from running the program. `ffprobe`'s
//! answer is data, and every subtlety of reading it (numbers as strings, `und` in
//! place of an absent language, a frame rate as a fraction) is checked by a test
//! against a recorded answer, with no file on disk and no program at all.

use super::ffmpeg;
use crate::domain::source::{AudioTrack, SourceFile};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    #[error(transparent)]
    Ffmpeg(#[from] ffmpeg::FfmpegError),

    #[error("file could not be parsed: {0}")]
    Unreadable(String),

    #[error("the file has no video")]
    NoVideo,
}

pub type Result<T> = std::result::Result<T, ProbeError>;

/// Examine a source file.
pub async fn probe(path: &Path) -> Result<SourceFile> {
    let ffprobe = ffmpeg::locate("ffprobe")?;

    let out = tokio::process::Command::new(&ffprobe)
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path)
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|e| ProbeError::Unreadable(format!("{}: {e}", path.display())))?;

    if !out.status.success() {
        // The word `ffprobe` tells a person nothing, but its complaint does: "moov
        // atom not found", "Invalid data found". Passed on as it stands.
        let complaint = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        return Err(ProbeError::Unreadable(if complaint.is_empty() {
            format!("{} — parsing failed with no explanation", path.display())
        } else {
            complaint
        }));
    }

    let text = String::from_utf8_lossy(&out.stdout);
    parse(&text, &path.display().to_string())
}

// ---------- parsing the answer ----------

/// `ffprobe`'s answer in the form it arrives.
///
/// Numbers are strings here not by oversight: that is how `ffprobe` prints them.
/// Trying to read them as numbers means a parse failure on the very first file.
#[derive(Debug, Deserialize)]
struct Probed {
    #[serde(default)]
    streams: Vec<Stream>,
    #[serde(default)]
    format: Format,
}

#[derive(Debug, Default, Deserialize)]
struct Format {
    #[serde(default)]
    duration: Option<String>,
    #[serde(default)]
    size: Option<String>,
    #[serde(default)]
    bit_rate: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Stream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    pix_fmt: Option<String>,
    color_transfer: Option<String>,
    r_frame_rate: Option<String>,
    avg_frame_rate: Option<String>,
    bit_rate: Option<String>,
    channels: Option<u16>,
    #[serde(default)]
    tags: Tags,
    #[serde(default)]
    disposition: Disposition,
}

#[derive(Debug, Default, Deserialize)]
struct Tags {
    language: Option<String>,
    title: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Disposition {
    #[serde(default)]
    default: u8,
}

/// Read `ffprobe`'s answer.
pub fn parse(json: &str, path: &str) -> Result<SourceFile> {
    let probed: Probed = serde_json::from_str(json).map_err(|e| {
        ProbeError::Unreadable(format!("the prober's answer could not be read: {e}"))
    })?;

    let video = probed
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("video"))
        .ok_or(ProbeError::NoVideo)?;

    let audio_tracks = probed
        .streams
        .iter()
        .filter(|s| s.codec_type.as_deref() == Some("audio"))
        .enumerate()
        .map(|(index, s)| AudioTrack {
            // The index among the AUDIO streams, not among all of them: that is what
            // ffmpeg understands in `-map 0:a:<N>`. Taking the overall stream index
            // means picking the wrong track on any file where audio is not first.
            index,
            codec: s.codec_name.clone().unwrap_or_default(),
            channels: s.channels.unwrap_or(0),
            bitrate_bps: number(&s.bit_rate),
            language: language(&s.tags.language),
            title: not_empty(&s.tags.title),
            is_default: s.disposition.default == 1,
        })
        .collect();

    Ok(SourceFile {
        path: path.to_owned(),
        size_bytes: number(&probed.format.size).unwrap_or(0),
        duration_s: probed
            .format
            .duration
            .as_deref()
            .and_then(|d| d.parse::<f64>().ok())
            .unwrap_or(0.0),
        width: video.width.unwrap_or(0),
        height: video.height.unwrap_or(0),
        fps: fps(video),
        bitrate_bps: number(&video.bit_rate)
            .or_else(|| number(&probed.format.bit_rate))
            .unwrap_or(0),
        peak_bps: None,
        video_codec: video.codec_name.clone().unwrap_or_default(),
        pix_fmt: video.pix_fmt.clone().unwrap_or_default(),
        color_transfer: not_empty(&video.color_transfer),
        audio_tracks,
    })
}

/// The frame rate, rounded **up**.
///
/// It arrives as a fraction: `24/1`, `24000/1001`. Rounding down would turn 47.952
/// into 47 and understate the compatibility level — and a strict decoder is entitled
/// to refuse a file whose level is understated.
fn fps(video: &Stream) -> u32 {
    let source = video
        .r_frame_rate
        .as_deref()
        .filter(|s| *s != "0/0")
        .or(video.avg_frame_rate.as_deref())
        .unwrap_or("");

    let (num, den) = match source.split_once('/') {
        Some((n, d)) => (n.parse::<u64>().ok(), d.parse::<u64>().ok()),
        None => (source.parse::<u64>().ok(), Some(1)),
    };

    match (num, den) {
        (Some(n), Some(d)) if d > 0 && n > 0 => n.div_ceil(d) as u32,
        // Zero frames a second does not happen. Thirty is a harmless guess: it
        // overstates the compatibility level, and overstating is always safe.
        _ => 30,
    }
}

fn number(s: &Option<String>) -> Option<u64> {
    s.as_deref().and_then(|v| v.parse::<u64>().ok())
}

fn not_empty(s: &Option<String>) -> Option<String> {
    s.as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
}

/// The track's language.
///
/// `und` means "not specified" rather than being the name of a language. Showing it to
/// a person means offering them a choice between "und" and "und"; the ordinal number
/// is more use in that case (the boundary case of the specification for FR-020).
fn language(raw: &Option<String>) -> Option<String> {
    not_empty(raw).filter(|v| !v.eq_ignore_ascii_case("und"))
}
