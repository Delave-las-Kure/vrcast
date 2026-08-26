//! T185, T186 — the description of a quality set: what is written in it, and reading it
//! back.
//!
//! **The figures are the variants' own figures** (FR-046). Everything here is worked out
//! from the segments that exist, not from what the ladder asked for: an encoder does not
//! deliver exactly what it was told, and a description that repeats the request describes
//! something that was never made.
//!
//! Carried over from `.claude/skills/vrcast-hls/scripts/upgrade-masters.py` without
//! changing the arithmetic (constitution, principle VI).

use serde::{Deserialize, Serialize};

/// One segment, as it lies on disk.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub duration_s: f64,
    pub bytes: u64,
}

/// The peak of a variant: the heaviest second of it.
///
/// **The tail stub does not count.** The last segment is often a fragment — four hundredths
/// of a second happens — and a fragment's bytes over a fragment's duration comes out as a
/// bitrate no part of the film ever had. It gave a fictitious 51 Mbit/s on a real ladder,
/// and every viewer's player then reserved a channel for a peak that did not exist.
/// Segments shorter than half the longest are left out of the reckoning.
pub fn peak_bps(segments: &[Segment]) -> u64 {
    let longest = segments
        .iter()
        .map(|s| s.duration_s)
        .fold(0.0_f64, f64::max);
    if longest <= 0.0 {
        return 0;
    }
    segments
        .iter()
        .filter(|s| s.duration_s >= longest * 0.5 && s.duration_s > 0.0)
        .map(|s| (s.bytes as f64 * 8.0 / s.duration_s) as u64)
        .max()
        .unwrap_or(0)
}

/// The average over the whole variant.
///
/// Every segment counts here, tail included: it is part of the film, and its bytes are
/// bytes a viewer downloads.
pub fn average_bps(segments: &[Segment]) -> u64 {
    let total_bytes: u64 = segments.iter().map(|s| s.bytes).sum();
    let total_time: f64 = segments.iter().map(|s| s.duration_s).sum();
    if total_time <= 0.0 {
        return 0;
    }
    (total_bytes as f64 * 8.0 / total_time) as u64
}

/// One variant, as the description names it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Variant {
    /// Where its playlist is, as written in the description.
    pub path: String,
    /// The peak. A player sizes a connection by this, not by the average.
    pub bandwidth: u64,
    pub average_bandwidth: u64,
    pub width: u32,
    pub height: u32,
    pub fps: Option<f64>,
    /// The codecs string, carrying the variant's **actual** level.
    pub codecs: String,
}

/// The codecs string for a variant at a given H.264 level.
///
/// **The real level, never a fixed one.** A player decides whether it can play a variant at
/// all from this: a constant `avc1.640034` — level 5.2 — on the lowest rung cuts that rung
/// off from weak devices, which are exactly the devices a ladder is built for. The level
/// goes in as two hexadecimal digits, so 4.1 becomes 29 and 5.2 becomes 34.
pub fn codecs_for(level: &str) -> String {
    let idc = level_idc(level);
    format!("avc1.6400{idc:02X},mp4a.40.2")
}

/// A level's numeric form: "4.1" is 41, "5.2" is 52.
fn level_idc(level: &str) -> u8 {
    let digits: String = level.chars().filter(char::is_ascii_digit).collect();
    match digits.parse::<u8>() {
        // A one-digit level means a whole number: "5" is 50.
        Ok(n) if digits.len() == 1 => n * 10,
        Ok(n) => n,
        // An unreadable level becomes the highest there is. Overstating a level is always
        // safe; understating it is what a strict decoder is entitled to refuse.
        Err(_) => 52,
    }
}

/// Write the description of a quality set.
pub fn build(variants: &[Variant]) -> String {
    let mut out = String::from("#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-INDEPENDENT-SEGMENTS\n");
    for v in variants {
        out.push_str("#EXT-X-STREAM-INF:");
        out.push_str(&format!("BANDWIDTH={}", v.bandwidth));
        out.push_str(&format!(",AVERAGE-BANDWIDTH={}", v.average_bandwidth));
        out.push_str(&format!(",RESOLUTION={}x{}", v.width, v.height));
        if let Some(fps) = v.fps {
            out.push_str(&format!(",FRAME-RATE={fps:.3}"));
        }
        out.push_str(&format!(",CODECS=\"{}\"", v.codecs));
        // Said outright rather than left out: a player that has to guess whether there are
        // subtitles goes looking for them, and the looking costs a round trip before
        // anything plays.
        out.push_str(",CLOSED-CAPTIONS=NONE\n");
        out.push_str(&v.path);
        out.push('\n');
    }
    out
}

/// Why a description could not be read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MasterProblem {
    /// Not a playlist at all.
    NotAPlaylist,
    /// A variant was declared and no path followed it.
    VariantWithoutPath { line: usize },
    /// A variant carries no bandwidth, which is the one figure a player cannot do without.
    VariantWithoutBandwidth { line: usize },
}

/// Read a description back into its variants.
///
/// Needed twice over: to check that what was published says what it should (FR-046), and to
/// make the shortened description a limited viewer is handed (Phase 6) out of the full one.
pub fn parse(text: &str) -> Result<Vec<Variant>, MasterProblem> {
    let lines: Vec<&str> = text.lines().collect();
    if !lines.iter().any(|l| l.trim() == "#EXTM3U") {
        return Err(MasterProblem::NotAPlaylist);
    }

    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let Some(attrs) = line.trim().strip_prefix("#EXT-X-STREAM-INF:") else {
            continue;
        };
        // The path is the next line that is neither blank nor a tag. Blank lines between
        // the two are legal and do turn up in files written by hand.
        let path = lines[i + 1..]
            .iter()
            .map(|l| l.trim())
            .find(|l| !l.is_empty())
            .filter(|l| !l.starts_with('#'))
            .ok_or(MasterProblem::VariantWithoutPath { line: i + 1 })?;

        let bandwidth = number(attrs, "BANDWIDTH")
            .ok_or(MasterProblem::VariantWithoutBandwidth { line: i + 1 })?;
        let (width, height) = resolution(attrs).unwrap_or((0, 0));

        out.push(Variant {
            path: (*path).to_owned(),
            bandwidth,
            // Absent means "the same as the peak" rather than zero: a description written
            // by an older tool carries only BANDWIDTH, and calling its average zero would
            // make every such variant look free.
            average_bandwidth: number(attrs, "AVERAGE-BANDWIDTH").unwrap_or(bandwidth),
            width,
            height,
            fps: attribute(attrs, "FRAME-RATE").and_then(|v| v.parse().ok()),
            codecs: attribute(attrs, "CODECS")
                .map(|v| v.trim_matches('"').to_owned())
                .unwrap_or_default(),
        });
    }
    Ok(out)
}

/// One attribute out of an `#EXT-X-STREAM-INF` line.
///
/// Split on commas outside quotes: `CODECS="avc1.640029,mp4a.40.2"` holds a comma of its
/// own, and splitting on every comma would cut it in half and lose the audio codec.
fn attribute<'a>(attrs: &'a str, key: &str) -> Option<&'a str> {
    let mut in_quotes = false;
    let mut start = 0;
    let mut parts = Vec::new();
    for (i, c) in attrs.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                parts.push(&attrs[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&attrs[start..]);

    parts.iter().find_map(|p| {
        let (name, value) = p.split_once('=')?;
        (name.trim() == key).then(|| value.trim())
    })
}

fn number(attrs: &str, key: &str) -> Option<u64> {
    attribute(attrs, key)?.parse().ok()
}

fn resolution(attrs: &str) -> Option<(u32, u32)> {
    let value = attribute(attrs, "RESOLUTION")?;
    let (w, h) = value.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?))
}
