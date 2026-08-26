//! T232 — the measurement's own arithmetic: where to measure, and what to keep.
//!
//! Every expected value here was produced by running the script's own Python
//! (`measure-ladder.sh`), not by working out what the Rust ought to do. That is the only way
//! a port can be shown to be a port (constitution VI).

use vrcast_studio_lib::domain::chunks::{reference_chunks, CHUNK_S};
use vrcast_studio_lib::domain::ladder::plan;
use vrcast_studio_lib::domain::ladder::{
    buildable, from_measurement, NotBuildable, Quality, Reason, SourceFacts,
};
use vrcast_studio_lib::domain::measure_grid::{grid, grid_bitrates_mbps, grid_heights, Cell};
use vrcast_studio_lib::domain::measured_ladder::{select, Chosen, Point, TARGET_VMAF, VMAF_STEP};

fn source(width: u32, height: u32, fps: u32, native: Option<u32>) -> SourceFacts {
    SourceFacts {
        width,
        height,
        fps,
        bitrate_bps: 60_000_000,
        heavier_codec: false,
        native_height: native,
    }
}

fn point(bitrate_mbps: u64, height: u32, vmaf: f64) -> Point {
    Point {
        bitrate_mbps,
        height,
        actual_bps: bitrate_mbps * 1_000_000,
        vmaf,
    }
}

fn chosen(rungs: &[Chosen]) -> Vec<(u64, u32)> {
    rungs.iter().map(|c| (c.bitrate_mbps, c.height)).collect()
}

// ---------- where to measure ----------

#[test]
fn the_grid_reaches_past_the_anchor_in_both_directions() {
    // Half again above and down to below a fifth: the probe only has to land within about a
    // factor of two, and the optimum has to fall inside the grid rather than on its edge.
    assert_eq!(grid_bitrates_mbps(16), vec![3, 6, 10, 16, 24]);
    assert_eq!(grid_bitrates_mbps(22), vec![4, 8, 13, 22, 33]);
    assert_eq!(grid_bitrates_mbps(35), vec![6, 12, 21, 35, 52]);

    // And it is deliberately **not** the ladder's [1.0, 0.55, 0.3, 0.17], which only walks
    // down: a grid that stopped at the anchor could never show the anchor was too low.
    assert!(
        grid_bitrates_mbps(16).iter().any(|b| *b > 16),
        "the grid never looked above the anchor"
    );
}

#[test]
fn halves_in_the_grid_go_to_the_even_number() {
    // Same tie-break as the ladder, and for the same reason: the script rounds in Python.
    // An anchor of 3 gives a top of 4, because 4.5 goes to the even number. Rounding away
    // from zero would give 5 — a bitrate the grid would then measure and the script never
    // would.
    assert_eq!(grid_bitrates_mbps(3), vec![1, 2, 3, 4]);
    assert_eq!(grid_bitrates_mbps(5), vec![1, 2, 3, 5, 8]);
    // Nothing falls below a megabit, however light the material.
    assert_eq!(grid_bitrates_mbps(1), vec![1, 2]);
}

#[test]
fn an_upscale_caps_the_grid_where_the_detail_stops() {
    // Mars Express: 1080 upscaled to 2160. The optimum sat at 1728 = 1.6× the native height
    // and held there at 4, 8 and 14 Mbit/s, while everything measured above it lost. Points
    // above the cap are not a safety margin — they are minutes of encoding spent on a
    // picture that has no more detail in it.
    assert_eq!(
        grid_heights(&source(3840, 2160, 24, Some(1080))),
        vec![1728, 1382, 1104, 884, 706]
    );
    // Native material keeps its full height.
    assert_eq!(
        grid_heights(&source(3840, 2160, 24, None)),
        vec![2160, 1728, 1382, 1104, 884, 706]
    );
    // Every height is even: an odd one is not encodable in 4:2:0 at all.
    for h in grid_heights(&source(3840, 1038, 48, None)) {
        assert_eq!(h % 2, 0, "height {h} is odd");
    }
}

#[test]
fn each_bitrate_is_measured_at_three_heights_rather_than_one() {
    // The density guess says roughly where the optimum should be; its neighbours are what
    // catch it when the guess is wrong, which on real material it regularly is.
    let cells = grid(&source(3840, 2160, 24, None), 16);
    let expected: Vec<Cell> = [
        (3, 1382),
        (3, 1104),
        (3, 884),
        (6, 2160),
        (6, 1728),
        (6, 1382),
        (10, 2160),
        (10, 1728),
        (16, 2160),
        (16, 1728),
        (24, 2160),
        (24, 1728),
    ]
    .iter()
    .map(|(bitrate_mbps, height)| Cell {
        bitrate_mbps: *bitrate_mbps,
        height: *height,
    })
    .collect();
    assert_eq!(cells, expected);

    // The top bitrates get two heights, not three, because the grid ends at the source's
    // own: there is no point above full resolution to try.
    assert_eq!(cells.iter().filter(|c| c.bitrate_mbps == 24).count(), 2);
    assert_eq!(cells.iter().filter(|c| c.bitrate_mbps == 3).count(), 3);

    // The same point is never measured twice — half an hour is already the budget.
    let mut seen = cells.clone();
    seen.sort_by_key(|c| (c.bitrate_mbps, c.height));
    seen.dedup();
    assert_eq!(seen.len(), cells.len());
}

// ---------- which chunks ----------

#[test]
fn the_chunks_skip_the_titles_but_not_at_the_cost_of_measuring_nothing() {
    // A two-hour film: the guards hold, and the three chunks land on the light stretch, the
    // ordinary middle and the heavy one.
    let mut long = vec![500u64; 1200];
    long[400..420].fill(50);
    long[900..920].fill(5000);
    assert_eq!(reference_chunks(&long, CHUNK_S), vec![233, 590, 947]);

    // Two minutes: 180 + 190 seconds of guard leave nothing at all, so the guards are
    // dropped. A measurement including the titles beats no measurement.
    let mut short = vec![100u64; 60];
    short.extend(std::iter::repeat_n(900, 20));
    short.extend(std::iter::repeat_n(500, 40));
    assert_eq!(reference_chunks(&short, CHUNK_S), vec![11, 55, 60]);

    // Nothing at all is not a crash.
    assert_eq!(reference_chunks(&[], CHUNK_S), vec![0, 0, 0]);
}

// ---------- what to keep ----------

#[test]
fn the_top_is_cut_by_the_target_rather_than_by_the_size_of_the_gain() {
    // mandoup: 22 Mbit/s reaches the target, and 35 buys +1.12 VMAF over it against a
    // threshold of about 6 that a person can notice at all. Those bits are paid for by
    // every viewer and seen by none.
    let measured = [
        point(4, 1440, 85.60),
        point(8, 1440, 90.20),
        point(14, 2160, 93.80),
        point(22, 2160, 96.10),
        point(35, 2160, 97.22),
    ];
    let chose = select(&measured, TARGET_VMAF, VMAF_STEP);
    assert_eq!(chosen(&chose.above_target), vec![(35, 2160)]);
    assert_eq!(chosen(&chose.rungs), vec![(22, 2160), (8, 1440), (4, 1440)]);
}

#[test]
fn a_thin_gain_below_the_target_does_not_take_the_top_away() {
    // This is where a relative cut goes wrong. "Drop it if the gain is under two" would take
    // 22 off here, and then ask the same question of 14, and the ladder slides downwards
    // until nobody is served the quality the source has. An absolute target holds still.
    let measured = [
        point(4, 1440, 85.60),
        point(8, 1440, 90.20),
        point(14, 2160, 93.80),
        point(22, 2160, 94.30),
    ];
    let chose = select(&measured, TARGET_VMAF, VMAF_STEP);
    assert!(chose.above_target.is_empty());
    assert_eq!(chose.rungs[0].bitrate_mbps, 22, "the top was given away");
    assert_eq!(chosen(&chose.rungs), vec![(22, 2160), (8, 1440), (4, 1440)]);
}

#[test]
fn the_same_score_from_more_of_the_frame_is_the_better_of_the_two() {
    let chose = select(
        &[
            point(8, 1080, 92.0),
            point(8, 1440, 92.0),
            point(14, 1440, 96.5),
        ],
        TARGET_VMAF,
        VMAF_STEP,
    );
    assert_eq!(chosen(&chose.rungs), vec![(14, 1440), (8, 1440)]);
}

#[test]
fn material_already_at_the_target_gets_one_rung_and_the_rest_is_dropped() {
    // Animation from a 1080 upscale saturated at 4 Mbit/s in HEVC. More than half of the
    // old hard-wired grid lay above the point of saturation and was measured for nothing.
    let chose = select(
        &[point(2, 2160, 96.4), point(4, 2160, 97.1)],
        TARGET_VMAF,
        VMAF_STEP,
    );
    assert_eq!(chosen(&chose.rungs), vec![(2, 2160)]);
    assert_eq!(chosen(&chose.above_target), vec![(4, 2160)]);
}

#[test]
fn a_ladder_of_one_is_widened_when_there_is_anything_to_widen_it_with() {
    // Nothing below the top is a whole step away, but a viewer who cannot hold the top still
    // needs somewhere to go. The lightest measured rung is taken even though the two look
    // alike to everyone else.
    let chose = select(
        &[
            point(4, 1080, 94.0),
            point(6, 1080, 95.2),
            point(8, 1440, 95.9),
        ],
        TARGET_VMAF,
        VMAF_STEP,
    );
    assert_eq!(chosen(&chose.rungs), vec![(8, 1440), (4, 1080)]);

    // With a single point measured there is nothing to widen it with, and one rung stands.
    let alone = select(&[point(5, 1080, 88.0)], TARGET_VMAF, VMAF_STEP);
    assert_eq!(chosen(&alone.rungs), vec![(5, 1080)]);
    assert!(alone.above_target.is_empty());
}

#[test]
fn nothing_measured_is_not_a_ladder_of_nonsense() {
    let empty = select(&[], TARGET_VMAF, VMAF_STEP);
    assert!(empty.rungs.is_empty());
    assert!(empty.hull.is_empty());
    assert!(empty.above_target.is_empty());
}

// ---------- a rung that knows where its number came from (T239, T240) ----------

#[test]
fn a_measured_ladder_keeps_the_height_the_measurement_chose() {
    // Not the one the density formula would have guessed. On mandoup the formula dropped
    // 22 Mbit/s to a height of 1604 and the measurement said the full 2160 was better —
    // and the measurement is right by construction: it looked.
    let src = source(3840, 2160, 24, None);
    let chose = select(
        &[
            point(4, 1440, 85.60),
            point(8, 1440, 90.20),
            point(22, 2160, 96.10),
        ],
        TARGET_VMAF,
        VMAF_STEP,
    );

    let laid =
        from_measurement(&chose.rungs, &src, None, false).expect("a sound source was refused");
    assert_eq!(
        laid.rungs
            .iter()
            .map(|r| (r.bitrate_bps / 1_000_000, r.height))
            .collect::<Vec<_>>(),
        vec![(22, 2160), (8, 1440), (4, 1440)]
    );

    // Each rung carries what it is worth, and says it was measured here.
    assert_eq!(
        laid.rungs[0].quality,
        Quality::MeasuredHere { vmaf_x100: 9610 }
    );
    assert_eq!(laid.rungs[0].quality.vmaf(), Some(96.1));
    assert!(laid.rungs[0].reasons.contains(&Reason::MeasuredOptimum));

    // And the geometry is the same as the formula's ladder gets: one place builds a rung.
    assert_eq!(laid.rungs[0].width, 3840);
    assert_eq!(laid.rungs[1].width, 2560, "the aspect was not kept");
    assert!(laid.rungs[0].bufsize_bps >= laid.rungs[0].maxrate_bps);
}

#[test]
fn a_borrowed_measurement_does_not_pass_for_one_taken_here() {
    // The next episode of a season is usually the same source, so building on its
    // measurement is right. Showing it as measured on THIS file is not (FR-145).
    let src = source(3840, 2160, 24, Some(1080));
    let chose = select(
        &[point(6, 1728, 92.0), point(14, 1728, 96.4)],
        TARGET_VMAF,
        VMAF_STEP,
    );

    let borrowed = from_measurement(&chose.rungs, &src, None, true).expect("refused");
    assert_eq!(
        borrowed.rungs[0].quality,
        Quality::Borrowed { vmaf_x100: 9640 }
    );
    assert!(borrowed.rungs[0]
        .reasons
        .contains(&Reason::BorrowedMeasurement));

    // It is still enough to build on: the alternative is half an hour per episode.
    assert!(buildable(&borrowed.rungs).is_ok());
}

#[test]
fn a_ladder_out_of_the_formula_is_not_built() {
    // This is the rule the whole measurement exists for (FR-141). The formula misses in
    // both directions on real films, and building on it makes the miss permanent: hours
    // of encoding and gigabytes on a server behind rungs nobody can defend.
    let src = source(3840, 2160, 24, None);
    let guessed = plan(Some(22_000_000), &src, None).expect("a sound source was refused");

    for rung in &guessed.rungs {
        assert_eq!(rung.quality, Quality::NotMeasured);
        assert_eq!(
            rung.quality.vmaf(),
            None,
            "a guess was handed out as a score"
        );
    }
    assert_eq!(
        buildable(&guessed.rungs),
        Err(NotBuildable::RungsNotMeasured {
            indexes: (0..guessed.rungs.len()).collect()
        })
    );

    // And an empty ladder is refused as an empty ladder rather than as an unmeasured one:
    // the two want different things said to a person.
    assert_eq!(buildable(&[]), Err(NotBuildable::NoRungs));
}

#[test]
fn one_rung_edited_by_hand_stops_the_build_by_itself() {
    // A rung moved off the measured grid is no longer measured, and the ladder is only as
    // measured as its least measured rung. Re-measuring one point is minutes; the half
    // hour is the price of a whole grid, not of a correction.
    let src = source(3840, 2160, 24, None);
    let chose = select(
        &[
            point(4, 1440, 85.60),
            point(8, 1440, 90.20),
            point(22, 2160, 96.10),
        ],
        TARGET_VMAF,
        VMAF_STEP,
    );
    let mut laid = from_measurement(&chose.rungs, &src, None, false).expect("refused");
    assert!(buildable(&laid.rungs).is_ok());

    laid.rungs[1].bitrate_bps = 12_000_000;
    laid.rungs[1].quality = Quality::NotMeasured;
    assert_eq!(
        buildable(&laid.rungs),
        Err(NotBuildable::RungsNotMeasured { indexes: vec![1] })
    );
}
