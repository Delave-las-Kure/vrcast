//! T191 — measuring a source: what it averages, where it peaks, and how hard the peaks
//! are (FR-040, and later FR-073).
//!
//! **Why the peaks matter separately from the average.** A viewer's connection has to hold
//! the peak, not the average: a film that averages 8 Mbit/s and reaches 40 in one battle
//! scene freezes for everyone whose line is under 40 when that scene arrives. The average
//! alone would say the film is comfortable.
//!
//! The packets are read rather than reasoned about. The container's declared bitrate is one
//! number for the whole file and says nothing about where it is heavy.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::ffmpeg;

/// What a second of the film weighed.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Moment {
    /// How far into the film, in seconds.
    pub at_s: f64,
    pub bitrate_bps: u64,
}

/// What the measurement found.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Measured {
    pub average_bps: u64,
    pub peak_bps: u64,
    /// The heaviest moments, worst first — where to look, not just how bad.
    ///
    /// Kept short: a person wants somewhere to jump to, not a list of every second of a
    /// two-hour film.
    pub worst: Vec<Moment>,
    /// How many seconds were measured.
    pub seconds: usize,
}

/// How many heavy moments are worth naming.
const WORST_KEPT: usize = 5;

/// Measure a file's video stream.
///
/// One second is the window, deliberately: it is the unit a connection is judged in, and
/// the unit `-maxrate` is expressed in. A shorter window would find spikes that no buffer
/// ever notices.
pub async fn measure(path: &Path) -> Result<Measured, ffmpeg::FfmpegError> {
    let ffprobe = ffmpeg::locate("ffprobe")?;
    let output = tokio::process::Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "packet=pts_time,dts_time,size",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .await
        .map_err(|e| ffmpeg::FfmpegError::NotRunnable(e.to_string()))?;

    if !output.status.success() {
        return Err(ffmpeg::FfmpegError::Unexpected(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    Ok(from_packets(&String::from_utf8_lossy(&output.stdout)))
}

/// Add the packets up into seconds.
///
/// Kept apart from the running of ffprobe so that it can be checked without one — the
/// arithmetic is what goes wrong, not the process.
pub fn from_packets(csv: &str) -> Measured {
    let mut seconds: std::collections::BTreeMap<u64, u64> = std::collections::BTreeMap::new();
    let mut total_bytes: u64 = 0;

    for line in csv.lines() {
        let mut fields = line.split(',');
        let pts = fields.next().unwrap_or("").trim();
        let dts = fields.next().unwrap_or("").trim();
        let size = fields.next().unwrap_or("").trim();

        // The presentation time when there is one, and the decode time when there is not.
        // Some containers leave one of them out, and a packet without a time cannot be put
        // in a second — but its bytes still belong to the average.
        let Ok(bytes) = size.parse::<u64>() else {
            continue;
        };
        total_bytes += bytes;

        let time = pts.parse::<f64>().or_else(|_| dts.parse::<f64>());
        if let Ok(time) = time {
            if time.is_finite() && time >= 0.0 {
                *seconds.entry(time as u64).or_insert(0) += bytes;
            }
        }
    }

    let counted = seconds.len();
    let mut moments: Vec<Moment> = seconds
        .into_iter()
        .map(|(at, bytes)| Moment {
            at_s: at as f64,
            bitrate_bps: bytes * 8,
        })
        .collect();

    let peak_bps = moments.iter().map(|m| m.bitrate_bps).max().unwrap_or(0);
    let average_bps = if counted > 0 {
        total_bytes * 8 / counted as u64
    } else {
        0
    };

    // Heaviest first: a person opening this wants somewhere to jump to.
    moments.sort_by_key(|m| std::cmp::Reverse(m.bitrate_bps));
    moments.truncate(WORST_KEPT);

    Measured {
        average_bps,
        peak_bps,
        worst: moments,
        seconds: counted,
    }
}

/// Read a file's per-second weights.
///
/// The same ffprobe call as [`measure`], read differently: the chunk picker wants the whole
/// series, not the worst few moments of it.
pub async fn seconds_of(path: &Path) -> Result<Vec<u64>, ffmpeg::FfmpegError> {
    let ffprobe = ffmpeg::locate("ffprobe")?;
    let output = tokio::process::Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "packet=pts_time,dts_time,size",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .await
        .map_err(|e| ffmpeg::FfmpegError::NotRunnable(e.to_string()))?;

    if !output.status.success() {
        return Err(ffmpeg::FfmpegError::Unexpected(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    Ok(seconds_from_packets(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

/// What each second of the film weighed, from the first to the last, with silent seconds
/// present as zero.
///
/// The chunk picker needs the whole series rather than the worst few moments, and this is
/// the same bucketing rather than a second copy of it: two copies of one reading have
/// already once given this project different answers to the same question.
pub fn seconds_from_packets(csv: &str) -> Vec<u64> {
    let mut seconds: std::collections::BTreeMap<u64, u64> = std::collections::BTreeMap::new();

    for line in csv.lines() {
        let mut fields = line.split(',');
        let pts = fields.next().unwrap_or("").trim();
        let dts = fields.next().unwrap_or("").trim();
        let size = fields.next().unwrap_or("").trim();

        let Ok(bytes) = size.parse::<u64>() else {
            continue;
        };
        let time = pts.parse::<f64>().or_else(|_| dts.parse::<f64>());
        if let Ok(time) = time {
            if time.is_finite() && time >= 0.0 {
                *seconds.entry(time as u64).or_insert(0) += bytes;
            }
        }
    }

    let Some(last) = seconds.keys().next_back().copied() else {
        return Vec::new();
    };
    (0..=last)
        .map(|s| seconds.get(&s).copied().unwrap_or(0))
        .collect()
}

// ---------- T315: where the peaks are, and over what window ----------

/// How long the wider window is, in seconds.
///
/// **Ten, carried over from the diagnosis skill unchanged** (principle VI). This is the
/// window that hangs a player: a one-second spike is absorbed by any buffer worth the name,
/// and a whole-file average hides everything. Ten seconds is about as long as a player's
/// buffer holds, so a ten-second stretch the viewer's line cannot carry is a stretch the
/// buffer drains through and does not refill.
pub const WIDE_WINDOW_S: usize = 10;

/// How many heavy windows are worth naming. Somewhere to jump to, not a transcript.
const WIDE_KEPT: usize = 5;

/// A window of the film, and where it is.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Window {
    /// How far into the film the window starts, in seconds.
    pub at_s: f64,
    /// How long it is. Kept on the window itself so a screen showing one-second and
    /// ten-second peaks side by side does not have to be told which is which.
    pub length_s: f64,
    pub bitrate_bps: u64,
}

/// What a file looks like from a viewer's line's point of view (FR-073).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Peaks {
    pub average_bps: u64,
    /// The middle second. Shown beside the average because the two apart tell whether the
    /// film is evenly heavy or mostly light with a few walls in it.
    pub median_bps: u64,
    /// The heaviest single second, and where.
    pub one_second: Option<Window>,
    /// The heaviest ten seconds, and where. **This is the number to compare a viewer's line
    /// against**, and the gap between it and the average is what says whether a re-encode
    /// with the peaks capped would help.
    pub wide: Option<Window>,
    /// The heaviest wide windows, worst first, without overlapping each other — five
    /// neighbouring windows around one battle scene are one place to look, not five.
    pub worst_wide: Vec<Window>,
    pub seconds: usize,
}

impl Peaks {
    /// How many times heavier the worst wide window is than the average.
    ///
    /// The one figure that answers "would capping the peaks help?". `None` when there is no
    /// average to compare against, which is not the same as "no, it would not".
    pub fn peak_over_average(&self) -> Option<f64> {
        let wide = self.wide?;
        (self.average_bps > 0).then(|| wide.bitrate_bps as f64 / self.average_bps as f64)
    }
}

/// Work the windows out from the per-second weights.
///
/// Takes the series [`seconds_from_packets`] produces rather than reading the file again:
/// two readings of one file have already once given this project two answers to one
/// question.
pub fn peaks(second_bytes: &[u64]) -> Peaks {
    if second_bytes.is_empty() {
        return Peaks {
            average_bps: 0,
            median_bps: 0,
            one_second: None,
            wide: None,
            worst_wide: Vec::new(),
            seconds: 0,
        };
    }

    let total: u64 = second_bytes.iter().sum();
    let average_bps = total * 8 / second_bytes.len() as u64;

    let mut sorted: Vec<u64> = second_bytes.iter().map(|b| b * 8).collect();
    sorted.sort_unstable();
    let median_bps = sorted[sorted.len() / 2];

    let one_second = second_bytes
        .iter()
        .enumerate()
        .max_by_key(|(_, bytes)| **bytes)
        .map(|(at, bytes)| Window {
            at_s: at as f64,
            length_s: 1.0,
            bitrate_bps: bytes * 8,
        });

    // Every wide window, by its start. A film shorter than the window gets one window over
    // the whole of it rather than none: "no peak" on a nine-second clip would be a lie of
    // exactly the kind this module exists to avoid.
    let span = WIDE_WINDOW_S.min(second_bytes.len());
    let mut windows: Vec<Window> = (0..=second_bytes.len().saturating_sub(span))
        .map(|start| {
            let sum: u64 = second_bytes[start..start + span].iter().sum();
            Window {
                at_s: start as f64,
                length_s: span as f64,
                bitrate_bps: sum * 8 / span as u64,
            }
        })
        .collect();

    let wide = windows.iter().copied().max_by_key(|w| w.bitrate_bps);

    // Heaviest first, then thinned so the named places do not overlap: without this, the
    // five worst windows of a film are the five windows around one explosion, and a person
    // is sent to the same place five times while a second bad stretch goes unmentioned.
    windows.sort_by_key(|w| std::cmp::Reverse(w.bitrate_bps));
    let mut worst_wide: Vec<Window> = Vec::new();
    for w in windows {
        if worst_wide.len() >= WIDE_KEPT {
            break;
        }
        if worst_wide
            .iter()
            .any(|kept| (kept.at_s - w.at_s).abs() < span as f64)
        {
            continue;
        }
        worst_wide.push(w);
    }

    Peaks {
        average_bps,
        median_bps,
        one_second,
        wide,
        worst_wide,
        seconds: second_bytes.len(),
    }
}

/// Read a file and work out where its peaks are.
pub async fn peaks_of(path: &Path) -> Result<Peaks, ffmpeg::FfmpegError> {
    Ok(peaks(&seconds_of(path).await?))
}
