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
    average_bps, build, codecs_for, parse, peak_bps, MasterProblem, Segment, Variant,
};
use vrcast_studio_lib::domain::ladder::{
    density, plan, validate, Layout, LayoutSource, Objection, Reason, Rung, SourceFacts,
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
    let laid = plan(20_000_000, &source(3840, 1080, 60, 40), None);
    assert_eq!(laid.shape.layout, Layout::SideBySide);
    assert_eq!(laid.shape.from, LayoutSource::Guessed);
    assert_eq!(Layout::SideBySide.per_eye(3840, 1080), (1920, 1080));

    let stacked = plan(20_000_000, &source(1920, 2160, 48, 40), None);
    assert_eq!(stacked.shape.layout, Layout::OverUnder);
    assert_eq!(Layout::OverUnder.per_eye(1920, 2160), (1920, 1080));

    // An ordinary film is not mistaken for either, at any of the rates.
    for fps in [24, 48, 60] {
        assert_eq!(
            plan(20_000_000, &source(3840, 2160, fps, 40), None)
                .shape
                .layout,
            Layout::Flat,
            "ordinary 4K at {fps} frames was taken for stereoscopic"
        );
        assert_eq!(
            plan(20_000_000, &source(1920, 800, fps, 40), None)
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
    let told = plan(20_000_000, &source(3840, 1080, 60, 40), Some(Layout::Flat));
    assert_eq!(told.shape.layout, Layout::Flat);
    assert_eq!(told.shape.from, LayoutSource::Declared);
}

#[test]
fn lowering_the_resolution_keeps_both_eyes_together() {
    // Scaling a stereoscopic frame has to keep its proportions, or the split between the
    // eyes moves and the picture comes apart. A deliberately thin ladder over a heavy
    // side-by-side source, so that a rung really is lowered.
    let laid = plan(4_000_000, &source(3840, 1080, 60, 40), None);
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
    let laid = plan(35_000_000, &source(3840, 2160, 48, 12), None);
    assert_eq!(laid.rungs[0].bitrate_bps, 12_000_000);
    assert!(laid.rungs[0].reasons.contains(&Reason::CappedBySource));
}

#[test]
fn a_heavier_source_codec_buys_the_ladder_more_room() {
    // The same picture needs more bits in H.264, so a cap taken straight from an HEVC
    // source's bitrate would cut the ladder off far below where the detail runs out.
    let mut hevc = source(3840, 2160, 24, 12);
    hevc.heavier_codec = true;
    let laid = plan(35_000_000, &hevc, None);
    assert_eq!(laid.rungs[0].bitrate_bps, 19_200_000, "12 × 1.6");
}

#[test]
fn the_rungs_go_down_at_about_one_and_eight_tenths() {
    let laid = plan(20_000_000, &source(3840, 2160, 24, 60), None);
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
    // Animation on a small source asks for very little, and the multipliers then collide.
    // Two identical rungs are a ladder with a rung missing, not a ladder with an extra one;
    // and a person should be told a ladder is not what this material needs.
    let laid = plan(1, &source(1280, 720, 24, 1), None);
    assert_eq!(laid.rungs.len(), 1);
    assert!(laid.rungs[0].reasons.contains(&Reason::SingleRungOnly));
}

#[test]
fn the_resolution_drops_only_when_the_bits_have_run_thin() {
    // The target is 0.05 and not 0.10, and that is a measurement: the old target gave 1604
    // at 22 Mbit/s where full 2160 measured better, and would have given 810 at 8 — the
    // worst of everything tried.
    let src = source(3840, 2160, 24, 60);

    // 22 Mbit/s over 4K at 24 frames is a density of about 0.11 — full resolution.
    assert!(density(22_000_000, 3840, 2160, 24) > 0.05);
    let laid = plan(22_000_000, &src, None);
    assert_eq!(laid.rungs[0].height, 2160);
    assert!(laid.rungs[0].reasons.contains(&Reason::FullResolution));

    // The same 22 Mbit/s at 60 frames is thinner and does come down. Frame rate deciding
    // the resolution is the whole of why it belongs in the formula.
    let quick = source(3840, 2160, 60, 60);
    assert!(density(22_000_000, 3840, 2160, 60) < 0.05);
    let fast = plan(22_000_000, &quick, None);
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
    let laid = plan(22_000_000, &upscaled, None);

    assert_eq!(laid.rungs[0].height, 1728, "1080 × 1.6");
    assert!(laid.rungs[0].reasons.contains(&Reason::CappedByUpscale));
}

#[test]
fn every_rung_carries_its_own_level_and_it_holds() {
    for (w, h, fps) in [(3840, 2160, 48), (3840, 1080, 60), (1920, 1080, 24)] {
        let laid = plan(20_000_000, &source(w, h, fps, 60), None);
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
        let src = source(w, h, fps, 60);
        let laid = plan(20_000_000, &src, None);
        let objections = validate(&laid.rungs, &src, fps);
        assert!(
            objections.is_empty(),
            "the planner's own ladder for {w}×{h}@{fps} was objected to: {objections:?}"
        );
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
