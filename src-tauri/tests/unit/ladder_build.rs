//! T194, T195 — what a rung needs before it can be cut, and the keyframe rule.

use vrcast_studio_lib::domain::convert_plan::VideoAction;
use vrcast_studio_lib::domain::ladder::{Quality, Rung};
use vrcast_studio_lib::domain::ladder_build::{
    file_name, keyframes_line_up, shared_gop, sub_name, work_for,
};
use vrcast_studio_lib::domain::source::{AudioTrack, SourceFile};
use vrcast_studio_lib::domain::wording::DetailCode;
use vrcast_studio_lib::media::keyframes::from_times;

fn source(width: u32, height: u32, fps: u32, bitrate_bps: u64, codec: &str) -> SourceFile {
    SourceFile {
        path: String::from("F:/films/film.mp4"),
        size_bytes: 4_000_000_000,
        duration_s: 3600.0,
        width,
        height,
        fps,
        bitrate_bps,
        peak_bps: None,
        video_codec: codec.to_owned(),
        pix_fmt: String::from("yuv420p"),
        color_transfer: None,
        audio_tracks: vec![AudioTrack {
            index: 0,
            codec: String::from("aac"),
            channels: 2,
            bitrate_bps: Some(192_000),
            language: None,
            title: None,
            is_default: true,
        }],
    }
}

fn rung(index: usize, bitrate_bps: u64, height: u32) -> Rung {
    Rung {
        index,
        bitrate_bps,
        maxrate_bps: bitrate_bps * 11 / 10,
        bufsize_bps: bitrate_bps * 11 / 10,
        width: height * 16 / 9,
        height,
        level: String::from("5.1"),
        reasons: Vec::new(),
        quality: Quality::MeasuredHere { vmaf_x100: 9500 },
    }
}

// ---------- names ----------

#[test]
fn a_variant_is_named_by_its_whole_megabits_as_everything_here_always_has_been() {
    // A person looking at `v22` beside `film_22.mp4` knows they belong together without
    // being told, and every file this project has ever made is named this way.
    assert_eq!(sub_name(&rung(0, 22_000_000, 2160)), "v22");
    assert_eq!(file_name("film", &rung(0, 22_000_000, 2160)), "film_22.mp4");
    // Never a zero: a rung under a megabit cannot exist, but if one did it would collide
    // with every other such rung in the same directory.
    assert_eq!(sub_name(&rung(0, 400_000, 480)), "v1");
}

// ---------- the keyframe rule ----------

#[test]
fn keyframes_line_up_when_their_spacing_divides_a_segment() {
    // Not "the same number as ours": a source with a keyframe every second and segments of
    // four seconds line up perfectly well, and demanding equality would re-encode it for
    // nothing.
    assert!(keyframes_line_up(1.0, 24, 4));
    assert!(keyframes_line_up(2.0, 24, 4));
    assert!(keyframes_line_up(4.0, 24, 4));
    // 23.976 frames a second gives a keyframe every 1.001 s. That is not one second and is
    // nevertheless exactly right: the variants encoded from this source get the same.
    assert!(keyframes_line_up(1.001, 24, 4));
    assert!(keyframes_line_up(2.002, 24, 4));
}

#[test]
fn keyframes_that_do_not_divide_a_segment_do_not_line_up() {
    // Five seconds against four: the boundaries agree once every twenty seconds and
    // disagree the rest of the time. A viewer changing quality then waits for the next
    // point at which the two meet, and what they see is a stall.
    assert!(!keyframes_line_up(5.0, 24, 4));
    assert!(!keyframes_line_up(3.0, 24, 4));
    assert!(!keyframes_line_up(10.0, 24, 4));

    // **The one that looks fine and is not.** Twenty-three frames between keyframes is
    // 0.958 s — within a frame of a second, and a whole second out after twenty-four
    // intervals. Counting in seconds with a frame of slack would have let this through,
    // and the fault would have shown up as an occasional stutter nobody could reproduce.
    assert!(!keyframes_line_up(23.0 / 24.0, 24, 4));

    // Nonsense is not "probably fine".
    assert!(!keyframes_line_up(0.0, 24, 4));
    assert!(!keyframes_line_up(-1.0, 24, 4));
    assert!(!keyframes_line_up(f64::NAN, 24, 4));
    assert!(!keyframes_line_up(1.0, 0, 4));
}

#[test]
fn the_spacing_is_the_middle_gap_rather_than_the_average() {
    // A film regularly has an extra keyframe at a hard cut. One such quarter-second gap
    // drags an average down far enough to make a stream look finer-grained than it is —
    // and that is the direction that ends in a copy which should never have been allowed.
    let with_a_cut = "0.0\n2.0\n4.0\n4.25\n6.0\n8.0\n10.0\n";
    let spacing = from_times(with_a_cut).expect("nothing was worked out");
    assert!(
        (spacing - 2.0).abs() < 0.01,
        "the odd gap at the cut moved the answer to {spacing}"
    );

    // One keyframe says nothing about spacing, and nothing is the honest answer.
    assert_eq!(from_times("0.0\n"), None);
    assert_eq!(from_times(""), None);
    assert_eq!(from_times("not a number\n"), None);
}

// ---------- what each rung needs ----------

#[test]
fn every_variant_gets_the_same_keyframe_spacing_and_it_follows_the_frame_rate() {
    // A constant would be wrong twice over: 48 means "once a second" on 48-frame material
    // and "once every two" on 24-frame, and two rungs given different numbers stop agreeing
    // about where a segment may begin.
    assert_eq!(shared_gop(&source(3840, 2160, 24, 60_000_000, "h264")), 24);
    assert_eq!(shared_gop(&source(3840, 2160, 48, 60_000_000, "h264")), 48);

    let src = source(3840, 2160, 24, 60_000_000, "h264");
    let work = work_for(
        "film",
        &[
            rung(0, 22_000_000, 2160),
            rung(1, 12_000_000, 1440),
            rung(2, 6_000_000, 1080),
        ],
        &src,
        0,
        Some(1.0),
        4,
    );
    assert_eq!(work.len(), 3);
    let spacings: Vec<u32> = work.iter().map(|w| w.plan.gop).collect();
    assert_eq!(
        spacings,
        vec![24, 24, 24],
        "the variants were given different keyframe spacings and their segments will not meet"
    );
}

#[test]
fn a_rung_that_needs_no_change_of_quality_is_carried_across_untouched() {
    // FR-045. Minutes instead of hours, and no loss at all.
    let src = source(3840, 2160, 24, 22_000_000, "h264");
    let work = work_for("film", &[rung(0, 22_000_000, 2160)], &src, 0, Some(1.0), 4);
    assert_eq!(work[0].plan.video, VideoAction::Copy);
    assert!(work[0].lossless);
    assert!(work[0].notices.is_empty());
}

#[test]
fn a_copy_is_taken_away_when_the_keyframes_would_not_line_up_and_the_person_is_told() {
    // The one place a copy is refused for a reason that has nothing to do with quality.
    // "This rung will take hours after all" is not something to discover from a progress
    // bar.
    let src = source(3840, 2160, 24, 22_000_000, "h264");
    let work = work_for("film", &[rung(0, 22_000_000, 2160)], &src, 0, Some(5.0), 4);

    assert_ne!(
        work[0].plan.video,
        VideoAction::Copy,
        "a stream whose keyframes sit in the wrong places was carried across anyway"
    );
    assert!(!work[0].lossless);
    assert_eq!(
        work[0].notices.iter().map(|n| n.key).collect::<Vec<_>>(),
        vec![DetailCode::NoticeReencodedForKeyframes],
        "the rung is being re-encoded and nothing says why"
    );
}

#[test]
fn not_knowing_where_the_keyframes_are_is_not_permission_to_copy() {
    // Guessing that they line up is the one guess in a ladder that a viewer pays for.
    let src = source(3840, 2160, 24, 22_000_000, "h264");
    let work = work_for("film", &[rung(0, 22_000_000, 2160)], &src, 0, None, 4);
    assert_ne!(work[0].plan.video, VideoAction::Copy);
}

#[test]
fn a_lower_rung_is_re_encoded_because_its_quality_really_does_change() {
    // Nothing to do with keyframes: half the height and a quarter of the bitrate cannot be
    // carried across whatever the keyframes do.
    let src = source(3840, 2160, 24, 22_000_000, "h264");
    let work = work_for("film", &[rung(0, 6_000_000, 1080)], &src, 0, Some(1.0), 4);
    assert_ne!(work[0].plan.video, VideoAction::Copy);
    assert!(
        work[0].notices.is_empty(),
        "a rung re-encoded for its quality was blamed on the keyframes"
    );
}
