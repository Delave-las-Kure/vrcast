//! T111 — tests for the preparation's pure logic (US4).
//!
//! The rules checked here were each bought with a mistake in this very project: the
//! compatibility level by two limits, the bitrate ceiling in kilobits, the three conditions
//! for carrying audio across. The reference cases are taken from the project's records
//! rather than invented — an invented case checks a formula against itself.

use vrcast_studio_lib::domain::convert_plan::{
    self as plan, AudioAction, ConvertRequest, PlanProblem, VideoAction,
};
use vrcast_studio_lib::domain::source::{AudioTrack, SourceFile};
use vrcast_studio_lib::domain::wording::DetailCode;

fn track(codec: &str, channels: u16) -> AudioTrack {
    AudioTrack {
        index: 0,
        codec: String::from(codec),
        channels,
        bitrate_bps: None,
        language: Some(String::from("rus")),
        title: None,
        is_default: true,
    }
}

/// A source that is certainly compatible: H.264 yuv420p, AAC stereo audio.
fn compatible() -> SourceFile {
    SourceFile {
        path: String::from("F:/video/film.mp4"),
        size_bytes: 8_000_000_000,
        duration_s: 7200.0,
        width: 1920,
        height: 1080,
        fps: 24,
        bitrate_bps: 9_000_000,
        peak_bps: None,
        video_codec: String::from("h264"),
        pix_fmt: String::from("yuv420p"),
        color_transfer: Some(String::from("bt709")),
        audio_tracks: vec![track("aac", 2)],
    }
}

fn as_is() -> ConvertRequest {
    ConvertRequest {
        audio_track: 0,
        target_kbps: None,
        height: None,
    }
}

// ---------- carrying across against re-encoding (T109, FR-022) ----------

#[test]
fn a_compatible_file_is_not_re_encoded() {
    // The phase's main rule: hours of work against minutes, and a lost generation against
    // none.
    let p = plan::plan(&compatible(), &as_is()).expect("the plan would not be made");
    assert_eq!(p.video, VideoAction::Copy);
    assert_eq!(p.audio, AudioAction::Copy);
    assert!(p.lossless());
}

#[test]
fn hevc_is_re_encoded_even_though_it_is_already_compressed() {
    // The recorded case of 2026-07-30: HEVC saves bitrate, but Windows has no system
    // decoder, and four viewers out of eight could not watch. Copying such a stream is
    // cheaper — which is exactly why the temptation is there.
    let mut src = compatible();
    src.video_codec = String::from("hevc");

    let p = plan::plan(&src, &as_is()).expect("the plan would not be made");
    match p.video {
        VideoAction::Reencode { reason, .. } => {
            // The reason is a code with a substitution rather than a sentence: the
            // interface picks the wording, and it exists in both languages. The codec is
            // named as a value, because "the video is in hevc" without the hevc explains
            // nothing.
            assert_eq!(reason.key, DetailCode::ReasonVideoNotH264);
            assert_eq!(
                reason.params.get("codec").and_then(|v| v.as_str()),
                Some("hevc"),
                "the reason does not name the source's codec: {reason:?}"
            );
        }
        other => panic!("HEVC was carried across without re-encoding: {other:?}"),
    }
}

#[test]
fn ten_bit_h264_is_re_encoded() {
    // Formally the same codec, and a check of "codec == h264" would let it through. A
    // strict decoder will not take it, and that comes to light at a viewer's machine.
    let mut src = compatible();
    src.pix_fmt = String::from("yuv420p10le");

    let p = plan::plan(&src, &as_is()).expect("the plan would not be made");
    assert!(
        matches!(p.video, VideoAction::Reencode { .. }),
        "a ten-bit stream was carried across as it stands: {:?}",
        p.video
    );
}

#[test]
fn high_dynamic_range_needs_tone_mapping_and_therefore_re_encoding() {
    let mut src = compatible();
    src.color_transfer = Some(String::from("smpte2084"));

    let p = plan::plan(&src, &as_is()).expect("the plan would not be made");
    assert!(p.tonemap, "high dynamic range was not recognised");
    assert!(matches!(p.video, VideoAction::Reencode { .. }));
}

#[test]
fn changing_the_frame_size_rules_out_carrying_across() {
    // A stream cannot be copied and the picture changed at the same time: any change needs
    // decoding, and once decoded it cannot be put back the way it was.
    let src = compatible();
    let req = ConvertRequest {
        height: Some(720),
        ..as_is()
    };

    let p = plan::plan(&src, &req).expect("the plan would not be made");
    assert!(matches!(p.video, VideoAction::Reencode { .. }));
}

// ---------- audio (FR-021, FR-024) ----------

#[test]
fn multi_channel_aac_is_not_carried_across_despite_the_codec() {
    // A recorded mistake: a check of the codec alone let a six-channel track through, and
    // given AAC 5.1 on the way in the file went out with six channels against a stereo
    // target.
    let mut src = compatible();
    src.audio_tracks = vec![track("aac", 6)];

    let p = plan::plan(&src, &as_is()).expect("the plan would not be made");
    match p.audio {
        AudioAction::Reencode { reason, .. } => {
            assert_eq!(reason.key, DetailCode::ReasonAudioChannels);
            assert_eq!(
                reason.params.get("channels").and_then(|v| v.as_u64()),
                Some(6),
                "the reason does not say how many channels: {reason:?}"
            );
        }
        other => panic!("a six-channel track was carried across as it stands: {other:?}"),
    }
}

#[test]
fn re_encoding_audio_always_lines_the_drift_up() {
    // FR-024. AAC writes its priming samples through an edit list, and the VRChat player
    // does not read one — the sound drifts. Without this field the plan would be
    // incomplete, and the drift would show up during playback.
    let mut src = compatible();
    src.audio_tracks = vec![track("eac3", 6)];

    let p = plan::plan(&src, &as_is()).expect("the plan would not be made");
    match p.audio {
        AudioAction::Reencode { resample_fix, .. } => {
            assert!(
                resample_fix,
                "the audio alignment was not switched on for a re-encode"
            );
        }
        other => panic!("incompatible audio was carried across as it stands: {other:?}"),
    }
}

#[test]
fn a_slightly_fatter_track_is_carried_across_within_the_allowance() {
    // Real AAC runs a little over its nominal size, consistently: "256k" weighs more than
    // 256,000. Without the allowance the track would go off to be re-encoded, losing a
    // generation for nothing.
    let mut src = compatible();
    let mut t = track("aac", 2);
    t.bitrate_bps = Some(263_000);
    src.audio_tracks = vec![t];

    let p = plan::plan(&src, &as_is()).expect("the plan would not be made");
    assert_eq!(p.audio, AudioAction::Copy);
}

#[test]
fn a_noticeably_fatter_track_is_re_encoded() {
    let mut src = compatible();
    let mut t = track("aac", 2);
    t.bitrate_bps = Some(640_000);
    src.audio_tracks = vec![t];

    let p = plan::plan(&src, &as_is()).expect("the plan would not be made");
    assert!(matches!(p.audio, AudioAction::Reencode { .. }));
}

#[test]
fn a_file_with_no_audio_is_rejected_with_an_objection_of_its_own() {
    let mut src = compatible();
    src.audio_tracks.clear();

    let problems = plan::plan(&src, &as_is()).expect_err("a file with no audio was accepted");
    assert!(problems.contains(&PlanProblem::NoAudioTracks));
    assert_eq!(problems[0].detail().key, DetailCode::PlanNoAudioTracks);
}

#[test]
fn a_track_that_does_not_exist_is_named_in_human_terms() {
    let src = compatible();
    let req = ConvertRequest {
        audio_track: 5,
        ..as_is()
    };

    let problems = plan::plan(&src, &req).expect_err("a track that does not exist was accepted");
    // Numbers are shown to a person from one: "there is no track 0" reads as an error. The
    // conversion is done by the core, once — otherwise every catalogue would have to
    // remember it separately.
    let detail = problems[0].detail();
    assert_eq!(detail.key, DetailCode::PlanNoSuchTrack);
    assert_eq!(
        detail.params.get("number").and_then(|v| v.as_u64()),
        Some(6),
        "the track number is not the one a person sees: {detail:?}"
    );
}

// ---------- the compatibility level (FR-029) ----------

#[test]
fn the_level_is_counted_by_two_limits_rather_than_by_frame_size() {
    // One and the same frame at different rates needs different levels. A check of the size
    // alone would declare 4.1 in both cases, and a strict decoder is entitled to refuse such
    // a file — a class of fault recorded in this project.
    //
    // 1920×1080 is 8160 macroblocks, which fits the 4.1 frame limit (8192). But
    // 8160 × 48 = 391,680 per second against the 4.1 limit of 245,760.
    assert_eq!(
        plan::h264_level(1920, 1080, 24),
        "4.1",
        "at 24 frames the level is overstated"
    );
    assert_eq!(
        plan::h264_level(1920, 1080, 48),
        "4.2",
        "the frame rate was not taken into account: the level is understated, and a strict player may refuse"
    );
}

#[test]
fn a_partial_macroblock_counts_as_a_whole_one() {
    // A macroblock is 16×16, and a partial one takes up room too: 1922 pixels is 121
    // columns, not 120. Rounding down understates the count and the level with it.
    //
    // 1920×1072 is exactly 8040 macroblocks, which fits 4.1 at 24 frames.
    assert_eq!(plan::h264_level(1920, 1072, 24), "4.1");
    // Two pixels more on each side and it is 121×68 = 8228, past the 4.1 limit. Rounding
    // down would still give 8040, and the level would stay as it was.
    assert_eq!(
        plan::h264_level(1922, 1074, 24),
        "4.2",
        "a partial macroblock was not counted: the level is understated"
    );
}

#[test]
fn a_large_frame_gets_a_high_level() {
    // 3840×2160 at 48: 32,400 macroblocks and 1,555,200 per second — past 5.1.
    assert_eq!(plan::h264_level(3840, 2160, 48), "5.2");
    // The same frame at half the rate fits 5.1 (777,600 against a limit of 983,040).
    assert_eq!(plan::h264_level(3840, 2160, 24), "5.1");
}

// ---------- holding the peaks down (T110, FR-025) ----------

#[test]
fn the_ceiling_is_counted_in_kilobits_rather_than_megabits() {
    // A recorded mistake: in megabits the integer 8*11/10 gives exactly 8 — the ceiling
    // equals the target, there is no buffer, and out comes constant bitrate, which lost in
    // the measurements. At the old +30 % this never showed; at +10 % it broke quietly.
    let (maxrate, _) = plan::peak_control(8_000);
    assert_eq!(maxrate, 8_800, "the ceiling was not counted in kilobits");
    assert!(
        maxrate > 8_000,
        "the ceiling equalled the target — there is no buffer at all"
    );
}

#[test]
fn the_buffer_equals_the_ceiling() {
    // A large buffer allows a surge above the ceiling: it used to be "ceiling 45 / buffer
    // 60", with peaks of 54 Mbit/s that froze viewers.
    for target in [4_000u32, 8_000, 22_000, 35_000] {
        let (maxrate, bufsize) = plan::peak_control(target);
        assert_eq!(
            bufsize, maxrate,
            "the buffer diverged from the ceiling at the target {target}"
        );
    }
}

#[test]
fn the_ceiling_never_equals_the_target() {
    // Even at tiny values, where rounding eats the margin.
    for target in 1..=40u32 {
        let (maxrate, _) = plan::peak_control(target);
        assert!(
            maxrate > target,
            "at the target {target} the ceiling came out {maxrate} — that is constant bitrate"
        );
    }
}

#[test]
fn for_a_given_peak_the_ceiling_is_set_below_it() {
    // The real peak comes out 5–6 % above the ceiling: a viewer's connection is sized for
    // the peak rather than for the average, and setting the ceiling equal to the peak means
    // exceeding it.
    let ceiling = plan::maxrate_for_peak(38_000);
    assert!(
        ceiling < 38_000,
        "the ceiling is not below the peak asked for"
    );
    // And back again: a ceiling set that way gives a peak close to the one wanted.
    let expected_peak = ceiling * 106 / 100;
    assert!(
        (37_000..=38_100).contains(&expected_peak),
        "the peak came out {expected_peak} instead of roughly 38,000"
    );
}

#[test]
fn a_given_bitrate_forces_a_re_encode_even_of_a_compatible_stream() {
    // Otherwise the request would go unmet while a person believed it was honoured.
    let src = compatible();
    let req = ConvertRequest {
        target_kbps: Some(6_000),
        ..as_is()
    };

    let p = plan::plan(&src, &req).expect("the plan would not be made");
    match p.video {
        VideoAction::ReencodeCapped {
            target_kbps,
            maxrate_kbps,
            bufsize_kbps,
            ..
        } => {
            assert_eq!(target_kbps, 6_000);
            assert_eq!(maxrate_kbps, 6_600);
            assert_eq!(bufsize_kbps, 6_600);
        }
        other => panic!("the given bitrate was not honoured: {other:?}"),
    }
}

#[test]
fn a_bitrate_noticeably_above_the_source_is_rejected_with_an_explanation() {
    // FR-029: a combination that is plainly senseless does not pass in silence.
    let src = compatible(); // 9 Mbit/s
    let req = ConvertRequest {
        target_kbps: Some(40_000),
        ..as_is()
    };

    let problems = plan::plan(&src, &req).expect_err("a bitrate above the source was accepted");
    let detail = problems[0].detail();
    assert_eq!(detail.key, DetailCode::PlanBitrateAboveSource);
    // Both numbers are named: without them the objection does not say how far above the
    // source the request is, and there is nothing to argue with.
    assert_eq!(
        detail.params.get("asked_kbps").and_then(|v| v.as_u64()),
        Some(40_000)
    );
    assert_eq!(
        detail.params.get("source_kbps").and_then(|v| v.as_u64()),
        Some(9_000)
    );
}

#[test]
fn stretching_the_frame_is_rejected_with_an_explanation() {
    let src = compatible(); // 1080 lines
    let req = ConvertRequest {
        height: Some(2160),
        ..as_is()
    };

    let problems = plan::plan(&src, &req).expect_err("stretching was accepted");
    let detail = problems[0].detail();
    assert_eq!(detail.key, DetailCode::PlanHeightAboveSource);
    assert_eq!(
        detail.params.get("asked").and_then(|v| v.as_u64()),
        Some(2160)
    );
    assert_eq!(
        detail.params.get("source").and_then(|v| v.as_u64()),
        Some(1080)
    );
}

#[test]
fn the_objections_all_come_back_at_once() {
    // There are often several, and dealing with one per round is work that need not
    // exist.
    let mut src = compatible();
    src.audio_tracks.clear();
    let req = ConvertRequest {
        audio_track: 3,
        target_kbps: Some(0),
        height: Some(0),
    };

    let problems = plan::plan(&src, &req).expect_err("a plan was made from an unfit request");
    assert!(
        problems.len() >= 3,
        "only {} objections out of three came back: {problems:?}",
        problems.len()
    );
}

// ---------- keyframes ----------

#[test]
fn a_keyframe_once_a_second_at_any_rate() {
    // A constant here would be a mistake: 48 was written for 48-frame video and meant "once
    // a second", while on 24-frame material it gave one every two.
    for fps in [24u32, 25, 30, 48, 60] {
        let mut src = compatible();
        src.fps = fps;
        let p = plan::plan(&src, &as_is()).unwrap();
        assert_eq!(
            p.gop, fps,
            "at {fps} frames the keyframe is not once a second"
        );
    }
}

// ---------- showing the tracks (FR-020, an edge case) ----------

#[test]
fn the_default_track_is_the_marked_one_rather_than_the_first() {
    let mut src = compatible();
    let mut first = track("aac", 2);
    first.index = 0;
    first.is_default = false;
    let mut second = track("aac", 2);
    second.index = 1;
    second.is_default = true;
    src.audio_tracks = vec![first, second];

    assert_eq!(src.default_track().map(|t| t.index), Some(1));
}
