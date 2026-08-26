//! T229 — the three chunks a quality measurement is taken on (FR-142).
//!
//! **Why three and not one.** A film is not uniformly hard. Measure a quiet dialogue and
//! every rung looks generous; measure the one battle scene and every rung looks starved.
//! The measurement takes a light, a middling and a heavy chunk — the tenth, fiftieth and
//! ninetieth percentile by weight — so that a single untypical scene cannot decide the
//! ladder for the whole film.
//!
//! **Why weight and not position.** Positions are a guess about where a film is hard.
//! Weight is a reading of where it actually is: the packets say so.
//!
//! Ported from `measure-ladder.sh` without changing the arithmetic (constitution VI).

/// How long one chunk is.
///
/// Ten seconds is the script's `DUR`. Long enough to hold a cut or two, short enough that
/// the whole grid — fifteen points on three chunks — stays inside half an hour.
pub const CHUNK_S: usize = 10;

/// Seconds skipped at the start, and at the end.
///
/// Opening and closing titles are still frames over a soundtrack: nearly free to encode and
/// nothing like the film. Left in, they would win the "light" percentile every time and the
/// bottom rung would be chosen on material that never appears again.
///
/// The two numbers differ because the script's do; they were arrived at by looking at real
/// files, and there is no measurement that says they should be equal.
pub const HEAD_GUARD_S: usize = 180;
pub const TAIL_GUARD_S: usize = 190;

/// The percentiles taken, in the order they are reported: light, middling, heavy.
pub const PERCENTILES: [f64; 3] = [0.10, 0.50, 0.90];

/// Where the three chunks start, in seconds from the beginning of the film.
///
/// `second_bytes` is what each second of the video stream weighed, from the first second to
/// the last, with silent seconds present as zero — see [`crate::media::measure`].
///
/// **The guards are dropped rather than honoured when they leave nothing.** A twenty-minute
/// episode has fewer than 180 + 190 seconds to spare, and a measurement of nothing at all is
/// worse than a measurement that includes the titles.
pub fn reference_chunks(second_bytes: &[u64], chunk_s: usize) -> Vec<u64> {
    let len = second_bytes.len() as i64;
    let chunk = chunk_s as i64;

    let (lo, hi) = {
        let lo = HEAD_GUARD_S as i64;
        let hi = len - chunk - TAIL_GUARD_S as i64;
        if hi <= lo {
            (0, (len - chunk).max(1))
        } else {
            (lo, hi)
        }
    };

    // Every window that fits, by what it weighs. Ties keep the earlier position, which is
    // what a stable sort over positions in order gives.
    let mut windows: Vec<(u64, u64)> = (lo..hi)
        .map(|start| {
            let from = start as usize;
            let weight: u64 = second_bytes.iter().skip(from).take(chunk_s).copied().sum();
            (weight, start as u64)
        })
        .collect();
    windows.sort_by_key(|(weight, start)| (*weight, *start));

    let n = windows.len();
    if n == 0 {
        return vec![0; PERCENTILES.len()];
    }
    PERCENTILES
        .iter()
        .map(|q| windows[((n as f64 * q) as usize).min(n - 1)].1)
        .collect()
}
