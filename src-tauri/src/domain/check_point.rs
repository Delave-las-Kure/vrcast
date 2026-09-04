//! T437 — the check after a loan: one cell, measured on the borrower, against the donor's.
//!
//! **Why a measurement and not more fields.** Lending compares eight fields of the container
//! (`store::measurements::differs`), and R-48 measured an episode that passes every one of
//! them and is still the wrong material: the same season, the same release, the same frame,
//! frame rate, codec, pixel format and colour transfer — and a different encode, 4.06 Mbit/s
//! against 6.33. No field lending looks at would ever separate them. One measured cell does.
//!
//! **Where the cell is, and why it is not a number.** R-48 measured six subjects on the same
//! seconds. Down the ladder the check is not merely weak, it is impossible: at 1 Mbit/s a
//! wholly different film scored 86.76, which is *inside* the band four legitimate episodes
//! made (83.18–91.03). At 2 and 3 Mbit/s the distance to alien material is smaller than the
//! spread among legitimate ones. The bands only come apart near the top — and they close
//! again above it, where every subject presses against VMAF's ceiling and the distance to
//! alien material falls faster than the spread among kin.
//!
//! | cell | spread among kin | to the nearest alien | margin |
//! |---|---|---|---|
//! | 4 Mbit/s | 1.18 | 2.32 | 2.0x |
//! | 6 Mbit/s | 0.56 | 2.41 | 4.3x |
//! | 8 Mbit/s | 0.44 | 2.18 | 5.0x |
//! | 12 Mbit/s | 0.57 | 1.69 | 3.0x |
//!
//! Six and eight are numbers about this material, and would be invented constants in the
//! code. They have a name that is not: `TARGET_VMAF` puts the ladder's top rung exactly
//! where quality has levelled off but has not yet hit the ceiling — 96.38 at 6 Mbit/s for
//! these episodes. So the cell is **the donor's top rung**, and the rule carries no number of
//! its own.

use super::measure_grid::Cell;
use super::measured_ladder::Selection;

/// How far two measurements of one cell may sit apart and still be one material.
///
/// **Both walls are measured, and the threshold sits nearer the closer one.** On the top rung
/// four legitimate episodes disagreed by at most 0.56, and a wholly different film stood 2.41
/// away at six megabits and 2.18 at eight (R-48). One point passes the first with 1.8x to
/// spare and catches the second with 2.2x.
///
/// ⚠ **The far wall rests on one subject, not two.** R-48 had a second — an episode of the
/// same season that scored 3.19 below its neighbours — and R-50 found that file to be 98%
/// zeroes. It was measuring a half-downloaded torrent, not another encode. What that leaves
/// is one alien film at 2.41, which is enough to place the threshold and not enough to be
/// sure of it.
///
/// It does not have to clear the measurement's own noise, because there is none to clear:
/// the same cell measured three times running gave 96.86, 96.86, 96.86. That is one machine
/// — which is the only case there is, since donor and borrower are always measured on the
/// same one.
pub const MAX_APART: f64 = 1.0;

/// The cell a borrowed measurement is checked at: the donor's top rung.
///
/// `None` when the donor chose no rungs at all — there is then nothing to check against, and
/// answering with some other cell would be inventing one.
pub fn cell(donor: &Selection) -> Option<Cell> {
    // `rungs` is heaviest first, and the heaviest is the one `TARGET_VMAF` put at the top.
    donor.rungs.first().map(|top| Cell {
        bitrate_mbps: top.bitrate_mbps,
        height: top.height,
    })
}

/// What the donor scored at that cell, to compare the borrower against.
pub fn donor_vmaf(donor: &Selection) -> Option<f64> {
    donor.rungs.first().map(|top| top.vmaf)
}

/// What the check found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Close enough to be one material.
    Same { apart_x100: u64 },
    /// Too far apart: the loan has to go.
    Apart { apart_x100: u64 },
    /// The cell would not measure on all of the donor's seconds, so the two numbers describe
    /// different parts of two films and cannot be subtracted.
    NotComparable { used: usize, asked: usize },
}

/// Compare the borrower's cell with the donor's.
///
/// **By absolute distance, in both directions.** A borrower that scores far *above* the donor
/// is not thereby safe: its ladder would be built for material harder than its own and spend
/// bitrate nobody needed. The ladder is wrong either way, and the check says so either way.
///
/// ⚠ **And only when both sides describe the same seconds.** R-50: an episode 98% of which is
/// zeroes — a half-downloaded file — had one of the donor's three seconds fail to encode. The
/// remaining two averaged 95.81 against the donor's 96.38, and the loan was held at 0.57
/// apart. Nothing was wrong with the arithmetic; the two numbers simply described different
/// parts of two different films. A missing sample is a refusal, never a pass: the more of a
/// film is missing, the fewer chunks survive, and the better the wreckage scores.
pub fn judge(donor: f64, borrower: f64, chunks_used: usize, chunks_asked: usize) -> Verdict {
    if chunks_used != chunks_asked || chunks_asked == 0 {
        return Verdict::NotComparable {
            used: chunks_used,
            asked: chunks_asked,
        };
    }
    let apart = (donor - borrower).abs();
    // Hundredths, so that the wording carries an integer and reads the same in both languages.
    let apart_x100 = (apart * 100.0).round() as u64;
    if apart > MAX_APART {
        Verdict::Apart { apart_x100 }
    } else {
        Verdict::Same { apart_x100 }
    }
}
