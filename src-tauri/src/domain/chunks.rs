//! T229 — the three chunks a quality measurement is taken on (FR-142).
//!
//! **Why three and not one.** A film is not uniformly hard. Measure a quiet dialogue and
//! every rung looks generous; measure the one battle scene and every rung looks starved.
//! The measurement takes a light, a middling and a heavy chunk — the tenth, fiftieth and
//! ninetieth percentile by weight — so that a single untypical scene cannot decide the
//! ladder for the whole film.
//!
//! **Why weight and not position.** Positions are a guess about where a film is hard.
//! Weight is a reading of where it actually is: the packets say so.
//!
//! Ported from `measure-ladder.sh` without changing the arithmetic (constitution VI).

/// How long one chunk is.
///
/// Ten seconds is the script's `DUR`. Long enough to hold a cut or two, short enough that
/// the whole grid — fifteen points on three chunks — stays inside half an hour.
pub const CHUNK_S: usize = 10;

/// Seconds skipped at the start, and at the end.
///
/// Opening and closing titles are still frames over a soundtrack: nearly free to encode and
/// nothing like the film. Left in, they would win the "light" percentile every time and the
/// bottom rung would be chosen on material that never appears again.
///
/// The two numbers differ because the script's do; they were arrived at by looking at real
/// files, and there is no measurement that says they should be equal.
pub const HEAD_GUARD_S: usize = 180;
pub const TAIL_GUARD_S: usize = 190;

/// The percentiles taken, in the order they are reported: light, middling, heavy.
pub const PERCENTILES: [f64; 3] = [0.10, 0.50, 0.90];

/// Where the three chunks start, in seconds from the beginning of the film.
///
/// `second_bytes` is what each second of the video stream weighed, from the first second to
/// the last, with silent seconds present as zero — see [`crate::media::measure`].
///
/// **The guards are dropped rather than honoured when they leave nothing.** A twenty-minute
/// episode has fewer than 180 + 190 seconds to spare, and a measurement of nothing at all is
/// worse than a measurement that includes the titles.
pub fn reference_chunks(second_bytes: &[u64], chunk_s: usize) -> Vec<u64> {
    let len = second_bytes.len() as i64;
    let chunk = chunk_s as i64;

    let (lo, hi) = {
        let lo = HEAD_GUARD_S as i64;
        let hi = len - chunk - TAIL_GUARD_S as i64;
        if hi <= lo {
            (0, (len - chunk).max(1))
        } else {
            (lo, hi)
        }
    };

    // Every window that fits, by what it weighs. Ties keep the earlier position, which is
    // what a stable sort over positions in order gives.
    let mut windows: Vec<(u64, u64)> = (lo..hi)
        .map(|start| {
            let from = start as usize;
            let weight: u64 = second_bytes.iter().skip(from).take(chunk_s).copied().sum();
            (weight, start as u64)
        })
        .collect();
    windows.sort_by_key(|(weight, start)| (*weight, *start));

    let n = windows.len();
    if n == 0 {
        return vec![0; PERCENTILES.len()];
    }
    PERCENTILES
        .iter()
        .map(|q| windows[((n as f64 * q) as usize).min(n - 1)].1)
        .collect()
}

/// What a film's weight-per-second looks like, in five numbers (T435).
///
/// **The richest signal about the material there is, and it was thrown away every time.**
/// Every measurement reads each packet's size to decide where the light, middling and heavy
/// chunks fall — a full second-by-second weight profile of the film — and then keeps three
/// timestamps out of it. The rest went nowhere.
///
/// It is what would tell one film from another without measuring either. Two episodes of a
/// season have profiles that look alike; an episode and a trailer do not, whatever their
/// codec and frame size agree about. Lending compares eight fields today (T431), and every
/// one of them is a property of the container rather than of the picture.
///
/// **A summary rather than the row itself.** A two-hour film is seven thousand numbers, and
/// keeping them per measurement would make the store grow with the library while answering
/// no question the five below cannot. What is kept is what a comparison would ask for.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Shape {
    /// The middle second. Half the film is lighter than this.
    pub median_bps: u64,
    /// The ninth decile: the weight the heavy scenes sit at.
    pub p90_bps: u64,
    /// The heaviest single second there is.
    pub peak_bps: u64,
    /// How much heavier the peak is than the middle, in hundredths.
    ///
    /// The one number that says what *kind* of film this is rather than how big it is. Flat
    /// material — an interview, an animation with still backgrounds — sits near 100; a film
    /// that alternates between talk and battle runs to several hundred. Two files with the
    /// same average and different ratios do not want the same ladder.
    pub peak_to_median_x100: u64,
    /// Seconds at or above twice the median: the walls a connection has to get over.
    pub walls: u64,
}

/// Work the shape out from a film's weight-per-second.
///
/// `None` for an empty reading — not a shape of noughts, which would compare equal to another
/// empty one and let lending vouch for two films nobody has looked at.
pub fn shape_of(seconds: &[u64]) -> Option<Shape> {
    if seconds.is_empty() {
        return None;
    }
    let mut sorted: Vec<u64> = seconds.to_vec();
    sorted.sort_unstable();

    let median = sorted[sorted.len() / 2];
    // The ninth decile by position, and the last element when the film is short enough that
    // the position lands past the end.
    let p90 = sorted[(sorted.len() * 9 / 10).min(sorted.len() - 1)];
    let peak = *sorted.last().unwrap_or(&0);

    Some(Shape {
        median_bps: median,
        p90_bps: p90,
        peak_bps: peak,
        // Guarded: a film of pure black has a median of nought, and the ratio then says
        // nothing rather than dividing by it.
        // A film of pure black has a median of nought, and the ratio then says nothing
        // rather than dividing by it.
        peak_to_median_x100: peak.saturating_mul(100).checked_div(median).unwrap_or(0),
        walls: seconds
            .iter()
            .filter(|&&s| s >= median.saturating_mul(2))
            .count() as u64,
    })
}

/// How far apart two films are by the shape of their weight, in hundredths.
///
/// **Why this exists, and what it deliberately does not do.** R-46 measured four episodes of
/// one season: identical frame, frame rate, codec and source bitrate, and one of the four
/// scored fourteen VMAF below its neighbours — three times the measurement's own noise
/// (R-45). Every field lending compares was equal on all four. So container equality is not
/// material equality, and that is measured rather than suspected.
///
/// The shape is the one thing recorded that describes the picture rather than the container
/// (T435), and it is the natural place to look. **But nobody has measured what a shape
/// difference means yet** — R-45 and R-46 measured VMAF, not this — so this returns the
/// distance and refuses anything. Judging it would mean inventing a threshold, and an
/// invented threshold in a check is worse than no check: it looks like knowledge.
///
/// What it is for is the person. Shown the numbers beside a loan, somebody who knows their
/// material can see that two films are not alike; shown nothing, they cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShapeGap {
    /// How far the middle second differs, in hundredths of the lighter one.
    pub median_x100: u64,
    /// The same for the ninth decile.
    pub p90_x100: u64,
    /// And for the peak against the middle — the number that says what kind of film it is.
    pub ratio_x100: u64,
}

/// The distance between two shapes, or `None` where either is not known.
///
/// `None` rather than nought: two films nobody has a shape for are not thereby alike, and a
/// nought would say they were.
pub fn shape_gap(a: Option<Shape>, b: Option<Shape>) -> Option<ShapeGap> {
    let (a, b) = (a?, b?);
    Some(ShapeGap {
        median_x100: apart(a.median_bps, b.median_bps),
        p90_x100: apart(a.p90_bps, b.p90_bps),
        ratio_x100: apart(a.peak_to_median_x100, b.peak_to_median_x100),
    })
}

/// How far two numbers are apart, as hundredths of the smaller.
///
/// Of the smaller rather than of either one in particular, so that the answer does not depend
/// on which film is called the donor.
fn apart(a: u64, b: u64) -> u64 {
    let (low, high) = if a <= b { (a, b) } else { (b, a) };
    let difference = high - low;
    difference.saturating_mul(100).checked_div(low).unwrap_or(0)
}
