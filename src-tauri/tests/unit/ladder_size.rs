//! T405, T406 — the room a quality set needs, before a byte of it is made.
//!
//! Every number here is arithmetic anybody can redo on paper, on purpose: the estimate's
//! whole job is to be trusted enough to refuse hours of work, and one that cannot be checked
//! by hand is one nobody will believe when it does refuse.

use vrcast_studio_lib::domain::ladder_size::{
    bytes_for_rung, bytes_for_set, AUDIO_BUDGET_BPS, SEGMENTS_OVER_MP4,
};

/// Bytes as gigabytes — the decimal kind, which is how disks are sold and how the figure
/// this phase was argued from was worked out. A gibibyte is seven per cent smaller, and
/// mixing the two silently is how an estimate acquires a seven per cent error nobody put
/// there on purpose.
fn gb(bytes: u64) -> f64 {
    bytes as f64 / 1_000_000_000.0
}

#[test]
fn one_variant_is_its_bitrate_times_its_length_twice_over() {
    // 4 Mbit/s of picture and 256 kbit/s of sound over a hundred seconds is 53 200 000 bytes
    // nominal. On the server that is an MP4 and the segments cut from it: 53 200 000 x 2.046.
    // 4 256 000 bit/s over 100 s is 53 200 000 bytes nominal; times 2.046 is 108 847 200.
    // Written out rather than recomputed here: a test that redoes the implementation's own
    // arithmetic agrees with it whatever it does.
    //
    // **And one byte more than that**, because `1.0 + 1.046` in binary floating point lands a
    // hair above 2.046 and the result is rounded up. Three bytes on a set of three rungs,
    // against gigabytes — and upward, which is the direction this whole module is built to
    // err in. Written down rather than rounded away: a number nobody can account for is a
    // number nobody will trust when it refuses an hour of work.
    assert_eq!(
        bytes_for_rung(4_000_000, AUDIO_BUDGET_BPS, 100.0),
        108_847_201
    );
}

#[test]
fn a_season_sized_set_lands_where_it_was_reckoned_to() {
    // The number that made this phase exist: eight episodes of forty-five minutes in three
    // rungs. If this comes out a great deal smaller, the estimate has stopped describing
    // what a set does to a disk.
    let episode = bytes_for_set(
        &[22_000_000, 10_000_000, 4_000_000],
        AUDIO_BUDGET_BPS,
        45.0 * 60.0,
    );
    let season = episode * 8;
    assert!(
        (195.0..210.0).contains(&gb(season)),
        "a season came out at {:.0} GB, and the reckoning behind this phase was about 200",
        gb(season)
    );
}

#[test]
fn a_copied_audio_track_is_counted_at_what_it_actually_weighs() {
    // A copied track can be far heavier than the budget a re-encoded one is held to — a
    // multichannel source runs to 1.5 Mbit/s and more. Counting it at 256 kbit/s would
    // under-count every set built from such a source, which is the one direction that
    // matters.
    let budget = bytes_for_rung(4_000_000, AUDIO_BUDGET_BPS, 100.0);
    let fat = bytes_for_rung(4_000_000, 1_536_000, 100.0);
    assert!(fat > budget, "a heavier track did not make the set heavier");
}

#[test]
fn nothing_to_build_needs_no_room() {
    assert_eq!(bytes_for_set(&[], AUDIO_BUDGET_BPS, 3600.0), 0);
    assert_eq!(bytes_for_rung(4_000_000, AUDIO_BUDGET_BPS, 0.0), 0);
}

#[test]
fn a_source_of_unknown_length_asks_for_nothing_rather_than_for_a_guess() {
    // `duration_s` arrives from the probe and can be zero or negative on a file whose header
    // does not say. Nought is the honest answer: the check that reads it must then say the
    // room could not be worked out, rather than be handed a number that came from nowhere.
    assert_eq!(bytes_for_rung(4_000_000, AUDIO_BUDGET_BPS, -1.0), 0);
}

/// **The number most likely to be tuned down by somebody watching a check refuse.**
///
/// It is a reading, not a preference: 1.0457 at 1.5 Mbit/s and 1.0296 at 5, with the bundled
/// FFmpeg on 2026-08-28. The larger is taken because the overhead grows as a share on the
/// light rungs, and a ladder's bottom is where the light rungs are. Lowered, every set is
/// reckoned smaller than it is, and the refusal this feeds lets through exactly the builds
/// that do not fit.
///
/// Guarded at **build** time rather than in a test body. Clippy was right that comparing a
/// constant is decided before anything runs — so the right place for it is the compiler,
/// where lowering the number stops being a red test and becomes a build that will not
/// finish.
const _: () = assert!(SEGMENTS_OVER_MP4 >= 1.046);

#[test]
fn the_estimate_never_comes_out_under_the_nominal_bytes() {
    // The property that matters more than accuracy. Being told there is room and running out
    // anyway is the failure this exists to prevent; being told there is no room when there
    // just about was costs a person one look at the number.
    for bitrate in [1_000_000u64, 4_000_000, 22_000_000, 60_000_000] {
        for seconds in [10.0f64, 600.0, 7200.0] {
            let nominal = (bitrate + AUDIO_BUDGET_BPS) as f64 * seconds / 8.0;
            let got = bytes_for_rung(bitrate, AUDIO_BUDGET_BPS, seconds);
            assert!(
                got as f64 >= nominal * 2.0,
                "{bitrate} bit/s over {seconds} s was reckoned at {got}, under the \
                 {nominal} x 2 that is on the disk before any overhead at all"
            );
        }
    }
}

/// T693 — what «On the server ≈» says is the set alone: the prepared files are removed once
/// the set is checked. The room asked for before a build is still both.
#[test]
fn a_checked_set_leaves_its_segments_alone_on_the_server() {
    use vrcast_studio_lib::domain::ladder_size::served_bytes_for_set;
    let rungs = [9_000_000u64, 4_000_000];
    let served = served_bytes_for_set(&rungs, AUDIO_BUDGET_BPS, 3600.0);
    let peak = bytes_for_set(&rungs, AUDIO_BUDGET_BPS, 3600.0);
    let nominal: f64 = rungs
        .iter()
        .map(|b| (b + AUDIO_BUDGET_BPS) as f64 * 3600.0 / 8.0)
        .sum();
    assert!(served as f64 >= nominal * SEGMENTS_OVER_MP4 - 2.0);
    assert!(served < peak);
    assert!(peak as f64 >= nominal * 2.0);
    assert_eq!(served_bytes_for_set(&rungs, AUDIO_BUDGET_BPS, 0.0), 0);
}

fn source_with(
    track: vrcast_studio_lib::domain::source::AudioTrack,
) -> vrcast_studio_lib::domain::source::SourceFile {
    vrcast_studio_lib::domain::source::SourceFile {
        path: String::from("F:/films/film.mkv"),
        size_bytes: 40_000_000_000,
        duration_s: 7200.0,
        width: 1920,
        height: 1080,
        fps: 24,
        bitrate_bps: 30_000_000,
        peak_bps: None,
        video_codec: String::from("h264"),
        pix_fmt: String::from("yuv420p"),
        color_transfer: None,
        audio_tracks: vec![track],
    }
}

fn track(
    codec: &str,
    channels: u16,
    bitrate_bps: Option<u64>,
) -> vrcast_studio_lib::domain::source::AudioTrack {
    vrcast_studio_lib::domain::source::AudioTrack {
        index: 0,
        codec: codec.to_owned(),
        profile: Some(String::from("LC")),
        channels,
        bitrate_bps,
        language: None,
        title: None,
        is_default: true,
    }
}

/// T699 (QA-26 №9) — the sound in every size is the output's: a TrueHD track at 4.6 Mbit/s
/// goes out as AAC stereo at the budget, and was reckoned at 4.6 — nearly twice the room.
#[test]
fn the_sound_is_reckoned_as_it_goes_out_not_as_it_came_in() {
    use vrcast_studio_lib::domain::ladder_size::audio_out_bps;
    assert_eq!(
        audio_out_bps(&source_with(track("truehd", 8, Some(4_600_000))), 0),
        AUDIO_BUDGET_BPS
    );
    assert_eq!(
        audio_out_bps(&source_with(track("aac", 6, Some(640_000))), 0),
        AUDIO_BUDGET_BPS
    );
    // Copied as it is: what it weighs, never under the budget.
    assert_eq!(
        audio_out_bps(&source_with(track("aac", 2, Some(192_000))), 0),
        AUDIO_BUDGET_BPS
    );
    assert_eq!(
        audio_out_bps(&source_with(track("aac", 2, Some(270_000))), 0),
        270_000
    );
    // No such track: the budget, not nothing.
    assert_eq!(
        audio_out_bps(&source_with(track("aac", 2, Some(192_000))), 5),
        AUDIO_BUDGET_BPS
    );
}

/// T699 — this computer holds one prepared file at a time, the heaviest, and no segments:
/// those are cut on the server. Reckoned at the rung's ceiling, never under the file.
#[test]
fn this_computer_is_asked_for_the_heaviest_prepared_file_alone() {
    use vrcast_studio_lib::domain::ladder_size::local_peak_bytes;
    let ceilings = [9_900_000u64, 4_400_000, 1_100_000];
    let here = local_peak_bytes(&ceilings, AUDIO_BUDGET_BPS, 7200.0);
    let file = (9_900_000 + AUDIO_BUDGET_BPS) as f64 * 7200.0 / 8.0;
    assert_eq!(here, file.ceil() as u64);
    // Not the old reckoning of the file plus its segments.
    assert!(here < bytes_for_rung(9_000_000, AUDIO_BUDGET_BPS, 7200.0));
    assert_eq!(local_peak_bytes(&[], AUDIO_BUDGET_BPS, 7200.0), 0);
    assert_eq!(local_peak_bytes(&ceilings, AUDIO_BUDGET_BPS, 0.0), 0);
}
