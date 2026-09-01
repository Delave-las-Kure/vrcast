//! T455, T456 — where to cut a film so that the pieces can be put back together.
//!
//! **What this is for, and what it is not.** The owner cuts a film into pieces, feeds them to
//! a 3D converter that is not part of this application, and joins what comes back. So the
//! cutting has to be lossless — a piece re-encoded on the way out has been through two
//! generations before the converter even sees it — and the joins have to be invisible.
//!
//! **Both of those follow from one rule: cut only on a keyframe.** A piece that begins in the
//! middle of a group of pictures cannot be copied out, only re-encoded; and a join between two
//! pieces whose timestamps do not meet is a stutter at every seam.
//!
//! **Scene changes are where a seam hides.** A cut in the middle of a shot is visible if
//! anything goes wrong with it; a cut where the picture changes completely is not. Measured on
//! `Blue.Eye.Samurai.S01E04` on 2026-08-28: at a threshold of 0.15, `scdet` found 54 scene
//! changes and **all 54 sat exactly on a keyframe**, to 0.000 s. That is expected rather than
//! lucky — x264 and x265 place a keyframe at a scene change by default — and it is what makes
//! cutting on scene boundaries free on this material.
//!
//! **It is not free on every material**, and that is why nothing here assumes it. A broadcast
//! stream with a fixed group of pictures, NVENC with scene-cut detection turned off, or a file
//! that has already been cut once will have scenes and keyframes in different places. Then the
//! choice is between an inexact cut and a re-encode, and it belongs to the person — so a
//! boundary that did not land on a keyframe is reported, never quietly moved.

/// A place to cut, and how well it landed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cut {
    /// Where to cut, in seconds from the start.
    pub at_s: f64,
    /// The scene change this was chosen for, when it was chosen for one.
    ///
    /// `None` means no scene change fell anywhere near the wanted length and the nearest
    /// keyframe was taken instead — an honest cut, in the middle of a shot.
    pub scene_s: Option<f64>,
    /// How far the cut is from the scene change it was chosen for, in seconds.
    ///
    /// Nought on the material this was measured on. Anything above a frame is the finding
    /// T456 exists to surface: the scenes and the keyframes of this file do not agree, and
    /// cutting here will either shift the picture or cost a re-encode.
    pub off_by_s: f64,
}

/// How far from a scene change a cut may sit before it stops being that scene's cut.
///
/// One frame at 24 per second, rounded up. Below this the difference is not visible and not
/// worth a word; above it, the file's scenes and keyframes disagree and somebody has to decide
/// what to do about it.
pub const A_FRAME_S: f64 = 1.0 / 24.0;

/// The window around the wanted length that a scene change may be taken from.
///
/// **Approximate on purpose.** The owner asks for pieces of about so many seconds; taking the
/// strongest scene change within a third either way buys a seam nobody can see, at the cost of
/// pieces that are not all the same length — which nothing downstream cares about.
pub const WINDOW: (f64, f64) = (0.7, 1.3);

/// Choose where to cut.
///
/// `scenes` are scene changes with their strengths, `keyframes` the times a keyframe sits at,
/// both in seconds and both sorted. `target_s` is the length asked for.
///
/// **Walking forward from the last cut rather than dividing the film up.** Dividing would put
/// every boundary at a fixed multiple of the target and then look for a scene near each; one
/// piece coming out long would push every later boundary off its scene. Walking forward means
/// each piece is measured from where the last one really ended.
pub fn choose(
    scenes: &[(f64, f64)],
    keyframes: &[f64],
    target_s: f64,
    duration_s: f64,
) -> Vec<Cut> {
    let mut cuts = Vec::new();
    if target_s <= 0.0 || duration_s <= 0.0 || keyframes.is_empty() {
        return cuts;
    }

    let mut from = 0.0_f64;
    loop {
        let want = from + target_s;
        // The last piece is whatever is left. Cutting again within a window of the end would
        // leave a scrap of a few seconds, which is a piece to send through a converter and
        // get back for no reason.
        if want >= duration_s - target_s * WINDOW.0 {
            break;
        }

        let low = from + target_s * WINDOW.0;
        let high = from + target_s * WINDOW.1;

        // The strongest scene change in the window: the one where the picture changes most is
        // the one where a seam is least visible.
        let strongest = scenes
            .iter()
            .filter(|(at, _)| *at > low && *at < high)
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .copied();

        let cut = match strongest {
            Some((scene_at, _)) => {
                let key = nearest(keyframes, scene_at);
                Cut {
                    at_s: key,
                    scene_s: Some(scene_at),
                    off_by_s: (key - scene_at).abs(),
                }
            }
            // Nothing to hide the seam behind. The keyframe nearest the wanted length is the
            // honest answer: an exact cut in the middle of a shot.
            None => Cut {
                at_s: nearest(keyframes, want),
                scene_s: None,
                off_by_s: 0.0,
            },
        };

        // A cut that did not move forward would repeat for ever: it happens when the nearest
        // keyframe to the whole window is the one already cut at.
        if cut.at_s <= from + A_FRAME_S {
            break;
        }
        from = cut.at_s;
        cuts.push(cut);
    }
    cuts
}

/// The keyframe closest to a moment.
fn nearest(keyframes: &[f64], to: f64) -> f64 {
    keyframes
        .iter()
        .copied()
        .min_by(|a, b| {
            (a - to)
                .abs()
                .partial_cmp(&(b - to).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(to)
}

/// The cuts that did not land where the scene did (T456).
///
/// **Said out loud rather than quietly accepted.** On material whose keyframes follow its
/// scenes these are none. Where they are not none, the file has a fixed group of pictures or
/// has been cut before, and every one of these seams will sit a little away from where the
/// picture changes. That is a decision — take the inexact cut, or re-encode — and it is not
/// this code's to make silently.
pub fn seams_that_missed(cuts: &[Cut]) -> Vec<Cut> {
    cuts.iter()
        .filter(|c| c.scene_s.is_some() && c.off_by_s > A_FRAME_S)
        .copied()
        .collect()
}
