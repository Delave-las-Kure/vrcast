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
