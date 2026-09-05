//! T187, T188 — the rules a ladder is worked out by.
//!
//! The control cases come from `SERVER.md` and from the measurements recorded in
//! `plan-ladder.sh` and `measure-ladder.sh`. Every one of them was bought with a mistake or
//! a measurement in this project, and a test that let one through would let the mistake back
//! in (constitution, principle VI).
//!
//! **Frame rate and the shape of the picture run through all of it deliberately.** The
//! material this is for is not one kind: 24-frame film, 48- and 60-frame, animation, and
//! stereoscopic video where one frame holds two pictures. Frame rate is not handled by a
//! special case anywhere — it goes into the level by way of macroblocks per second and into
//! the bit density by way of pixels per second — so the checks below use all three rates on
//! purpose, to show that the same rules place all of them.

use vrcast_studio_lib::domain::convert_plan::{h264_level, level_exceeded, LevelLimit};
use vrcast_studio_lib::domain::hls_master::{
    average_bps, build, build_with_provenance, codecs_for, parse, peak_bps, MasterProblem,
    Provenance, Segment, Variant,
};
use vrcast_studio_lib::domain::ladder::{
    density, plan, source_cap_mbps, validate, Layout, LayoutSource, Objection, Reason, Refusal,
    Rung, SourceFacts, FALLBACK_MBPS,
};

fn source(width: u32, height: u32, fps: u32, bitrate_mbps: u64) -> SourceFacts {
    SourceFacts {
        width,
        height,
        fps,
        bitrate_bps: bitrate_mbps * 1_000_000,
        heavier_codec: false,
        native_height: None,
    }
}

// ---------- the level, by both limits ----------

#[test]
fn a_frame_can_fit_a_level_while_the_stream_does_not() {
    // The control case from SERVER.md, and it is 48-frame stereoscopic material — the very
    // kind this is for. 1922×1082 is 8228 macroblocks a frame, which all but fits level
    // 4.1's limit of 8192; per second it is 394,944 against a limit of 245,760, over by
    // 1.6 times. Checking the frame alone would call this 4.1 and a strict player would
    // refuse the file.
    assert_eq!(h264_level(1922, 1082, 48), "4.2");

    // At 24 frames it is still 4.2, and that is worth being precise about: 8228 is over
    // the frame limit by itself. The rate is the second way to miss, not the only one.
    assert_eq!(h264_level(1922, 1082, 24), "4.2");

    // A frame that really does turn on the rate: 1920×1080 is 8160 macroblocks, inside the
    // frame limit, and 8160 × 48 is 391,680 against 245,760.
    assert_eq!(h264_level(1920, 1080, 24), "4.1");
    assert_eq!(h264_level(1920, 1080, 48), "4.2");
    assert_eq!(
        level_exceeded("4.1", 1920, 1080, 48),
        vec![LevelLimit::Second],
        "the frame fits and the stream does not — the case checking frame size alone misses"
    );
    assert!(level_exceeded("4.1", 1920, 1080, 24).is_empty());

    // And when a level is claimed that does not hold, it is said which limit broke.
    assert_eq!(
        level_exceeded("4.1", 1922, 1082, 48),
        vec![LevelLimit::Frame, LevelLimit::Second],
        "at 48 frames both limits go, and a person editing a rung should be told both"
    );
    assert_eq!(
        level_exceeded("4.1", 1922, 1082, 24),
        vec![LevelLimit::Frame]
    );
    assert!(level_exceeded("4.2", 1922, 1082, 48).is_empty());
}

#[test]
fn a_higher_frame_rate_raises_the_level_on_the_same_frame() {
    // 60-frame material is not a special case in the code and must not need one.
    assert_eq!(h264_level(3840, 2160, 24), "5.1");
    assert_eq!(h264_level(3840, 2160, 48), "5.2");
    assert_eq!(h264_level(3840, 2160, 60), "5.2");
    assert_eq!(h264_level(1920, 1080, 60), "4.2");
}

// ---------- the shape of the picture ----------

#[test]
fn a_stereoscopic_frame_is_recognised_and_its_eyes_are_measured() {
    // Told to a person as "3840×1080" this is true and useless: what they have is 1920 per
    // eye. Recognising it changes none of the arithmetic — that would take a measurement
    // nobody has made — but it does change what they are shown.
    let laid = plan(Some(20_000_000), &source(3840, 1080, 60, 40), None)
        .expect("a sound source was refused");
    assert_eq!(laid.shape.layout, Layout::SideBySide);
    assert_eq!(laid.shape.from, LayoutSource::Guessed);
    assert_eq!(Layout::SideBySide.per_eye(3840, 1080), (1920, 1080));

    let stacked = plan(Some(20_000_000), &source(1920, 2160, 48, 40), None)
        .expect("a sound source was refused");
    assert_eq!(stacked.shape.layout, Layout::OverUnder);
    assert_eq!(Layout::OverUnder.per_eye(1920, 2160), (1920, 1080));

    // An ordinary film is not mistaken for either, at any of the rates.
    for fps in [24, 48, 60] {
        assert_eq!(
            plan(Some(20_000_000), &source(3840, 2160, fps, 40), None)
                .expect("a sound source was refused")
                .shape
                .layout,
            Layout::Flat,
            "ordinary 4K at {fps} frames was taken for stereoscopic"
        );
        assert_eq!(
            plan(Some(20_000_000), &source(1920, 800, fps, 40), None)
                .expect("a sound source was refused")
                .shape
                .layout,
            Layout::Flat,
            "a wide cinema frame at {fps} frames was taken for stereoscopic"
        );
    }
}

#[test]
fn what_the_file_says_beats_what_its_proportions_suggest() {
    // A guess shown as knowledge is how a person ends up correcting something that was
    // right. A very wide flat panorama exists; if the file says it is flat, it is flat.
    let told = plan(
        Some(20_000_000),
        &source(3840, 1080, 60, 40),
        Some(Layout::Flat),
    )
    .expect("a sound source was refused");
    assert_eq!(told.shape.layout, Layout::Flat);
    assert_eq!(told.shape.from, LayoutSource::Declared);
}

#[test]
fn lowering_the_resolution_keeps_both_eyes_together() {
    // Scaling a stereoscopic frame has to keep its proportions, or the split between the
    // eyes moves and the picture comes apart. A deliberately thin ladder over a heavy
    // side-by-side source, so that a rung really is lowered.
    let laid = plan(Some(4_000_000), &source(3840, 1080, 60, 40), None)
        .expect("a sound source was refused");
    for rung in &laid.rungs {
        let expected = (3840 * rung.height) / 1080;
        assert_eq!(
            rung.width,
            expected - (expected % 2),
            "rung {} moved the split between the eyes: {}×{}",
            rung.index,
            rung.width,
            rung.height
        );
    }
}

// ---------- the rungs ----------

#[test]
fn the_top_never_goes_above_the_source() {
    // Above the source there is no more detail to find, only weight (FR-042).
    let laid = plan(Some(35_000_000), &source(3840, 2160, 48, 12), None)
        .expect("a sound source was refused");
    assert_eq!(laid.rungs[0].bitrate_bps, 12_000_000);
    assert!(laid.rungs[0].reasons.contains(&Reason::CappedBySource));
}

#[test]
fn a_heavier_source_codec_buys_the_ladder_more_room() {
    // The same picture needs more bits in H.264, so a cap taken straight from an HEVC
    // source's bitrate would cut the ladder off far below where the detail runs out.
    let mut hevc = source(3840, 2160, 24, 12);
    hevc.heavier_codec = true;
    let laid = plan(Some(35_000_000), &hevc, None).expect("a sound source was refused");
    // `SCAP=$(( S * 16 / 10 ))` — integer arithmetic on whole megabits: 12 × 16 / 10 is
    // 192 / 10 is **19**, not 19.2. The truncation is what keeps the cap on the same grid
    // as the rungs and as every measurement this project owns.
    assert_eq!(source_cap_mbps(&hevc), 19);
    assert_eq!(laid.rungs[0].bitrate_bps, 19_000_000, "12 × 16 / 10");
}

#[test]
fn the_rungs_go_down_at_about_one_and_eight_tenths() {
    let laid = plan(Some(20_000_000), &source(3840, 2160, 24, 60), None)
        .expect("a sound source was refused");
    assert_eq!(laid.rungs.len(), 4);
    for pair in laid.rungs.windows(2) {
        let times = pair[0].bitrate_bps as f64 / pair[1].bitrate_bps as f64;
        assert!(
            (1.5..=2.0).contains(&times),
            "rungs {} and {} are {times:.2} times apart",
            pair[0].index,
            pair[1].index
        );
    }
}

#[test]
fn light_material_gets_one_rung_and_is_told_so() {
    // A fifteen-minute animation episode at 1.5 Mbit/s, probed at 1.4 — an input somebody
    // could actually have. The anchor truncates to 1 Mbit/s, and every multiplier then
    // lands on the same whole megabit, so the ladder folds to one rung and says so.
    //
    // This test used to be given an anchor of **one bit per second** — the only input in
    // the whole domain where the rule still fired once the port had moved off the megabit
    // grid. It passed, and the rule was gone.
    let laid =
        plan(Some(1_400_000), &source(1280, 720, 24, 2), None).expect("a sound source was refused");
    assert_eq!(
        laid.rungs.len(),
        1,
        "light material got a ladder where the rule says it wants one file: {:?}",
        laid.rungs
    );
    assert!(laid.rungs[0].reasons.contains(&Reason::SingleRungOnly));
    assert_eq!(laid.rungs[0].bitrate_bps, 1_000_000);
}

#[test]
fn every_rung_lands_on_a_whole_megabit() {
    // The grid is not decoration. Every VMAF measurement this project owns was taken at
    // whole megabits, every prepared file is named by one (`film_22.mp4`), and a rung at
    // 8,891,666 bit/s can be compared to none of them.
    for anchor_mbps in [1u64, 2, 3, 5, 8, 14, 15, 16, 22, 35] {
        let laid = plan(
            Some(anchor_mbps * 1_000_000 + 666_666),
            &source(3840, 2160, 24, 60),
            None,
        )
        .expect("a sound source was refused");
        for rung in &laid.rungs {
            assert_eq!(
                rung.bitrate_bps % 1_000_000,
                0,
                "anchor {anchor_mbps}: rung {} is {} bit/s, off the grid",
                rung.index,
                rung.bitrate_bps
            );
            assert!(
                rung.bitrate_bps >= 1_000_000,
                "anchor {anchor_mbps}: rung {} fell below a megabit at {} bit/s — no \
                 quality worth serving lives down there",
                rung.index,
                rung.bitrate_bps
            );
        }
    }
}

#[test]
fn without_a_measurement_the_constant_is_held_down_by_what_the_source_allows() {
    // `ANCHOR=$(( SCAP < 35 ? SCAP : 35 ))`. The cap is SCAP — which already carries the
    // heavier-codec allowance — and not the source's own bitrate. Taking the latter cut
    // every ladder over an HEVC master by a third, and the person was shown the top rung
    // as "this is where the material stopped asking for more" when nothing had been asked.
    let mut hevc = source(3840, 2160, 24, 12);
    hevc.heavier_codec = true;
    let laid = plan(None, &hevc, None).expect("a sound source was refused");
    assert_eq!(
        laid.rungs[0].bitrate_bps, 19_000_000,
        "min(35, 12 × 16 / 10)"
    );
    assert!(laid.rungs[0].reasons.contains(&Reason::FallbackConstant));
    assert!(
        !laid.rungs[0].reasons.contains(&Reason::ProbedAnchor),
        "an unprobed ladder claimed the material had been asked"
    );

    // And a source heavier than the constant is held to the constant.
    let heavy = source(3840, 2160, 24, 80);
    assert_eq!(
        plan(None, &heavy, None)
            .expect("a sound source was refused")
            .rungs[0]
            .bitrate_bps,
        FALLBACK_MBPS * 1_000_000
    );
}

#[test]
fn the_resolution_drops_only_when_the_bits_have_run_thin() {
    // The target is 0.05 and not 0.10, and that is a measurement: the old target gave 1604
    // at 22 Mbit/s where full 2160 measured better, and would have given 810 at 8 — the
    // worst of everything tried.
    let src = source(3840, 2160, 24, 60);

    // 22 Mbit/s over 4K at 24 frames is a density of about 0.11 — full resolution.
    assert!(density(22_000_000, 3840, 2160, 24) > 0.05);
    let laid = plan(Some(22_000_000), &src, None).expect("a sound source was refused");
    assert_eq!(laid.rungs[0].height, 2160);
    assert!(laid.rungs[0].reasons.contains(&Reason::FullResolution));

    // The same 22 Mbit/s at 60 frames is thinner and does come down. Frame rate deciding
    // the resolution is the whole of why it belongs in the formula.
    let quick = source(3840, 2160, 60, 60);
    assert!(density(22_000_000, 3840, 2160, 60) < 0.05);
    let fast = plan(Some(22_000_000), &quick, None).expect("a sound source was refused");
    assert!(
        fast.rungs[0].height < 2160,
        "at 60 frames the same bitrate is spread thinner and the height should come down"
    );
    assert!(fast.rungs[0].reasons.contains(&Reason::LoweredForDensity));
    assert_eq!(
        fast.rungs[0].height % 2,
        0,
        "an odd height is not a size an encoder takes"
    );
}

#[test]
fn an_upscaled_source_is_not_encoded_above_what_it_really_has() {
    // Measured on 2026-08-07: on material upscaled from 1080 to 2160 the best height by
    // VMAF settled at 1728 and stayed there at 4, 8 and 14 Mbit/s, while the density
    // formula was calling for 2160.
    let mut upscaled = source(3840, 2160, 24, 60);
    upscaled.native_height = Some(1080);
    let laid = plan(Some(22_000_000), &upscaled, None).expect("a sound source was refused");

    assert_eq!(laid.rungs[0].height, 1728, "1080 × 1.6");
    assert!(laid.rungs[0].reasons.contains(&Reason::CappedByUpscale));
}

#[test]
fn every_rung_carries_its_own_level_and_it_holds() {
    for (w, h, fps) in [(3840, 2160, 48), (3840, 1080, 60), (1920, 1080, 24)] {
        let laid = plan(Some(20_000_000), &source(w, h, fps, 60), None)
            .expect("a sound source was refused");
        for rung in &laid.rungs {
            assert!(
                level_exceeded(&rung.level, rung.width, rung.height, fps).is_empty(),
                "rung {} of {w}×{h}@{fps} claims level {} and does not fit it",
                rung.index,
                rung.level
            );
        }
    }
}

// ---------- checking a ladder somebody edited ----------

fn rung(index: usize, mbps: u64, w: u32, h: u32, level: &str) -> Rung {
    Rung {
        index,
        bitrate_bps: mbps * 1_000_000,
        maxrate_bps: mbps * 1_100_000,
        bufsize_bps: mbps * 1_100_000,
        width: w,
        height: h,
        level: level.to_owned(),
        reasons: Vec::new(),
        quality: vrcast_studio_lib::domain::ladder::Quality::NotMeasured,
    }
}

#[test]
fn a_rung_above_the_source_is_objected_to() {
    let objections = validate(
        &[rung(0, 30, 3840, 2160, "5.1")],
        &source(3840, 2160, 24, 12),
        24,
    );
    assert!(objections.iter().any(|o| matches!(
        o,
        Objection::RungAboveSource { index: 0, source_bps } if *source_bps == 12_000_000
    )));
}

#[test]
fn a_buffer_larger_than_the_ceiling_is_objected_to() {
    // The recorded case: ceiling 45 with buffer 60 let peaks of 54 through, and viewers
    // froze on them.
    let mut bad = rung(0, 8, 1920, 1080, "4.1");
    bad.maxrate_bps = 45_000_000;
    bad.bufsize_bps = 60_000_000;
    let objections = validate(&[bad], &source(3840, 2160, 24, 60), 24);
    assert!(objections
        .iter()
        .any(|o| matches!(o, Objection::BufsizeTooLarge { .. })));
}

#[test]
fn every_objection_comes_back_at_once_rather_than_one_at_a_time() {
    // An edited ladder usually has several things wrong with it. A person shown one at a
    // time has to go round the loop once per objection.
    let objections = validate(
        &[
            rung(0, 30, 1922, 1082, "4.1"),
            // Not below the one above at all.
            rung(1, 30, 1922, 1082, "4.1"),
        ],
        &source(1922, 1082, 48, 12),
        48,
    );
    assert!(objections.len() >= 3, "only got {objections:?}");
    assert!(objections
        .iter()
        .any(|o| matches!(o, Objection::RungAboveSource { .. })));
    assert!(objections
        .iter()
        .any(|o| matches!(o, Objection::LevelExceeded { .. })));
    assert!(objections
        .iter()
        .any(|o| matches!(o, Objection::OutOfOrder { index: 1 })));
}

#[test]
fn a_ladder_this_code_planned_has_nothing_wrong_with_it() {
    // The two halves have to agree: a planner that produced ladders its own checker
    // objected to would be shouting at the person about its own work.
    for (w, h, fps) in [(3840, 2160, 24), (3840, 1080, 60), (1920, 1080, 48)] {
        // Every anchor from one to forty, not one comfortable value. At an anchor of 15 the
        // rungs come out 15, 8, 4, 3 — and 4 over 3 is 1.33, outside the stated range of
        // one and a half to two. That ladder is what the project's own script emits, and
        // the checker used to object to it.
        for anchor_mbps in 1..=40u64 {
            let src = source(w, h, fps, 60);
            let laid = plan(Some(anchor_mbps * 1_000_000), &src, None)
                .expect("a sound source was refused");
            let objections = validate(&laid.rungs, &src, fps);
            assert!(
                objections.is_empty(),
                "the planner's own ladder for {w}×{h}@{fps} at anchor {anchor_mbps} was \
                 objected to: {objections:?} — rungs {:?}",
                laid.rungs.iter().map(|r| r.bitrate_bps).collect::<Vec<_>>()
            );
        }
    }
}

// ---------- the description of the set ----------

#[test]
fn the_tail_stub_is_not_a_peak() {
    // A last segment of four hundredths of a second gave a fictitious 51 Mbit/s on a real
    // ladder, and every player then reserved a channel for a peak that did not exist.
    let segments = [
        Segment {
            duration_s: 4.0,
            bytes: 4_000_000,
        },
        Segment {
            duration_s: 4.0,
            bytes: 5_000_000,
        },
        Segment {
            duration_s: 0.04,
            bytes: 250_000,
        },
    ];
    // The stub alone would be 50 Mbit/s.
    assert_eq!(peak_bps(&segments), 10_000_000);
    // The average does count it: those bytes are bytes a viewer downloads. 9,250,000
    // bytes over 8.04 seconds — the tail lengthens the film as well as weighing on it.
    assert_eq!(average_bps(&segments), 9_203_980);
}

#[test]
fn the_level_in_the_codecs_string_is_the_real_one() {
    // A fixed 5.2 on the lowest rung cuts it off from weak devices — that is, from exactly
    // the people a ladder is built for.
    assert_eq!(codecs_for("4.1"), "avc1.640029,mp4a.40.2");
    assert_eq!(codecs_for("4.2"), "avc1.64002A,mp4a.40.2");
    assert_eq!(codecs_for("5.1"), "avc1.640033,mp4a.40.2");
    assert_eq!(codecs_for("5.2"), "avc1.640034,mp4a.40.2");
}

#[test]
fn a_description_reads_back_as_what_was_written() {
    let variants = vec![
        Variant {
            path: String::from("v1/stream.m3u8"),
            bandwidth: 10_000_000,
            average_bandwidth: 8_000_000,
            width: 3840,
            height: 2160,
            fps: Some(47.952),
            codecs: codecs_for("5.2"),
        },
        Variant {
            path: String::from("v2/stream.m3u8"),
            bandwidth: 5_000_000,
            average_bandwidth: 4_000_000,
            width: 1920,
            height: 1080,
            fps: Some(47.952),
            codecs: codecs_for("4.2"),
        },
    ];
    let text = build(&variants);
    let read = parse(&text).expect("what was just written would not read back");

    assert_eq!(read.len(), 2);
    assert_eq!(read[0].bandwidth, 10_000_000);
    assert_eq!(read[0].average_bandwidth, 8_000_000);
    assert_eq!((read[0].width, read[0].height), (3840, 2160));
    // The comma inside the codecs string must not be taken for an attribute separator:
    // splitting on every comma cuts it in half and loses the audio codec.
    assert_eq!(read[0].codecs, "avc1.640034,mp4a.40.2");
    assert_eq!(read[1].path, "v2/stream.m3u8");
}

#[test]
fn a_description_written_by_something_older_still_reads() {
    // Only BANDWIDTH, no average, no resolution. Calling its average zero would make every
    // such variant look free, and a limit built on that would hand out the wrong rungs.
    let text = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=6000000\nv1/stream.m3u8\n";
    let read = parse(text).expect("an older description would not read");
    assert_eq!(read[0].bandwidth, 6_000_000);
    assert_eq!(read[0].average_bandwidth, 6_000_000);
}

#[test]
fn the_absolute_paths_of_a_shortened_description_survive_reading() {
    // The shortened description handed to a limited viewer points at the real directory
    // from the root of the serving (R-14). Losing that on the way through would send a
    // player looking for segments where there are none.
    let text = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=5000000\n/videos/demo/v2/stream.m3u8\n";
    let read = parse(text).expect("a shortened description would not read");
    assert_eq!(read[0].path, "/videos/demo/v2/stream.m3u8");
}

#[test]
fn something_that_is_not_a_playlist_is_refused() {
    assert_eq!(parse("<html>404</html>"), Err(MasterProblem::NotAPlaylist));
    assert_eq!(
        parse("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000\n"),
        Err(MasterProblem::VariantWithoutPath { line: 2 })
    );
}

// ---------- what the second audit found ----------

#[test]
fn halves_go_to_the_even_number_as_the_script_has_always_done() {
    // The rungs have always been produced by Python's `round`, which breaks a tie towards
    // the even number. Rust's `round` breaks it away from zero. They disagree on eight
    // anchors between 1 and 100, and one of them is the constant the ladder falls back on.
    //
    // Checked against the values the shell really emits, worked out by running its own
    // one-liner: `sorted({max(1,int(round(a*m))) for m in (1.0,0.55,0.3,0.17)}, reverse=True)`.
    let src = source(3840, 2160, 24, 200);
    let rungs_at = |anchor_mbps: u64| -> Vec<u64> {
        plan(Some(anchor_mbps * 1_000_000), &src, None)
            .expect("a sound source was refused")
            .rungs
            .iter()
            .map(|r| r.bitrate_bps / 1_000_000)
            .collect()
    };

    for (anchor, expected) in [
        (15u64, vec![15u64, 8, 4, 3]),
        (30, vec![30, 16, 9, 5]),
        (35, vec![35, 19, 10, 6]),
        (50, vec![50, 28, 15, 8]),
        (55, vec![55, 30, 16, 9]),
        (70, vec![70, 38, 21, 12]),
        (75, vec![75, 41, 22, 13]),
        (95, vec![95, 52, 28, 16]),
    ] {
        assert_eq!(
            rungs_at(anchor),
            expected,
            "anchor {anchor}: the rungs are not the ones the script emits"
        );
    }
}

#[test]
fn the_fallback_ladder_is_the_one_the_script_falls_back_to() {
    // The constant is 35, and 35 is among the anchors where the two roundings disagree —
    // so getting this wrong would have shipped an 11 Mbit rung, against a measured 10, on
    // every ladder built when the probe could not run.
    let src = source(3840, 2160, 24, 200);
    let laid = plan(None, &src, None).expect("a sound source was refused");
    assert_eq!(
        laid.rungs
            .iter()
            .map(|r| r.bitrate_bps / 1_000_000)
            .collect::<Vec<_>>(),
        vec![35, 19, 10, 6]
    );
}

#[test]
fn a_source_that_does_not_reach_a_megabit_is_refused_rather_than_laddered() {
    // `plan-ladder.sh` line 70 stops here. Carrying on instead gives a cap of zero, which
    // reads as "no cap at all": the ladder came out at 35, 19, 10, 6 Mbit/s over a source
    // holding 0.9 — thirty-nine times the source — and the same module then objected to
    // every rung of it.
    let thin = source(1280, 720, 24, 0);
    let mut thin = thin;
    thin.bitrate_bps = 900_000;

    assert_eq!(
        plan(None, &thin, None),
        Err(Refusal::SourceBitrateTooLow {
            bitrate_bps: 900_000
        })
    );
    assert_eq!(
        plan(Some(20_000_000), &thin, None),
        Err(Refusal::SourceBitrateTooLow {
            bitrate_bps: 900_000
        }),
        "a measurement does not make an unmeasurable source laddered"
    );

    // One bit more and it is a source like any other: a single 1 Mbit rung.
    thin.bitrate_bps = 1_000_000;
    let laid = plan(Some(20_000_000), &thin, None).expect("a whole megabit was refused");
    assert_eq!(laid.rungs.len(), 1);
    assert_eq!(laid.rungs[0].bitrate_bps, 1_000_000);
}

#[test]
fn the_step_check_still_catches_a_hole_and_a_duplicate() {
    // The allowance for whole-megabit rounding must not swallow the rule it is bending.
    // The earlier form let a threefold hole through — the very failure the rule exists for
    // — and could not fire at all below two and a half megabits.
    let src = source(3840, 2160, 24, 200);

    let objects = |rungs: &[(u64, &str)]| {
        let built: Vec<Rung> = rungs
            .iter()
            .enumerate()
            .map(|(i, (bps, level))| Rung {
                index: i,
                bitrate_bps: *bps,
                maxrate_bps: bps * 11 / 10,
                bufsize_bps: bps * 11 / 10,
                width: 1920,
                height: 1080,
                level: (*level).to_owned(),
                reasons: Vec::new(),
                quality: vrcast_studio_lib::domain::ladder::Quality::NotMeasured,
            })
            .collect();
        validate(&built, &src, 24)
            .iter()
            .any(|o| matches!(o, Objection::BadStep { .. }))
    };

    // A threefold hole: a viewer who cannot hold 3 falls all the way to 1.
    assert!(
        objects(&[(3_000_000, "4.1"), (1_000_000, "4.1")]),
        "a threefold hole passed"
    );
    // Two rungs a tenth apart: an encode nobody can tell from its neighbour.
    assert!(
        objects(&[(1_100_000, "4.1"), (1_000_000, "4.1")]),
        "a duplicate passed"
    );
    assert!(
        objects(&[(2_500_000, "4.1"), (2_400_000, "4.1")]),
        "a duplicate passed"
    );
    assert!(
        objects(&[(20_000_000, "4.1"), (4_000_000, "4.1")]),
        "a fivefold hole passed"
    );

    // And what the script itself emits is still accepted, rounding and all.
    assert!(
        !objects(&[(4_000_000, "4.1"), (3_000_000, "4.1")]),
        "4 over 3 was objected to"
    );
    assert!(
        !objects(&[(2_000_000, "4.1"), (1_000_000, "4.1")]),
        "2 over 1 was objected to"
    );
    assert!(
        !objects(&[(8_000_000, "4.1"), (5_000_000, "4.1")]),
        "8 over 5 was objected to"
    );
}

// ---------- where the rungs came from, in the description (T433) ----------

fn one_variant() -> Vec<Variant> {
    vec![Variant {
        path: String::from("v7/stream.m3u8"),
        bandwidth: 7_400_000,
        average_bandwidth: 6_900_000,
        width: 3840,
        height: 2160,
        fps: Some(24.0),
        codecs: String::from("avc1.640033,mp4a.40.2"),
    }]
}

#[test]
fn the_description_says_whether_anybody_measured_this_film() {
    // The numbers in a master say what each variant weighs and nothing about whether anybody
    // ever looked at it. A ladder from the formula and one from a measurement make the same
    // shape of file, and the difference between them is the whole of FR-141.
    let v = one_variant();
    let measured = build_with_provenance(&v, Provenance::Measured);
    let borrowed = build_with_provenance(&v, Provenance::Borrowed);
    let guessed = build_with_provenance(&v, Provenance::Formula);

    assert_ne!(measured, borrowed);
    assert_ne!(borrowed, guessed);
    assert!(borrowed.contains("lent to this one"));
    assert!(guessed.contains("not measured"));
}

#[test]
fn the_description_never_carries_the_donors_path() {
    // **Why this is a code and not a name.** `borrowed_from` is an absolute path on the
    // person's own machine, and this file is served to every viewer who opens the link.
    // Naming the donor would publish somebody's directory structure to strangers for the sake
    // of a note — and the note is just as true without it. The file's name belongs on the
    // screen at home, where `Borrow` shows it.
    let text = build_with_provenance(&one_variant(), Provenance::Borrowed);
    for leak in ["F:/", "C:/", "/home/", ".mkv", ".mp4"] {
        assert!(
            !text.contains(leak),
            "the description carries something path-shaped ({leak}):\n{text}"
        );
    }
}

#[test]
fn the_note_is_a_comment_every_player_must_ignore() {
    // It costs a viewer nothing only if it is a comment. A line a player tried to read would
    // be a broken playlist, and the whole point of putting it here is that it is free.
    let text = build_with_provenance(&one_variant(), Provenance::Measured);
    let note = text
        .lines()
        .find(|l| l.contains("ladder:"))
        .expect("the note is not there");
    assert!(note.starts_with('#'), "the note is not a comment: {note}");
    assert!(
        !note.starts_with("#EXT"),
        "the note looks like a tag: {note}"
    );

    // And the description still reads back as itself.
    let back = parse(&text).expect("the description with a note would not parse");
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].bandwidth, 7_400_000);
}

#[test]
fn a_description_without_a_note_is_what_it_always_was() {
    // `build` is still what everything that does not know about provenance calls, and it must
    // keep producing exactly the file it did — the parser, the server and every reader of an
    // older set depend on it.
    let plain = build(&one_variant());
    assert!(!plain.contains("ladder:"));
    assert!(plain
        .starts_with("#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-INDEPENDENT-SEGMENTS\n#EXT-X-STREAM-INF:"));
}

// ---------- the cap, on sources that are not whole megabits (2026-09-05) ----------

/// A source whose bitrate is not a round number, which is what real masters look like.
fn source_bps(bitrate_bps: u64, heavier: bool) -> SourceFacts {
    let mut s = source(3840, 2160, 24, 0);
    s.bitrate_bps = bitrate_bps;
    s.heavier_codec = heavier;
    s
}

#[test]
fn the_allowance_is_not_thrown_away_by_rounding_the_source_first() {
    // ⚠ **What this is written about, and why the test beside it could not find it.**
    // `convert.sh` truncated the source to whole megabits, applied the ×1.6, and truncated
    // again: 7.842 Mbit/s became 7, then 11 — where the source is worth 12.5. Twelve per
    // cent of the allowance gone to rounding, on every master whose bitrate is not a whole
    // number of megabits, which is nearly all of them. The script was corrected on
    // 2026-09-05 and this carries that over (principle VI).
    //
    // `a_heavier_source_codec_buys_the_ladder_more_room` above stands on 12 Mbit/s exactly —
    // the one input where both arithmetics agree — so it went on passing while the two
    // drifted a megabit apart. Whole numbers are the natural thing to write a test with and
    // the one thing that cannot show this.
    assert_eq!(
        source_cap_mbps(&source_bps(7_842_000, true)),
        12,
        "7.842 × 1.6 = 12.5"
    );
    assert_eq!(
        source_cap_mbps(&source_bps(4_820_000, true)),
        7,
        "4.820 × 1.6 = 7.7"
    );
    assert_eq!(
        source_cap_mbps(&source_bps(6_330_000, true)),
        10,
        "6.330 × 1.6 = 10.1"
    );
    assert_eq!(
        source_cap_mbps(&source_bps(19_900_000, true)),
        31,
        "19.9 × 1.6 = 31.8"
    );

    // And the whole-megabit cases still answer as they always did: the grid the rungs and
    // every measurement live on has not moved.
    assert_eq!(source_cap_mbps(&source_bps(12_000_000, true)), 19);
    assert_eq!(source_cap_mbps(&source_bps(35_000_000, false)), 35);
}

#[test]
fn a_source_too_light_for_a_ladder_is_refused_by_a_rule_and_not_by_a_rounding() {
    // ⚠ **This is the trap that came with correcting the arithmetic.** The refusal used to
    // be spelled `source_cap_mbps(source) == 0`, which held only because the cap truncated
    // its input to whole megabits: a 0.9 Mbit/s source gave 0, and 0 meant "send them away".
    // Truncate once instead of twice and the same source gives 1 — so an HEVC file between
    // 0.625 and 1 Mbit/s would quietly start receiving a one-rung ladder instead of being
    // told it does not want one. Nothing would have failed; the rule would simply have
    // stopped existing.
    let light = source_bps(900_000, true);
    assert!(
        source_cap_mbps(&light) > 0,
        "the test is built wrong: this source is supposed to survive the allowance"
    );
    assert!(matches!(
        plan(Some(35_000_000), &light, None),
        Err(Refusal::SourceBitrateTooLow { .. })
    ));

    // Just over the line, and it is a ladder again.
    assert!(plan(Some(35_000_000), &source_bps(1_000_000, false), None).is_ok());
}

#[test]
fn the_cap_is_a_cap_and_not_a_target() {
    // `convert.sh` clamps its own result up to 1, because it has to hand the encoder a
    // bitrate to work with. Carrying that clamp across would have deleted the refusal above
    // without touching a line of it: every source would have had a cap of at least 1, and
    // `worth_a_ladder` would never have been consulted. A cap of zero is a real answer here
    // — "less than a megabit survives the allowance" — and it is somebody else's business
    // what to do about it.
    assert_eq!(source_cap_mbps(&source_bps(400_000, false)), 0);
    assert_eq!(source_cap_mbps(&source_bps(0, true)), 0);
}
