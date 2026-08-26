//! T179–T184 — working out a ladder of qualities for a particular source.
//!
//! **Every rule here is carried over from `.claude/skills/vrcast-convert/scripts/plan-ladder.sh`
//! without changing the arithmetic** (constitution, principle VI). Each was bought with a
//! measurement or a mistake in this project, and reinventing one repeats it. Changing any
//! of them takes a new measurement, not an argument.
//!
//! **What this is and is not.** It is the fast path: a probe measures how many bits the
//! material asks for, and the rungs follow from that. It is an approximation, and the
//! project knows where it misses — `measure-ladder.sh` finds the real saturation point by
//! measuring VMAF over a grid of bitrates and resolutions, takes half an hour, and is the
//! policy for every film. This decides where to start from, not what the answer is.
//!
//! **On the variety of material.** Frame rate is not a special case here and needs none:
//! it goes into the compatibility level by way of macroblocks per second, and into the bit
//! density by way of pixels per second, so 48- and 60-frame material is placed by the same
//! rules that place 24. Animation is not a special case either — the probe encodes the
//! actual material, and flat drawings ask for fewer bits than dense live action all by
//! themselves. Stereoscopic video **is** recognised, and only to be reported: the numbers
//! for it would need a measurement, and there has not been one.

use serde::{Deserialize, Serialize};

/// How far apart the rungs are, going down from the top.
///
/// A step of roughly 1.8×. Closer together and the rungs are indistinguishable duplicates;
/// further apart and a hole opens for a weak connection to fall into.
const MULTIPLIERS: [f64; 4] = [1.0, 0.55, 0.3, 0.17];

/// A megabit per second, in bits.
///
/// **The unit the whole of this works in, and that is load bearing.** `plan-ladder.sh`
/// counts in whole megabits from beginning to end — the source's bitrate (`S=$(( SBPS /
/// 1000000 ))`), the anchor (`ANCHOR=$(( psum / pn / 1000000 ))`), its floor (`ANCHOR < 1`
/// means below one **megabit**), and every rung (`max(1, int(round(a*m)))`).
///
/// Ported into bits per second it all still compiles and every number comes out wrong. The
/// floor of one becomes one bit rather than one megabit, so a ladder can be planned with a
/// rung of 238 kbit/s. The rungs stop landing on whole megabits, so they no longer coincide
/// with the grid every VMAF measurement this project owns was taken on. And the collapsing
/// of duplicates stops happening, because at bit precision two multipliers never meet — so
/// light material, which the rule says needs one file and no ladder, gets four.
const MBIT: u64 = 1_000_000;

/// The bit density below which a rung's resolution is worth lowering.
///
/// **0.05 and not 0.10**, and that is a measurement rather than a preference. The old
/// target lowered resolution twice as eagerly as it should: at 22 Mbit/s it gave 1604
/// where full 2160 measured better by 0.32 VMAF, and at 8 Mbit/s it would have given 810 —
/// the worst of everything tried, 4.75 VMAF below the best.
const TARGET_DENSITY: f64 = 0.05;

/// How much more a source in a heavier codec is worth in H.264 terms.
///
/// H.264 needs more bits for the same picture, so a cap taken from an HEVC source's
/// bitrate would cut the ladder off far below where the detail actually runs out.
const HEVC_TO_H264: f64 = 1.6;

/// How far above an upscaled source's native height there is anything left to encode.
///
/// Measured on 2026-08-07 on material upscaled from 1080 to 2160: the best height by VMAF
/// settled at 1728 and **stayed there** at 4, 8 and 14 Mbit/s, while the density formula
/// was calling for 2160 and 1936. 1728/1080 is 1.6.
const UPSCALE_HEADROOM: f64 = 1.6;

/// How the two eyes of a stereoscopic frame are laid out.
///
/// Recognised so that it can be **said**, not so that it can change the arithmetic. A
/// side-by-side frame is 3840 wide and holds 1920 per eye, and a person told "3840×1080"
/// has been told something true and useless. Whether the density rule ought to differ for
/// two nearly identical halves is a fair question and an unanswered one — there has been
/// no measurement, and principle VI does not allow guessing at one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Layout {
    /// One picture.
    Flat,
    /// Two pictures side by side.
    SideBySide,
    /// One above the other.
    OverUnder,
}

impl Layout {
    /// The size of one eye's picture.
    pub fn per_eye(&self, width: u32, height: u32) -> (u32, u32) {
        match self {
            Self::Flat => (width, height),
            Self::SideBySide => (width / 2, height),
            Self::OverUnder => (width, height / 2),
        }
    }
}

/// How the layout was arrived at.
///
/// Told apart because they are worth different amounts. A file that says what it is has
/// been told; proportions are a guess, and a guess shown as knowledge is how a person ends
/// up correcting something that was right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayoutSource {
    /// The file says so.
    Declared,
    /// Worked out from the shape of the frame.
    Guessed,
}

/// What was worked out about the shape of the picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shape {
    pub layout: Layout,
    pub from: LayoutSource,
}

/// Recognise a stereoscopic frame by its proportions.
///
/// Only when the file itself says nothing. The proportions of a flat picture run from
/// about 1.3:1 to about 2.4:1; twice that, or half of it, is not a shape any flat film is
/// made in.
///
/// A guess and marked as one. A very wide flat panorama exists, and so does 3D material
/// cropped to an unusual size; the person is shown what was decided and can say otherwise.
pub fn guess_shape(width: u32, height: u32, declared: Option<Layout>) -> Shape {
    if let Some(layout) = declared {
        return Shape {
            layout,
            from: LayoutSource::Declared,
        };
    }
    if height == 0 || width == 0 {
        return Shape {
            layout: Layout::Flat,
            from: LayoutSource::Guessed,
        };
    }
    let ratio = f64::from(width) / f64::from(height);
    let layout = if ratio >= 2.6 {
        Layout::SideBySide
    } else if ratio <= 1.15 {
        Layout::OverUnder
    } else {
        Layout::Flat
    };
    Shape {
        layout,
        from: LayoutSource::Guessed,
    }
}

/// Why a rung looks the way it does.
///
/// Codes and numbers, never a sentence: the wordings live in the interface's catalogues
/// (T131), and a phrase built here would be stuck in whichever language it was written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// The top rung: this is where the material stopped asking for more.
    ProbedAnchor,
    /// The top was cut down to the source's own bitrate.
    CappedBySource,
    /// The top was cut down to what is left above an upscale.
    CappedByUpscale,
    /// A step down from the one above.
    StepDown,
    /// The material could not be measured, so the old constant was used instead.
    FallbackConstant,
    /// The resolution was lowered because the bits per pixel had fallen too low.
    LoweredForDensity,
    /// The resolution was left alone: the density is sound.
    FullResolution,
    /// The material is too light for a ladder at all.
    SingleRungOnly,
}

/// One rung of a ladder (data model, section 5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rung {
    pub index: usize,
    pub bitrate_bps: u64,
    /// The ceiling on peaks.
    pub maxrate_bps: u64,
    /// **Roughly equal to the ceiling.** A larger buffer lets a surge through above the
    /// ceiling, and that is what froze viewers: it used to be ceiling 45 with buffer 60,
    /// and the peaks came out at 54.
    pub bufsize_bps: u64,
    pub width: u32,
    pub height: u32,
    /// The **actual** level of this variant, by both limits.
    pub level: String,
    /// Why it is as it is, for the interface to put into words.
    pub reasons: Vec<Reason>,
}

/// What is known about the source before the rungs are worked out.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SourceFacts {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// The source's own average bitrate.
    pub bitrate_bps: u64,
    /// Whether the source is in a codec that carries more picture per bit than H.264.
    pub heavier_codec: bool,
    /// The height the material really has, when it was upscaled to its present size.
    ///
    /// Told by the person, not worked out. A round trip through a lower resolution gives no
    /// steady break on such material — 48 to 56 dB at every height — and the threshold
    /// would have to be fitted to each file. Whoever has the file knows; the measurement
    /// does not.
    pub native_height: Option<u32>,
}

/// A ladder, as planned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub rungs: Vec<Rung>,
    /// What the shape of the picture turned out to be.
    pub shape: Shape,
    /// The top of the ladder, in bits per second — always a whole number of megabits.
    pub anchor_bps: u64,
}

/// Why no ladder can be planned at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Refusal {
    /// The source does not amount to a whole megabit per second.
    ///
    /// `plan-ladder.sh` line 70 stops here — `не удалось измерить битрейт источника` — and
    /// stopping is the right answer rather than a formality. Everything downstream counts
    /// in whole megabits, so such a source gives a cap of zero, and a cap of zero is
    /// indistinguishable from "no cap at all": the ladder would be planned as if the source
    /// were unlimited, and every rung would stand above it. A file with 0.9 Mbit/s of
    /// detail in it does not want a ladder; it wants to be served as it is.
    SourceBitrateTooLow { bitrate_bps: u64 },
}

/// What the source allows the top of a ladder to be, in whole megabits per second.
///
/// The script's own arithmetic, integer division and all: a 12 Mbit/s HEVC source gives
/// `12 * 16 / 10` = **19**, not 19.2. The rounding is not a detail to be tidied up — it is
/// what puts the cap on the same whole-megabit grid as the rungs and the measurements.
pub fn source_cap_mbps(source: &SourceFacts) -> u64 {
    let s = source.bitrate_bps / MBIT;
    if source.heavier_codec {
        s * 16 / 10
    } else {
        s
    }
}

/// The constant the ladder used before the probe existed, in megabits per second.
///
/// Still the fallback when the probe cannot run. A ladder built on it is worse than one
/// built on a measurement and far better than none.
pub const FALLBACK_MBPS: u64 = 35;

/// The top of the ladder: where the material stopped asking for more, brought inside what
/// the source allows.
///
/// `measured_bps` is what the probe found, or `None` when it could not run. Both paths are
/// here, in the rules, rather than in the layer that runs ffmpeg: the fallback is a rule —
/// **the constant capped by what the source allows**, and that cap carries the heavier-codec
/// allowance with it. Left in the probe, it took the source's own bitrate instead and cut
/// every ladder over an HEVC master by a third.
fn top_rung(measured_bps: Option<u64>, source: &SourceFacts) -> (u64, Vec<Reason>) {
    // Never zero: [`plan`] refuses before this is reached, which is what makes the cap safe
    // to apply unconditionally — exactly as the script's line 128 is safe because its line
    // 70 has already stopped.
    let cap = source_cap_mbps(source);

    let Some(measured_bps) = measured_bps else {
        // `ANCHOR=$(( SCAP < 35 ? SCAP : 35 ))` — the constant, held down by the cap.
        return (cap.clamp(1, FALLBACK_MBPS), vec![Reason::FallbackConstant]);
    };

    let mut reasons = vec![Reason::ProbedAnchor];
    // Whole megabits, as the probe's own line does it. Truncated, not rounded.
    let mut top = (measured_bps / MBIT).max(1);
    if top > cap {
        top = cap;
        reasons.push(Reason::CappedBySource);
    }
    (top.max(1), reasons)
}

/// The rungs, in whole megabits per second, going down from the top.
///
/// Rungs that come out the same are folded together, and that folding is a rule rather than
/// tidiness: on light material the multipliers land on the same whole megabit, and the
/// ladder shrinks to two rungs or to one. One rung is the script's signal that this material
/// does not want a ladder at all — `# ВНИМАНИЕ: источник слабый → одна ступень, ABR-лесенка
/// не нужна`.
///
/// The floor is **one megabit**, not one of anything smaller. Below that there is no
/// quality worth serving, and the ladder would be planning rungs no measurement this project
/// owns has ever looked at.
fn steps_from(top_mbps: u64) -> Vec<u64> {
    let mut out: Vec<u64> = Vec::new();
    for m in MULTIPLIERS {
        // **Halves go to the even number, not away from zero.** The rungs have always
        // been produced by Python's `round`, which breaks a tie towards even; Rust's
        // `round` breaks it away from zero. They disagree on eight anchors between 1 and
        // 100 — and one of them is 35, the constant this falls back on, where the script
        // gives a rung of 10 and away-from-zero gives 11. Ten is a point this project has
        // measured and named files after; eleven is not.
        let mbps = ((top_mbps as f64 * m).round_ties_even() as u64).max(1);
        if !out.contains(&mbps) {
            out.push(mbps);
        }
    }
    out
}

/// How many bits fall on each pixel of each frame.
pub fn density(bitrate_bps: u64, width: u32, height: u32, fps: u32) -> f64 {
    let pixels = f64::from(width) * f64::from(height) * f64::from(fps.max(1));
    if pixels <= 0.0 {
        return 0.0;
    }
    bitrate_bps as f64 / pixels
}

/// The height a rung is worth encoding at.
///
/// Lowered only when the density has fallen below the target, and only as far as brings it
/// back to about the target. Heights stay even: an odd one is not a size the encoder will
/// take.
///
/// The ceiling from an upscale is applied afterwards and separately: above it the picture
/// is interpolation, and bits spent there buy nothing at any density.
fn height_for(bitrate_bps: u64, source: &SourceFacts) -> (u32, Vec<Reason>) {
    let mut reasons = Vec::new();
    let full = source.height;
    let mut height = full;

    let d = density(bitrate_bps, source.width, source.height, source.fps);
    if d < TARGET_DENSITY && d > 0.0 {
        let lowered = (f64::from(full) * (d / TARGET_DENSITY).sqrt()) as u32;
        height = lowered - (lowered % 2);
        reasons.push(Reason::LoweredForDensity);
    }

    if let Some(native) = source.native_height {
        let ceiling = {
            let c = (f64::from(native) * UPSCALE_HEADROOM) as u32;
            (c - (c % 2)).min(full)
        };
        if height > ceiling {
            height = ceiling;
            reasons.push(Reason::CappedByUpscale);
        }
    }

    if height >= full {
        height = full;
        if reasons.is_empty() {
            reasons.push(Reason::FullResolution);
        }
    }
    (height.max(2), reasons)
}

/// The width that goes with a height, keeping the picture's proportions.
///
/// Even, and worked out from the source's own proportions. That is what carries a
/// stereoscopic frame through unharmed: both eyes are scaled together, and neither the
/// split nor the geometry moves.
pub fn width_for(height: u32, source: &SourceFacts) -> u32 {
    if source.height == 0 {
        return source.width;
    }
    let w = (u64::from(source.width) * u64::from(height) / u64::from(source.height)) as u32;
    w - (w % 2)
}

/// Work out a ladder.
///
/// `measured_bps` is what the complexity probe found, or `None` when it could not run.
///
/// Refuses on a source that does not reach a whole megabit — see [`Refusal`]. That refusal
/// is what lets everything below treat the cap as a real number instead of a maybe.
pub fn plan(
    measured_bps: Option<u64>,
    source: &SourceFacts,
    declared: Option<Layout>,
) -> Result<Plan, Refusal> {
    if source_cap_mbps(source) == 0 {
        return Err(Refusal::SourceBitrateTooLow {
            bitrate_bps: source.bitrate_bps,
        });
    }
    let shape = guess_shape(source.width, source.height, declared);
    let (top_mbps, top_reasons) = top_rung(measured_bps, source);
    let steps = steps_from(top_mbps);
    let single = steps.len() == 1;

    let rungs = steps
        .into_iter()
        .map(|mbps| mbps * MBIT)
        .enumerate()
        .map(|(index, bitrate_bps)| {
            let (height, mut reasons) = height_for(bitrate_bps, source);
            let width = width_for(height, source);
            let (maxrate_bps, bufsize_bps) = peak_control(bitrate_bps);

            if index == 0 {
                let mut first = top_reasons.clone();
                first.append(&mut reasons);
                reasons = first;
                if single {
                    reasons.push(Reason::SingleRungOnly);
                }
            } else {
                reasons.insert(0, Reason::StepDown);
            }

            Rung {
                index,
                bitrate_bps,
                maxrate_bps,
                bufsize_bps,
                width,
                height,
                // The **actual** level of this variant. A fixed one — 5.2 was what the
                // scripts used to write — cuts the lowest rung off from weak devices, that
                // is, from exactly the people a ladder is built for.
                level: super::convert_plan::h264_level(width, height, source.fps).to_owned(),
                reasons,
            }
        })
        .collect();

    Ok(Plan {
        rungs,
        shape,
        anchor_bps: top_mbps * MBIT,
    })
}

/// The ceiling and the buffer for a rung, in bits per second.
///
/// The same arithmetic as for preparing a single file, and deliberately the same function:
/// two copies of this rule would drift, and the drift would show up as viewers freezing on
/// one path and not the other.
fn peak_control(bitrate_bps: u64) -> (u64, u64) {
    let (maxrate_kbps, bufsize_kbps) =
        super::convert_plan::peak_control((bitrate_bps / 1000).max(1) as u32);
    (
        u64::from(maxrate_kbps) * 1000,
        u64::from(bufsize_kbps) * 1000,
    )
}

// ---------- checking a ladder a person has edited (T184) ----------

/// How far apart neighbouring rungs are allowed to be.
///
/// Closer than one and a half and they are duplicates a viewer cannot tell apart; further
/// than double and a connection that cannot hold the upper one drops all the way past the
/// lower.
const MIN_STEP: f64 = 1.5;
const MAX_STEP: f64 = 2.0;

/// How much larger than the ceiling the buffer may be before peaks get through.
const BUFSIZE_OVER_MAXRATE: f64 = 1.1;

/// What is wrong with a rung.
///
/// Every objection names the rung it is about: a person editing the third rung wants to be
/// told about the third rung, not about "the ladder".
// Not `Eq`: one of the objections carries how many times apart two rungs are, and that is
// a fraction. Rounding it to fit an equality nobody needs would throw away the very number
// the person is being shown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Objection {
    /// Above the source there is no more detail to be had, only weight.
    RungAboveSource { index: usize, source_bps: u64 },
    /// The buffer is larger than the ceiling by enough to let peaks through.
    ///
    /// The recorded case: ceiling 45 with buffer 60 produced peaks of 54, and viewers
    /// froze on them.
    BufsizeTooLarge { index: usize, maxrate_bps: u64 },
    /// The variant does not fit the level written on it, and here is which limit it breaks.
    LevelExceeded {
        index: usize,
        level: String,
        limits: Vec<super::convert_plan::LevelLimit>,
    },
    /// The rungs are not in descending order.
    OutOfOrder { index: usize },
    /// Two rungs are too close together to tell apart, or too far apart to bridge.
    BadStep { index: usize, times: f64 },
}

/// Whether two neighbouring rungs are acceptably far apart.
///
/// **The grid has to be allowed for, or this objects to ladders the project's own script
/// produces.** At an anchor of 15 the script gives 15, 8, 4, 3 — and 4 over 3 is 1.33,
/// outside the stated range of one and a half to two. That ladder is not wrong: 4 is simply
/// the nearest whole megabit to 4.5, and near the bottom of a ladder half a megabit is a
/// sixth of the value.
///
/// So the range is widened by exactly the rounding that produced it — half a megabit — and
/// **only for rungs that are on the grid.** A value off the grid was not produced by that
/// rounding and gets no allowance for it; that is what keeps a hand-edited 1.1 against 1.0
/// from passing as a step.
///
/// The earlier form of this check gave each rung its own half-megabit and then read the two
/// bounds from two contradictory worlds. It let a threefold hole through — 3 against 1,
/// which is the very failure the rule exists for — and could not fire at all below two and
/// a half megabits.
fn step_is_allowable(above_bps: u64, below_bps: u64) -> bool {
    let above = above_bps as f64;
    let below = below_bps as f64;

    if above_bps % MBIT != 0 || below_bps % MBIT != 0 {
        let times = above / below;
        return (MIN_STEP..=MAX_STEP).contains(&times);
    }

    let half = MBIT as f64 / 2.0;
    above >= MIN_STEP * below - half && above <= MAX_STEP * below + half
}

/// Check a ladder, whoever wrote it.
///
/// **Every** objection comes back, not the first: an edited ladder often has several, and a
/// person shown one at a time has to go round the loop once per objection.
pub fn validate(rungs: &[Rung], source: &SourceFacts, fps: u32) -> Vec<Objection> {
    let mut out = Vec::new();

    // Above the source, in the same terms the top was capped in — otherwise a perfectly
    // sound ladder over an HEVC source would be objected to on every rung.
    let cap = if source.heavier_codec {
        (source.bitrate_bps as f64 * HEVC_TO_H264) as u64
    } else {
        source.bitrate_bps
    };

    for (i, rung) in rungs.iter().enumerate() {
        if cap > 0 && rung.bitrate_bps > cap {
            out.push(Objection::RungAboveSource {
                index: i,
                source_bps: cap,
            });
        }
        if rung.bufsize_bps as f64 > rung.maxrate_bps as f64 * BUFSIZE_OVER_MAXRATE {
            out.push(Objection::BufsizeTooLarge {
                index: i,
                maxrate_bps: rung.maxrate_bps,
            });
        }
        let limits = super::convert_plan::level_exceeded(&rung.level, rung.width, rung.height, fps);
        if !limits.is_empty() {
            out.push(Objection::LevelExceeded {
                index: i,
                level: rung.level.clone(),
                limits,
            });
        }

        if let Some(above) = i.checked_sub(1).and_then(|j| rungs.get(j)) {
            if rung.bitrate_bps >= above.bitrate_bps {
                out.push(Objection::OutOfOrder { index: i });
            } else if rung.bitrate_bps > 0 {
                let times = above.bitrate_bps as f64 / rung.bitrate_bps as f64;
                if !step_is_allowable(above.bitrate_bps, rung.bitrate_bps) {
                    out.push(Objection::BadStep { index: i, times });
                }
            }
        }
    }
    out
}
