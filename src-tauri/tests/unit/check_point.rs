//! T437 — the check after a loan, on the numbers it was derived from (R-48).
//!
//! Every figure here was measured on 2026-09-03: six subjects, the same seconds
//! (611/852/1713, ten each), the same grid. Written into the tests on purpose — if the
//! threshold or the cell ever drifts away from what was actually measured, these fail.

use vrcast_studio_lib::domain::check_point::{cell, donor_vmaf, judge, Verdict};
use vrcast_studio_lib::domain::measure_grid::Cell;
use vrcast_studio_lib::domain::measured_ladder::{
    select, Point, Selection, TARGET_VMAF, VMAF_STEP,
};

/// Blue Eye Samurai S01E01, the hull as measured.
fn e01() -> Selection {
    ladder_of(&[
        (1, 864, 84.07),
        (2, 864, 91.62),
        (3, 1080, 94.13),
        (4, 1080, 95.32),
        (6, 1080, 96.38),
        (8, 1080, 96.86),
        (12, 1080, 97.25),
    ])
}

fn ladder_of(points: &[(u64, u32, f64)]) -> Selection {
    let points: Vec<Point> = points
        .iter()
        .map(|&(bitrate_mbps, height, vmaf)| Point {
            bitrate_mbps,
            height,
            actual_bps: bitrate_mbps * 1_000_000,
            vmaf,
        })
        .collect();
    select(&points, TARGET_VMAF, VMAF_STEP)
}

/// What the four kin episodes scored on the top rung, and what the two others did.
const KIN_AT_TOP: [f64; 4] = [96.38, 96.67, 96.94, 96.42];
const SAME_SEASON_OTHER_ENCODE: f64 = 92.97;
const A_DIFFERENT_FILM: f64 = 93.97;

#[test]
fn the_cell_is_the_donors_top_rung() {
    // 6 Mbit/s is where `TARGET_VMAF` lands for this material — 96.38, the first point of the
    // hull to reach 96. The rule carries no number of its own; the number falls out of it.
    assert_eq!(
        cell(&e01()),
        Some(Cell {
            bitrate_mbps: 6,
            height: 1080
        })
    );
    assert_eq!(donor_vmaf(&e01()), Some(96.38));
}

#[test]
fn a_donor_with_no_rungs_gives_no_cell() {
    // Nothing to check against, and answering with some other cell would be inventing one.
    assert_eq!(cell(&ladder_of(&[])), None);
    assert_eq!(donor_vmaf(&ladder_of(&[])), None);
}

#[test]
fn every_pair_of_kin_passes_on_the_top_rung() {
    for (i, &a) in KIN_AT_TOP.iter().enumerate() {
        for &b in KIN_AT_TOP.iter().skip(i + 1) {
            assert!(
                matches!(judge(a, b), Verdict::Same { .. }),
                "two episodes of one release were called different material: {a} against {b}"
            );
        }
    }
}

#[test]
fn the_episode_that_passes_every_field_is_still_caught() {
    // **The whole reason this check exists.** E05 is the same season, the same release, and
    // equal to its neighbours in all eight fields `differs()` compares — frame, frame rate,
    // codec, pixel format, colour transfer and the rest. Only the encode is another one
    // (4.06 Mbit/s against 6.33). Lending cannot refuse it. One cell can.
    for &kin in KIN_AT_TOP.iter() {
        assert!(
            matches!(judge(kin, SAME_SEASON_OTHER_ENCODE), Verdict::Apart { .. }),
            "the other encode passed as the same material against {kin}"
        );
    }
}

#[test]
fn a_wholly_different_film_is_caught() {
    for &kin in KIN_AT_TOP.iter() {
        assert!(
            matches!(judge(kin, A_DIFFERENT_FILM), Verdict::Apart { .. }),
            "a different film passed as the same material against {kin}"
        );
    }
}

#[test]
fn a_borrower_far_above_the_donor_is_caught_too() {
    // Not a quality failure but still the wrong ladder: built for material harder than its
    // own, it spends bitrate nobody needed. Distance is absolute, in both directions.
    assert!(matches!(judge(93.97, 96.38), Verdict::Apart { .. }));
    assert!(matches!(judge(96.38, 93.97), Verdict::Apart { .. }));
}

#[test]
fn the_distance_is_reported_and_not_merely_the_verdict() {
    // A refusal that will not say by how much is a refusal nobody can argue with.
    assert_eq!(judge(96.38, 92.97), Verdict::Apart { apart_x100: 341 });
    assert_eq!(judge(96.38, 96.42), Verdict::Same { apart_x100: 4 });
}

#[test]
fn on_the_bottom_rung_no_threshold_could_work_at_all() {
    // **Why the cell is at the top, on the numbers rather than on a preference.** At 1 Mbit/s
    // a wholly different film scored 86.76, which is *inside* the band the four kin made
    // (83.18 to 91.03). Two legitimate episodes stand further apart than one of them stands
    // from an alien film — so any threshold either passes the alien or rejects the kin.
    // This is the check that fails if the cell is ever moved down the ladder.
    let kin_at_bottom: [f64; 4] = [84.07, 83.18, 91.03, 88.32];
    let alien_at_bottom: f64 = 86.76;

    let widest_among_kin = kin_at_bottom
        .iter()
        .flat_map(|a| kin_at_bottom.iter().map(move |b| (a - b).abs()))
        .fold(0.0_f64, f64::max);
    let nearest_alien = kin_at_bottom
        .iter()
        .map(|k| (k - alien_at_bottom).abs())
        .fold(f64::INFINITY, f64::min);

    assert!(
        widest_among_kin > nearest_alien,
        "the bands came apart at the bottom rung after all: kin spread {widest_among_kin}, \
         nearest alien {nearest_alien} — the reasoning for putting the cell at the top rests \
         on this being false"
    );
}
