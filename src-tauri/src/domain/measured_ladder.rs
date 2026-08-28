//! T231 — turning a grid of measurements into a ladder (FR-144).
//!
//! Three steps, all of them ported from `measure-ladder.sh` unchanged (constitution VI):
//!
//! 1. **The hull.** For each bitrate, the height that scored best. A bitrate has one right
//!    resolution and the measurement says which; the formula only guesses.
//! 2. **The top, by an absolute target.** Everything above the first rung to reach the
//!    target is dropped: those bits buy nothing a viewer can see, and cost both the channel
//!    and the disk.
//! 3. **Down by a visible step.** Rungs closer together than the step are two encodes of one
//!    quality, and a viewer who drops from one to the other has changed nothing.

use serde::{Deserialize, Serialize};

/// The quality the top rung aims for.
///
/// Measured in VMAF, where about 6 points is the difference a person notices at all. 96 is
/// close enough to the source that the remaining distance is not worth paying for — proved
/// the other way round on mandoup, where 35 Mbit/s bought **+1.12** over 22.
pub const TARGET_VMAF: f64 = 96.0;

/// How far apart two rungs have to be to be worth having separately.
pub const VMAF_STEP: f64 = 4.0;

/// One measured point of the grid.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub bitrate_mbps: u64,
    pub height: u32,
    /// What the encode actually came out at — the target is asked for, not obeyed.
    pub actual_bps: u64,
    pub vmaf: f64,
}

/// A rung as the measurement chose it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Chosen {
    pub bitrate_mbps: u64,
    pub height: u32,
    pub vmaf: f64,
}

/// What the measurement decided, and what it decided against.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Selection {
    /// The ladder, heaviest rung first.
    pub rungs: Vec<Chosen>,
    /// Rungs that reached past the target and were dropped — kept so a person can be shown
    /// what was left out and why, rather than wondering where their bitrate went.
    pub above_target: Vec<Chosen>,
    /// The best height at every bitrate measured, lightest first.
    pub hull: Vec<Chosen>,
}

/// Choose a ladder from measured points.
pub fn select(points: &[Point], target_vmaf: f64, vmaf_step: f64) -> Selection {
    // --- 1. the hull: the best height at each bitrate -------------------------------
    let mut bitrates: Vec<u64> = points.iter().map(|p| p.bitrate_mbps).collect();
    bitrates.sort_unstable();
    bitrates.dedup();

    let hull: Vec<Chosen> = bitrates
        .iter()
        .filter_map(|bitrate| {
            points
                .iter()
                .filter(|p| p.bitrate_mbps == *bitrate)
                // Best score, and on a tie the taller picture: same quality from more of
                // the frame is not a tie, it is the better of the two.
                .max_by(|a, b| {
                    a.vmaf
                        .partial_cmp(&b.vmaf)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then(a.height.cmp(&b.height))
                })
                .map(|p| Chosen {
                    bitrate_mbps: p.bitrate_mbps,
                    height: p.height,
                    vmaf: p.vmaf,
                })
        })
        .collect();

    if hull.is_empty() {
        return Selection {
            rungs: Vec::new(),
            above_target: Vec::new(),
            hull,
        };
    }

    // --- 2. the top, by an absolute target -------------------------------------------
    //
    // **Absolute, not relative.** A relative cut — "drop it if the gain is under two" —
    // cascades: having dropped 35 it drops 22 by the same reasoning, and the ladder slides
    // into 1440p. A target does not behave that way: it holds at one place and stops.
    let top = hull
        .iter()
        .position(|c| c.vmaf >= target_vmaf)
        .unwrap_or(hull.len() - 1);
    let above_target = hull[top + 1..].to_vec();
    let kept = &hull[..=top];

    // --- 3. down by a visible step ----------------------------------------------------
    let mut rungs = vec![kept[kept.len() - 1]];
    for candidate in kept[..kept.len() - 1].iter().rev() {
        if rungs[rungs.len() - 1].vmaf - candidate.vmaf >= vmaf_step {
            rungs.push(*candidate);
        }
    }
    // A ladder of one is not a ladder. If nothing below the top was far enough away, the
    // lightest measured rung is taken anyway: the viewer who cannot hold the top needs
    // somewhere to go, even if the two look alike to everyone else.
    if rungs.len() < 2 && kept.len() > 1 {
        rungs.push(kept[0]);
    }
    rungs.sort_by_key(|c| std::cmp::Reverse(c.bitrate_mbps));

    // --- 4. fill the holes the checker would object to ---------------------------------
    //
    // **The two halves of the rule, finally introduced.** Everything above judges a rung by
    // quality alone; `ladder::validate` judges it by bitrate alone, and refuses a step
    // outside 1.5–2.0×. On flat material — animation, and anything whose curve levels off —
    // a fourfold drop in bitrate is worth barely four points of VMAF, so the loop above
    // skips the middle and the checker then objects to the hole. The application showed a
    // person a complaint about a ladder it had chosen itself.
    //
    // **Filling, not descending.** The obvious alternative — walk down from the top taking
    // the largest legal step each time — gives the same answer on every ladder that has a
    // hole, and a worse one on every ladder that has none: it bolts a rung onto the bottom
    // of `15, 8, 4, 3`, which was already right. This cannot: where the checker is silent
    // it does nothing at all.
    //
    // The point inserted is a real one out of the hull, with a VMAF somebody measured — so
    // a filled rung is as measured as any other, and `Quality::MeasuredHere` stays true.
    //
    // Repeated until nothing changes, because filling one hole can leave a smaller one
    // beneath it. It terminates: every pass consumes a hull point, and the hull is finite.
    loop {
        let mut filled = false;
        for i in 0..rungs.len().saturating_sub(1) {
            let above = rungs[i].bitrate_mbps;
            let below = rungs[i + 1].bitrate_mbps;
            if super::ladder::step_is_allowable(
                above * super::ladder::MBIT,
                below * super::ladder::MBIT,
            ) {
                continue;
            }
            // The lightest point that makes the step above it legal: the hole is closed with
            // as little extra encoding as it can be closed with.
            let patch = hull
                .iter()
                .filter(|c| c.bitrate_mbps > below && c.bitrate_mbps < above)
                .filter(|c| {
                    super::ladder::step_is_allowable(
                        above * super::ladder::MBIT,
                        c.bitrate_mbps * super::ladder::MBIT,
                    )
                })
                .min_by_key(|c| c.bitrate_mbps)
                .copied();
            if let Some(patch) = patch {
                rungs.insert(i + 1, patch);
                filled = true;
                break;
            }
            // Nothing in the hull can close it. The hole is in the measurement, not in the
            // choosing, and inventing a rung nobody measured would be the one lie this
            // module could tell that a person would believe.
        }
        if !filled {
            break;
        }
    }

    Selection {
        rungs,
        above_target,
        hull,
    }
}
