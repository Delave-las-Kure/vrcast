//! T230 — where to take the quality measurement (FR-141).
//!
//! **This is not the ladder.** The ladder's multipliers walk *down* from the anchor, because
//! a ladder is what viewers get. The grid's multipliers reach *past* the anchor in both
//! directions, because a grid is where an optimum is looked for, and an optimum found at the
//! edge of the search is not an optimum — it is the edge.
//!
//! The probe's anchor only has to land within a factor of about two; the measurement itself
//! finds the real point. What matters is that the real point falls **inside** the grid.
//!
//! Ported from `measure-ladder.sh` without changing the arithmetic (constitution VI).

use serde::{Deserialize, Serialize};

use super::ladder::{SourceFacts, TARGET_DENSITY, UPSCALE_HEADROOM};

/// The grid's bitrates, as multiples of the probe's anchor.
///
/// **Deliberately not the ladder's `[1.0, 0.55, 0.3, 0.17]`.** Half again above the anchor
/// and down to below a fifth of it: the material that saturates early is caught by the low
/// end, and the dense live action that the anchor underestimates is caught by the high one.
pub const GRID_MULTIPLIERS: [f64; 5] = [1.5, 1.0, 0.6, 0.35, 0.18];

/// The ratio between neighbouring heights in the grid.
pub const HEIGHT_STEP: f64 = 0.8;

/// How far down the grid's heights go, as a share of the source's own.
///
/// Below about a third of the frame there is nothing a ladder would offer: the picture is
/// no longer the film, and the bitrate saved is a rounding error against the top rungs.
pub const LOWEST_SHARE: f64 = 0.28;

/// One point of the grid: a bitrate tried at a height.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cell {
    pub bitrate_mbps: u64,
    pub height: u32,
}

/// The bitrates the grid is measured at, lowest first.
///
/// **Halves go to the even number.** The script rounds in Python, whose `round` breaks a tie
/// towards even, and the same tie-break decides real points: an anchor of 3 gives a top of
/// 4 here, not 5. Rust's own `round` would give 5, and 5 is a bitrate nothing was measured
/// at. Same reason as in [`super::ladder`].
pub fn grid_bitrates_mbps(anchor_mbps: u64) -> Vec<u64> {
    let anchor = anchor_mbps as f64;
    let mut bitrates: Vec<u64> = GRID_MULTIPLIERS
        .iter()
        .map(|m| ((anchor * m).round_ties_even() as u64).max(1))
        .collect();
    bitrates.sort_unstable();
    bitrates.dedup();
    bitrates
}

/// The heights the grid may use, tallest first.
///
/// The top is the source's own height, or what an upscale left of it: above 1.6× the height
/// the material really has there is no detail to find, only weight. Measured on Mars Express
/// (1080 upscaled to 2160), where the optimum sat at 1728 = 1.6× and held there at 4, 8 and
/// 14 Mbit/s while everything measured above it lost.
pub fn grid_heights(source: &SourceFacts) -> Vec<u32> {
    let mut top = match source.native_height {
        Some(native) if native > 0 => source.height.min((native as f64 * UPSCALE_HEADROOM) as u32),
        _ => source.height,
    };
    top -= top % 2;

    let floor = source.height as f64 * LOWEST_SHARE;
    let mut heights = Vec::new();
    let mut h = top as f64;
    while h >= floor {
        let whole = h as u32;
        heights.push(whole - whole % 2);
        h *= HEIGHT_STEP;
    }
    heights
}

/// Every point the measurement will take.
///
/// **Three heights per bitrate, not one.** The density guess says roughly where the optimum
/// should be; its neighbours above and below are what catch it when the guess is wrong,
/// which on real material it regularly is — that is the whole reason a measurement exists
/// rather than a formula.
pub fn grid(source: &SourceFacts, anchor_mbps: u64) -> Vec<Cell> {
    let heights = grid_heights(source);
    if heights.is_empty() {
        return Vec::new();
    }

    let mut cells: Vec<Cell> = Vec::new();
    for bitrate_mbps in grid_bitrates_mbps(anchor_mbps) {
        let density = bitrate_mbps as f64 * 1e6
            / (source.width as f64 * source.height as f64 * source.fps.max(1) as f64);
        let guess = (source.height as f64) * (density / TARGET_DENSITY).sqrt();
        let guess = guess.min(source.height as f64);

        // The nearest height, and on a tie the taller one — `min` keeps the first, and the
        // heights are tallest first.
        let mut nearest = 0usize;
        let mut best = f64::INFINITY;
        for (i, h) in heights.iter().enumerate() {
            let away = (*h as f64 - guess).abs();
            if away < best {
                best = away;
                nearest = i;
            }
        }

        for k in [nearest.wrapping_sub(1), nearest, nearest + 1] {
            if let Some(height) = heights.get(k) {
                let cell = Cell {
                    bitrate_mbps,
                    height: *height,
                };
                if !cells.contains(&cell) {
                    cells.push(cell);
                }
            }
        }
    }
    cells
}
