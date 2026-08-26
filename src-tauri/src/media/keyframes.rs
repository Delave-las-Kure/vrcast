//! T195 — how far apart a film's own keyframes are.
//!
//! Wanted for exactly one decision: whether a rung may be carried across without
//! re-encoding. Segments are cut at keyframes and nowhere else, so a stream whose keyframes
//! sit in the wrong places cannot be cut to line up with the others however carefully
//! everything else is done.

use std::path::Path;

use super::ffmpeg;

/// How much of the film is looked at.
///
/// **Two minutes, not the whole thing.** Keyframe spacing is a property of how the file was
/// encoded, not of what happens in it, so the opening tells the same story as the middle —
/// and reading every frame of a two-hour film costs minutes for a number that will not
/// change.
const LOOK_AT_SECONDS: u32 = 120;

/// How far apart the keyframes are, in seconds.
///
/// `None` when it cannot be worked out — a file with one keyframe and no second one to
/// measure against, or a stream ffprobe will not read. **`None` is not "probably fine"**:
/// the caller must treat it as "do not copy", because guessing that keyframes line up is
/// the one guess in a ladder that a viewer pays for.
pub async fn spacing_s(path: &Path) -> Result<Option<f64>, ffmpeg::FfmpegError> {
    let ffprobe = ffmpeg::locate("ffprobe")?;
    let output = tokio::process::Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            // Only keyframes come back at all, which is both the question and the reason
            // this is quick.
            "-skip_frame",
            "nokey",
            "-show_entries",
            "frame=pts_time",
            "-of",
            "csv=p=0",
            "-read_intervals",
        ])
        .arg(format!("%+{LOOK_AT_SECONDS}"))
        .arg(path)
        .output()
        .await
        .map_err(|e| ffmpeg::FfmpegError::NotRunnable(e.to_string()))?;

    if !output.status.success() {
        return Err(ffmpeg::FfmpegError::Unexpected(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    Ok(from_times(&String::from_utf8_lossy(&output.stdout)))
}

/// Work the spacing out from the keyframes' own times.
///
/// **The middle gap, not the average.** A film regularly has an extra keyframe at a hard
/// cut, and one such gap of a quarter of a second pulls an average down far enough to make
/// a stream look finer-grained than it is — which is the direction that ends in a copy that
/// should not have been allowed.
pub fn from_times(csv: &str) -> Option<f64> {
    let times: Vec<f64> = csv
        .lines()
        .filter_map(|line| line.trim().trim_end_matches(',').parse::<f64>().ok())
        .filter(|t| t.is_finite() && *t >= 0.0)
        .collect();
    if times.len() < 2 {
        return None;
    }

    let mut gaps: Vec<f64> = times.windows(2).map(|pair| pair[1] - pair[0]).collect();
    gaps.retain(|g| *g > 0.0);
    if gaps.is_empty() {
        return None;
    }
    gaps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(gaps[gaps.len() / 2])
}
