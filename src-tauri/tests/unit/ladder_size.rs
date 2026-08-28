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
