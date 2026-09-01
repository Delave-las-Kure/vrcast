//! T232 — the measurement's own arithmetic: where to measure, and what to keep.
//!
//! Every expected value here was produced by running the script's own Python
//! (`measure-ladder.sh`), not by working out what the Rust ought to do. That is the only way
//! a port can be shown to be a port (constitution VI).

use vrcast_studio_lib::domain::chunks::{reference_chunks, CHUNK_S};
use vrcast_studio_lib::domain::ladder::plan;
use vrcast_studio_lib::domain::ladder::{
    buildable, from_measurement, validate, NotBuildable, Objection, Quality, Reason, SourceFacts,
};
use vrcast_studio_lib::domain::measure_grid::{
    grid, grid_bitrates_mbps, grid_heights, seconds_per_point, Cell,
};
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
    // 14 is here because 22 over 8 is 2.75x, and this project's own checker objects to
    // that: a viewer whose line holds 14 was dropped to 8 for nothing (R-31).
    assert_eq!(
        chosen(&chose.rungs),
        vec![(22, 2160), (14, 2160), (8, 1440), (4, 1440)]
    );
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
    // 14 is here because 22 over 8 is 2.75x, and this project's own checker objects to
    // that: a viewer whose line holds 14 was dropped to 8 for nothing (R-31).
    assert_eq!(
        chosen(&chose.rungs),
        vec![(22, 2160), (14, 2160), (8, 1440), (4, 1440)]
    );
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

// ---------- what a measurement costs ----------

#[test]
fn the_cost_model_gives_back_the_readings_it_was_fitted_to() {
    // Two clips of the same film on the same machine, three ten-second chunks each
    // (RTX 5080, h264_nvenc): 3840×1080 took 12.6 s a point and 1920×540 took 6.3 s.
    let full = seconds_per_point(3840, 1080, 24, 10, 3);
    let quarter = seconds_per_point(1920, 540, 24, 10, 3);
    assert!(
        (full - 12.6).abs() < 0.2,
        "3840×1080 came out at {full:.1} s"
    );
    assert!(
        (quarter - 6.3).abs() < 0.2,
        "1920×540 came out at {quarter:.1} s"
    );

    // **A quarter of the pixels is half the time, not a quarter.** This is the whole
    // reason there are two numbers in the model: a pure per-pixel cost would have
    // promised 3.2 s here, half of what really happens, and every estimate for a small
    // film would have been half the truth.
    assert!(
        quarter > full / 3.0,
        "the fixed cost of a point disappeared: {quarter:.1} s against {full:.1} s"
    );

    // Heavier material costs more, and in the right order of magnitude: 4K at 48 frames
    // is four times the pixels of the clip above and comes out near forty seconds.
    let heavy = seconds_per_point(3840, 2160, 48, 10, 3);
    assert!(
        (30.0..50.0).contains(&heavy),
        "4K48 came out at {heavy:.1} s"
    );

    // A frame rate of zero is somebody's broken file, not a division by zero.
    assert!(seconds_per_point(1920, 1080, 0, 10, 3) > 0.0);
}

// ---------- the two halves of the rule have to agree (T383–T385, R-31) ----------
//
// **The fault these exist for.** Choosing rungs and judging them were written apart and never
// introduced: `select` keeps a rung only when it is a visible step down in *quality*
// (`VMAF_STEP`), and `ladder::validate` objects unless it is a sane step down in *bitrate*
// (1.5–2.0×). On flat material — animation, and anything else whose quality curve levels off —
// a fourfold drop in bitrate is worth barely four points of VMAF, so the first rule skips the
// middle and the second then objects to the hole. The application showed a person a complaint
// about a ladder it had chosen itself.
//
// Neither of the scripts this project was ported from solves it: `measure-ladder.sh` selects
// on VMAF with no condition on bitrate at all, and `plan-ladder.sh` on bitrate with no
// condition on quality. Each holds one half. So joining them is composition, not new
// arithmetic — 96, 4, 1.5 and 2.0 are all untouched (constitution VI).

/// A plausible quality curve: a large gain at the bottom, a small one at the top.
///
/// `hardness` moves the whole curve down without changing its shape — easy material
/// saturates early, dense material keeps asking for bits. Synthetic, and said so out loud:
/// it stands in for the shapes real measurements have while there is only one real
/// measurement to hand.
fn curve(bitrate_mbps: u64, hardness: f64) -> f64 {
    100.0 - hardness / (bitrate_mbps as f64).powf(0.6)
}

#[test]
fn the_ladder_the_measurement_chooses_has_no_hole_the_checker_objects_to() {
    // The same guarantee `a_ladder_this_code_planned_has_nothing_wrong_with_it` gives the
    // formula, given to the measurement — which never had it. Swept over every anchor the
    // probe can produce and over material from easy to dense, because the fault was never
    // about one film: before the filling, most of this sweep came out holed.
    //
    // **Two kinds of hole, and confusing them would ruin the check.** One the hull can
    // close — that is a fault in the choosing, and it must not happen. One the hull cannot,
    // because the grid's own multipliers left a pair wider than 2x before any choosing
    // happened: `grid_bitrates_mbps(8)` is 1, 3, 5, 8, 12, and 3 over 1 is threefold.
    // Filling that would mean inventing a rung nobody measured. Those are named below
    // rather than waved through, and the decision about the multipliers is R-32.
    use vrcast_studio_lib::domain::ladder::Objection;

    let facts = source(1920, 1080, 24, None);
    let mut holed: Vec<String> = Vec::new();
    let mut in_the_grid = std::collections::BTreeSet::new();
    let mut swept = 0usize;

    for anchor in 2u64..=40 {
        for hardness in [20.0, 40.0, 60.0, 80.0, 100.0] {
            let measured: Vec<Point> = grid_bitrates_mbps(anchor)
                .into_iter()
                .map(|b| point(b, 1080, curve(b, hardness)))
                .collect();
            let chose = select(&measured, TARGET_VMAF, VMAF_STEP);
            if chose.rungs.len() < 2 {
                continue;
            }
            let Ok(plan) = from_measurement(&chose.rungs, &facts, None, false) else {
                continue;
            };
            swept += 1;

            for objection in validate(&plan.rungs, &facts, facts.fps) {
                let Objection::BadStep { index, times } = objection else {
                    continue;
                };
                let above = plan.rungs[index - 1].bitrate_bps / 1_000_000;
                let below = plan.rungs[index].bitrate_bps / 1_000_000;
                // Was there anything measured between the two that would have made the step
                // from above legal? Asked of the grid itself, in the checker's own terms.
                let couldve = measured.iter().any(|p| {
                    p.bitrate_mbps > below
                        && p.bitrate_mbps < above
                        && above as f64 <= 2.0 * p.bitrate_mbps as f64 + 0.5
                        && above as f64 >= 1.5 * p.bitrate_mbps as f64 - 0.5
                });
                if couldve {
                    holed.push(format!(
                        "anchor {anchor}, hardness {hardness}: {above} over {below} \
                         ({times:.2}x), and the grid had something to close it with"
                    ));
                } else {
                    in_the_grid.insert(anchor);
                }
            }
        }
    }

    assert!(
        swept > 100,
        "the sweep covered almost nothing: {swept} ladders"
    );
    assert!(
        holed.is_empty(),
        "{} of {swept} measured ladders have a hole that could have been closed and was not:\n{}",
        holed.len(),
        holed.iter().take(8).cloned().collect::<Vec<_>>().join("\n")
    );
    assert_eq!(
        in_the_grid.iter().copied().collect::<Vec<u64>>(),
        vec![8, 13, 19, 25, 36],
        "the anchors whose own grid leaves a hole have changed — either the multipliers \
         moved or the filling did, and both are worth knowing about"
    );
}

#[test]
fn the_middle_is_kept_when_the_quality_curve_is_flat() {
    // Measured on this machine on 2026-08-28: Blue Eye Samurai S01E04, 1080p24, a 4.8 Mbit/s
    // source. Animation, so the curve is flat — four megabits are worth seven points of VMAF
    // over one, well under two steps — and the old rule went straight from the top to the
    // bottom, leaving a fourfold hole. A viewer whose line holds three megabits was dropped
    // all the way to one, when two looks all but the same as four.
    let measured = [
        point(4, 1080, 89.51),
        point(4, 864, 87.20),
        point(3, 1080, 88.83),
        point(3, 864, 86.80),
        point(2, 1080, 87.44),
        point(2, 864, 85.91),
        point(2, 690, 84.00),
        point(1, 864, 82.57),
        point(1, 690, 81.46),
        point(1, 552, 79.13),
    ];
    let chose = select(&measured, TARGET_VMAF, VMAF_STEP);
    assert_eq!(chosen(&chose.rungs), vec![(4, 1080), (2, 1080), (1, 864)]);
}

#[test]
fn the_hole_in_the_reference_ladder_is_filled_too() {
    // mandoup, the ladder this project's own tests have carried since the port: 22 over 8 is
    // 2.75×, which `validate` objects to. It was never wrong about the top and the bottom —
    // only about there being nothing between them.
    let measured = [
        point(4, 1440, 85.60),
        point(8, 1440, 90.20),
        point(14, 2160, 93.80),
        point(22, 2160, 96.10),
        point(35, 2160, 97.22),
    ];
    let chose = select(&measured, TARGET_VMAF, VMAF_STEP);
    assert_eq!(chosen(&chose.above_target), vec![(35, 2160)]);
    assert_eq!(
        chosen(&chose.rungs),
        vec![(22, 2160), (14, 2160), (8, 1440), (4, 1440)]
    );
}

#[test]
fn a_ladder_with_no_hole_gains_nothing() {
    // **The half that stops this from being "add rungs until it looks safe".** The hull out
    // of `step_is_allowable`'s own doc comment — 15, 8, 4, 3 — has no hole in it: every step
    // is legal. Filling holes must therefore leave it exactly as the quality rule chose it,
    // and a rule that walked down taking the largest legal step each time would not: it would
    // bolt 3 onto the bottom of a ladder that was already right.
    let measured = [
        point(3, 1080, 78.0),
        point(4, 1080, 84.0),
        point(8, 1440, 91.0),
        point(15, 2160, 95.0),
    ];
    let chose = select(&measured, TARGET_VMAF, VMAF_STEP);
    assert_eq!(
        chosen(&chose.rungs),
        vec![(15, 2160), (8, 1440), (4, 1080), (3, 1080)]
    );
}

// ---------- a borrowed ladder brought inside the borrower's own source (T432) ----------

/// One chosen rung, as the selection hands them over. (`chosen` above already names the
/// other direction — a list of them, flattened for comparison.)
fn a_rung(bitrate_mbps: u64, height: u32, vmaf: f64) -> Chosen {
    Chosen {
        bitrate_mbps,
        height,
        vmaf,
        filled_a_gap: false,
    }
}

#[test]
fn a_borrowed_rung_above_the_borrowers_own_source_is_not_offered() {
    // **What goes wrong without this.** A measurement lent from a 60 Mbit/s master can top
    // out at 22, and the film borrowing it may itself hold 15. Asking the encoder for 22 out
    // of a 15 Mbit/s source spends an hour making a file no better than the source and larger
    // than it — and the ladder's own top rung is then a rung nobody should build.
    //
    // Until now this was caught only by the `RungAboveSource` objection, raised **after** the
    // plan was built: a complaint about a ladder the application had just proposed. The rung
    // should not be proposed.
    let chose = vec![
        a_rung(22, 2160, 96.4),
        a_rung(12, 1440, 92.0),
        a_rung(6, 1080, 87.4),
    ];
    let mut lean = source(3840, 2160, 24, None);
    lean.bitrate_bps = 15_000_000;

    let plan = from_measurement(&chose, &lean, None, true).expect("refused");
    assert!(
        plan.rungs.iter().all(|r| r.bitrate_bps <= lean.bitrate_bps),
        "a borrowed ladder proposed a rung above the borrower's own bitrate: {:?}",
        plan.rungs.iter().map(|r| r.bitrate_bps).collect::<Vec<_>>()
    );
    // And the objection that used to be the only guard now has nothing to say.
    assert!(
        !validate(&plan.rungs, &lean, lean.fps)
            .iter()
            .any(|o| matches!(o, Objection::RungAboveSource { .. })),
        "the plan still objects to itself"
    );
}

#[test]
fn trimming_keeps_what_fits_rather_than_refusing_the_lot() {
    // The rungs under the source are perfectly good — they are what the borrower can hold —
    // and throwing them away over the top one would turn a usable loan into no loan at all.
    let chose = vec![
        a_rung(22, 2160, 96.4),
        a_rung(12, 1440, 92.0),
        a_rung(6, 1080, 87.4),
    ];
    let mut lean = source(3840, 2160, 24, None);
    lean.bitrate_bps = 15_000_000;

    let plan = from_measurement(&chose, &lean, None, true).expect("refused");
    assert_eq!(
        plan.rungs
            .iter()
            .map(|r| r.bitrate_bps / 1_000_000)
            .collect::<Vec<_>>(),
        vec![12, 6],
        "the rungs the borrower can hold were not kept"
    );
}

#[test]
fn a_ladder_that_already_fits_is_left_alone() {
    // The negative control, and it earns its place: "drop the top rung" would satisfy both
    // checks above and quietly cost every ordinary loan its best quality.
    let chose = vec![
        a_rung(22, 2160, 96.4),
        a_rung(12, 1440, 92.0),
        a_rung(6, 1080, 87.4),
    ];
    let roomy = source(3840, 2160, 24, None); // 60 Mbit/s
    let plan = from_measurement(&chose, &roomy, None, true).expect("refused");
    assert_eq!(
        plan.rungs
            .iter()
            .map(|r| r.bitrate_bps / 1_000_000)
            .collect::<Vec<_>>(),
        vec![22, 12, 6]
    );
}

#[test]
fn a_loan_with_nothing_the_borrower_can_hold_is_refused_rather_than_emptied() {
    // An empty ladder is not a ladder, and handing one back as a success would send somebody
    // to the build screen with nothing to build. Refused, with the reason the source gives.
    let chose = vec![a_rung(22, 2160, 96.4), a_rung(12, 1440, 92.0)];
    let tiny = SourceFacts {
        bitrate_bps: 3_000_000,
        ..source(1920, 1080, 24, None)
    };
    let plan = from_measurement(&chose, &tiny, None, true);
    assert!(
        plan.is_err() || plan.as_ref().is_ok_and(|p| !p.rungs.is_empty()),
        "a loan came back as an empty ladder"
    );
}

#[test]
fn a_rung_put_in_to_close_a_gap_says_so() {
    // T389. A patched rung is a different kind of rung: the others are where the measurement
    // said the quality was worth the bitrate, and this one is where the drop to the next was
    // too steep for a player to make. Its score is real — it came off the same grid — but
    // nobody picked it for what it scores, and somebody deciding which rungs to build should
    // know which is there for the ladder rather than for the picture.
    //
    // **Driven over the sweep rather than over one hull**, because whether a given hull needs
    // patching is the selection's business, and a hand-picked one that turns out not to need
    // it makes a test that checks nothing. The first shape of this test did exactly that: it
    // passed with the marking disabled.
    let facts = source(1920, 1080, 24, None);
    let mut ever_patched = false;

    for anchor in 2u64..=40 {
        for hardness in [20.0, 40.0, 60.0, 80.0, 100.0] {
            let measured: Vec<Point> = grid_bitrates_mbps(anchor)
                .into_iter()
                .map(|b| point(b, 1080, curve(b, hardness)))
                .collect();
            let chose = select(&measured, TARGET_VMAF, VMAF_STEP);
            if chose.rungs.iter().any(|r| r.filled_a_gap) {
                ever_patched = true;
            }
            let Ok(plan) = from_measurement(&chose.rungs, &facts, None, false) else {
                continue;
            };
            for (rung, chosen) in plan.rungs.iter().zip(chose.rungs.iter()) {
                assert_eq!(
                    rung.reasons.contains(&Reason::FilledAGap),
                    chosen.filled_a_gap,
                    "the mark and the reason disagree on {} Mbit/s (anchor {anchor},                      hardness {hardness})",
                    chosen.bitrate_mbps
                );
            }
        }
    }

    assert!(
        ever_patched,
        "not one ladder in the whole sweep was patched, so this checked nothing at all —          either the filling stopped happening or the sweep stopped reaching it"
    );
}

#[test]
fn a_hole_the_grid_itself_left_is_still_shown() {
    // FR-152, written into the spec today (T391). A residual hole is not a fault in the
    // choosing: the grid's own multipliers leave a pair wider than twofold before any choosing
    // happens — `grid_bitrates_mbps(8)` is 1, 3, 5, 8, 12, and three over one is threefold.
    // Nothing can be put between them, because nothing between them was measured, and
    // inventing a rung would mean claiming a quality nobody looked at.
    //
    // **So it has to be visible.** A viewer whose connection falls in that gap gets a rung
    // markedly worse than the one they could hold, and saying nothing about it would leave
    // that as a mystery. The objection is what says it, and this is the check that the
    // objection survives — the patching must not swallow the one it cannot fix.
    use vrcast_studio_lib::domain::ladder::Objection;

    let facts = source(1920, 1080, 24, None);
    let mut ever_objected = false;

    for anchor in [8u64, 13, 19, 25, 36] {
        for hardness in [20.0, 40.0, 60.0, 80.0, 100.0] {
            let measured: Vec<Point> = grid_bitrates_mbps(anchor)
                .into_iter()
                .map(|b| point(b, 1080, curve(b, hardness)))
                .collect();
            let chose = select(&measured, TARGET_VMAF, VMAF_STEP);
            let Ok(plan) = from_measurement(&chose.rungs, &facts, None, false) else {
                continue;
            };
            if validate(&plan.rungs, &facts, facts.fps)
                .iter()
                .any(|o| matches!(o, Objection::BadStep { .. }))
            {
                ever_objected = true;
            }
        }
    }

    assert!(
        ever_objected,
        "not one of the anchors the grid cannot fix produced an objection, so either the \
         grid stopped leaving holes — in which case FR-152 and R-32 are settled and this \
         should say so — or the objection stopped being raised and the hole is now silent"
    );
}
