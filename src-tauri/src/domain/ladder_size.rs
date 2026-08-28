//! T405 — how much room a quality set needs, worked out before anything is encoded.
//!
//! **Why before.** Building a set is hours of encoding and gigabytes of transfer, and until
//! now nothing asked the server whether any of it would fit: `ladder_build` checked that the
//! rungs were measured and started. A season of eight episodes in three rungs is around two
//! hundred gigabytes, and the way that ends today is a transfer failing somewhere in the
//! middle of the fifth episode, hours in, with variants of the first four already served and
//! the fifth half written.
//!
//! **The number this produces is deliberately not clever.** It is bitrate times duration,
//! doubled, and it exists to answer one question — will this obviously not fit — rather than
//! to predict the size of the result. An estimate that is close but occasionally low is worse
//! than one that is crude and never low: being told "there is room" and running out anyway is
//! the failure this is meant to prevent, not a smaller version of it.
//!
//! ## Why doubled
//!
//! A set leaves **two** copies of every variant on the server. The prepared MP4 stays in the
//! serving directory for good — `tidy_up` removes the cutting script and its log, nothing
//! else — and the HLS segments sit beside it in `{slug}/v{mbit}/`.
//!
//! The segments are slightly heavier than the MP4 they came from, because MPEG-TS carries a
//! header every 188 bytes where MP4 carries an index once. Measured on 2026-08-28 with the
//! bundled FFmpeg, `-c copy` from a freshly encoded clip:
//!
//! | Nominal | MP4 | Segments | Segments / MP4 |
//! |---|---|---|---|
//! | 5000k + 256k | 38 928 267 | 40 081 078 | 1.0296 |
//! | 1500k + 256k | 13 005 849 | 13 600 150 | 1.0457 |
//!
//! The lighter the variant, the heavier the overhead as a share — the header count follows
//! bytes, not seconds. So the **larger** of the two ratios is the one taken: a ladder's
//! bottom rungs are the light ones, and rounding towards the lighter measurement would
//! under-count exactly where the overhead is worst.
//!
//! Playlists are not counted: about forty bytes a segment, some seventy kilobytes for a
//! two-hour set, against gigabytes.

/// What the segments weigh against the MP4 they were cut from.
///
/// The heavier of the two measurements above. Held down at build time in
/// `tests/unit/ladder_size.rs` (T406) so that it cannot be quietly lowered to a figure
/// nobody measured.
pub const SEGMENTS_OVER_MP4: f64 = 1.046;

/// The audio budget a re-encoded track is held to, in bits per second.
///
/// The same 256 kbit/s `convert_plan` gives it. A copied track can be heavier, and the
/// caller passes what it actually is.
pub const AUDIO_BUDGET_BPS: u64 = 256_000;

/// What one variant leaves on the server, in bytes.
///
/// `audio_bps` is what the audio will actually weigh: [`AUDIO_BUDGET_BPS`] when it is being
/// re-encoded, the source track's own bitrate when it is copied.
pub fn bytes_for_rung(bitrate_bps: u64, audio_bps: u64, duration_s: f64) -> u64 {
    if duration_s <= 0.0 {
        return 0;
    }
    let nominal = (bitrate_bps + audio_bps) as f64 * duration_s / 8.0;
    // The MP4 that stays, plus the segments cut from it.
    (nominal * (1.0 + SEGMENTS_OVER_MP4)).ceil() as u64
}

/// What a whole set leaves on the server, in bytes.
pub fn bytes_for_set(bitrates_bps: &[u64], audio_bps: u64, duration_s: f64) -> u64 {
    bitrates_bps
        .iter()
        .map(|b| bytes_for_rung(*b, audio_bps, duration_s))
        .sum()
}
