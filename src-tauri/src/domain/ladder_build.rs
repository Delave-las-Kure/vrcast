//! T194, T195 — what each rung of a ladder needs before it can be cut.
//!
//! Pure decisions only: the names the variants get, whether a rung has to be re-encoded at
//! all, and the one rule that catches people out — **keyframes**.
//!
//! Doing the work is [`crate::tasks::ladder_build`].

use serde::{Deserialize, Serialize};

use super::convert_plan::{ConvertPlan, ConvertRequest, VideoAction};
use super::ladder::Rung;
use super::source::SourceFile;
use super::wording::{Detail, DetailCode};

/// What one variant of a ladder is called and what has to happen to it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VariantWork {
    pub index: usize,
    /// The directory its segments go in, under the media's own: `v22`.
    pub sub: String,
    /// The prepared file's name in the serving directory: `film_22.mp4`.
    pub file: String,
    pub rung: Rung,
    /// How to prepare it.
    pub plan: ConvertPlan,
    /// True when the quality is carried across untouched — minutes instead of hours.
    pub lossless: bool,
    /// What has to be said about this variant, if anything.
    pub notices: Vec<Detail>,
}

/// The directory a rung's segments go in.
///
/// Named by the rung's whole megabits, as every file this project has ever made has been.
/// A person reading `v22` beside a file called `film_22.mp4` knows they belong together
/// without being told.
pub fn sub_name(rung: &Rung) -> String {
    format!("v{}", (rung.bitrate_bps / 1_000_000).max(1))
}

/// The prepared file's name for a rung.
pub fn file_name(slug: &str, rung: &Rung) -> String {
    format!("{slug}_{}.mp4", mbit_of(rung))
}

fn mbit_of(rung: &Rung) -> u64 {
    (rung.bitrate_bps / 1_000_000).max(1)
}

/// The names a rung's prepared file may take, in the order they are tried (T677):
/// `{slug}_{N}.mp4`, then `{slug}_{N}v.mp4`, `{slug}_{N}v2.mp4`, `{slug}_{N}v3.mp4` …
///
/// **Why there is more than one.** The shell script this application replaces named a
/// medium's single file exactly as a rung's prepared file is named — `film_9.mp4` — and a
/// set built into that medium would write over it. The owner's decision of 2026-10-02: the
/// medium's file is not touched and is not a reason to refuse; the rung takes the next name
/// nobody claims. A letter after the number keeps it apart from every other rung's first
/// name (`film_9v.mp4` is never `film_{N}.mp4` of any N) and keeps it beside its own.
pub fn file_names(slug: &str, rung: &Rung) -> impl Iterator<Item = String> {
    let slug = slug.to_owned();
    let mbit = mbit_of(rung);
    (0u32..).map(move |k| match k {
        0 => format!("{slug}_{mbit}.mp4"),
        1 => format!("{slug}_{mbit}v.mp4"),
        k => format!("{slug}_{mbit}v{k}.mp4"),
    })
}

/// The whole megabits of a prepared rung of the set `slug`, by its name — any of
/// [`file_names`]. `None` for anything else.
pub fn rung_mbit_of(slug: &str, name: &str) -> Option<u64> {
    let rest = name
        .strip_prefix(slug)?
        .strip_prefix('_')?
        .strip_suffix(".mp4")?;
    let (mbit, tail) = match rest.find('v') {
        Some(at) => (&rest[..at], Some(&rest[at + 1..])),
        None => (rest, None),
    };
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    if !digits(mbit) {
        return None;
    }
    match tail {
        None | Some("") => {}
        // `v2` and on: `v1` is never made (that is plain `v`), nor `v02`.
        Some(k) if digits(k) && !k.starts_with('0') && k != "1" => {}
        Some(_) => return None,
    }
    mbit.parse().ok()
}

/// The name of the set's own record of its prepared files, inside `{slug}/` (T677).
///
/// Written only when a rung took a name other than its first, before anything is encoded,
/// so carrying on after a restart and «Retry» find the rung under the same name even if the
/// catalogue changed meanwhile. One line per rung: `v9=film_9v.mp4`. Goes with the set's
/// directory: «Replace» and deleting the medium remove it with the rest.
pub const PREPARED_RECORD: &str = ".prepared";

/// Read [`PREPARED_RECORD`]: the rung directory and its prepared file, as written.
pub fn parse_prepared(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| l.trim().split_once('='))
        .map(|(sub, file)| (sub.trim().to_owned(), file.trim().to_owned()))
        .filter(|(sub, file)| !sub.is_empty() && !file.is_empty())
        .collect()
}

/// Write [`PREPARED_RECORD`] for these variants.
pub fn prepared_text(work: &[VariantWork]) -> String {
    work.iter()
        .map(|w| format!("{}={}\n", w.sub, w.file))
        .collect()
}

/// Which prepared file each rung is made into (T677), in the order of `work`.
///
/// - What the set's own record says, while that is still a rung's name of this set at this
///   bitrate and no medium claims it — carrying on finds what it began.
/// - Otherwise the first of [`file_names`] that no medium claims and no other rung of this
///   set has taken. `whole_claimed` are claimed files that already are their rung, whole:
///   those are taken as they are (T675 — a rebuild whose rungs a person filed under the
///   medium) and keep their first name.
///
/// A claimed file is never one of the names given out otherwise: it is somebody's, and it
/// is neither written over nor removed (T577, part b).
pub fn choose_files(
    slug: &str,
    work: &[VariantWork],
    claimed: &[&str],
    stored: &[(String, String)],
    whole_claimed: &[String],
) -> Vec<String> {
    let mut taken: Vec<String> = Vec::new();
    for w in work {
        let mbit = mbit_of(&w.rung);
        let from_record = stored
            .iter()
            .find(|(sub, _)| sub == &w.sub)
            .map(|(_, file)| file.clone())
            .filter(|f| rung_mbit_of(slug, f) == Some(mbit))
            .filter(|f| !claimed.contains(&f.as_str()) && !taken.contains(f));
        let first = file_name(slug, &w.rung);
        let name = from_record.unwrap_or_else(|| {
            if whole_claimed.contains(&first) && !taken.contains(&first) {
                return first;
            }
            file_names(slug, &w.rung)
                .find(|n| !claimed.contains(&n.as_str()) && !taken.contains(n))
                .unwrap_or(first)
        });
        taken.push(name);
    }
    taken
}

/// Whether a source's keyframes fall where every other variant's will.
///
/// **This is the rule that catches people out.** A rung whose quality needs no change is
/// carried across without re-encoding — minutes rather than hours, and no loss at all. But
/// a carried-across stream keeps the source's own keyframes, and segments can only be cut
/// at a keyframe. The re-encoded variants get one every second of film; if the source has
/// one every five, their boundaries stop lining up, and a viewer whose connection drops
/// changes quality in the middle of nothing: the player waits for the next point at which
/// the two agree, and what they see is a stall.
///
/// **Counted in frames, not in seconds.** Seconds are where this goes wrong twice. A film
/// at 23.976 frames a second has a keyframe every 1.001 s — which is not one second, and
/// is nevertheless exactly right, because the variants encoded from it get the same. And a
/// stream with 23 frames between keyframes is 0.958 s, which is within a frame of a second
/// and lines up with nothing: by the twenty-fourth interval it is a whole second out.
///
/// So: the spacing in frames must be a whole number, and that number must divide the
/// frames in a segment. 24 in a segment of 96 divides four ways; 23 divides nothing.
pub fn keyframes_line_up(source_keyframe_s: f64, fps: u32, segment_s: u32) -> bool {
    if !source_keyframe_s.is_finite() || source_keyframe_s <= 0.0 || fps == 0 {
        return false;
    }
    let frames = source_keyframe_s * f64::from(fps);
    let whole = frames.round();
    // Within a tenth of a frame of a whole number of frames. Anything looser lets in a
    // spacing that drifts, and drift is what this exists to prevent.
    if (frames - whole).abs() > 0.1 || whole < 1.0 {
        return false;
    }
    let per_segment = u64::from(segment_s) * u64::from(fps);
    let spacing = whole as u64;
    spacing <= per_segment && per_segment % spacing == 0
}
/// Work out what each rung needs.
///
/// `source_keyframe_s` is how far apart the source's own keyframes are, when that has been
/// measured. `None` means it has not been, and a copy is then not offered: guessing that
/// they line up is the one guess here that a viewer pays for.
pub fn work_for(
    slug: &str,
    rungs: &[Rung],
    source: &SourceFile,
    audio_track: usize,
    source_keyframe_s: Option<f64>,
    segment_s: u32,
) -> Vec<VariantWork> {
    rungs
        .iter()
        .map(|rung| {
            // **A rung that is the source asks for nothing.** Handing the planner a
            // target bitrate is itself a reason to re-encode — quite rightly, for a
            // single file somebody asked to compress. Here the top rung regularly *is*
            // the source, and asking it for exactly what it already has would spend
            // hours to arrive back where we started, with a generation of loss for it.
            let unchanged = rung.height == source.height
                && rung.bitrate_bps >= source.bitrate_bps
                && rung.width == source.width;
            let request = if unchanged {
                ConvertRequest {
                    audio_track,
                    target_kbps: None,
                    height: None,
                }
            } else {
                ConvertRequest {
                    audio_track,
                    target_kbps: Some((rung.bitrate_bps / 1000).max(1) as u32),
                    height: Some(rung.height),
                }
            };
            let mut plan = super::convert_plan::plan(source, &request).unwrap_or_else(|_| {
                // A rung that will not plan is not a reason to lose the others: it is
                // re-encoded on the ordinary path and the checker has already had its say
                // about whether it should exist at all.
                fallback_plan(source, &request)
            });

            let mut notices = Vec::new();
            // The one place a copy is taken away for a reason that has nothing to do with
            // quality. Said out loud, because "this rung will take hours after all" is not
            // something to discover from a progress bar.
            if plan.video == VideoAction::Copy
                && !source_keyframe_s
                    .map(|spacing| keyframes_line_up(spacing, source.fps, segment_s))
                    .unwrap_or(false)
            {
                plan.video = VideoAction::ReencodeCapped {
                    reason: Detail::new(DetailCode::ReasonKeyframesUnaligned),
                    target_kbps: (rung.bitrate_bps / 1000).max(1) as u32,
                    maxrate_kbps: (rung.maxrate_bps / 1000).max(1) as u32,
                    bufsize_kbps: (rung.bufsize_bps / 1000).max(1) as u32,
                    level: rung.level.clone(),
                };
                notices.push(Detail::new(DetailCode::NoticeReencodedForKeyframes));
            }

            VariantWork {
                index: rung.index,
                sub: sub_name(rung),
                file: file_name(slug, rung),
                rung: rung.clone(),
                lossless: plan.lossless(),
                plan,
                notices,
            }
        })
        .collect()
}

/// Every variant is prepared with the **same** keyframe spacing.
///
/// One per second of film, at whatever the frame rate is — the same rule as for a single
/// prepared file, and deliberately the same function. A constant would be wrong twice over:
/// 48 means "once a second" on 48-frame material and "once every two" on 24-frame, and two
/// rungs given different numbers stop agreeing about where a segment may begin.
pub fn shared_gop(source: &SourceFile) -> u32 {
    source.fps.max(1)
}

fn fallback_plan(source: &SourceFile, request: &ConvertRequest) -> ConvertPlan {
    ConvertPlan {
        video: VideoAction::Reencode {
            reason: Detail::new(DetailCode::ReasonTargetBitrate),
            level: super::convert_plan::h264_level(
                source.width,
                request.height.unwrap_or(source.height),
                source.fps,
            )
            .to_owned(),
        },
        audio: super::convert_plan::AudioAction::Copy,
        audio_track: request.audio_track,
        gop: shared_gop(source),
        tonemap: false,
        requested_height: request.height,
        faststart: true,
    }
}

/// The variants a rebuild is about to stop serving, out of what is on the server already.
///
/// **What this is for, and the day it was written.** On 2026-08-29 a set on the production
/// server was rebuilt without one of its rungs, and that rung stopped being served. The shell
/// script this application replaces loses it by deleting the directory outright; this
/// application loses it more quietly, and the quiet way was found by looking for the loud one:
/// the master is written from the rungs of *this* build, so a variant already on the server
/// and not in the set simply stops being mentioned. Its file and its segments stay on the
/// disk. Nobody is told, and a viewer on that quality finds it gone.
///
/// **The set is not widened behind the person's back.** Somebody who built 7/2/1 may have
/// meant to drop 4. Quietly putting it back would overrule a decision, which is the same
/// fault in the other direction. What is owed is the fact: this is still on the disk, and it
/// is no longer served.
///
/// `on_server` is the show's own directory, `wanted` the variants this build is making.
/// Only directories are considered — the prepared `.mp4` beside them is the source of a
/// variant, not a variant, and it is never served directly.
pub fn stranded(on_server: &[(String, bool)], wanted: &[VariantWork]) -> Vec<String> {
    let mut out: Vec<String> = on_server
        .iter()
        .filter(|(_, is_dir)| *is_dir)
        .map(|(name, _)| name.clone())
        // A variant's directory is named `v` and its whole megabits, and nothing else in a
        // show's directory is. Anything else there belongs to somebody else and is not ours
        // to have an opinion about.
        .filter(|name| {
            name.strip_prefix('v')
                .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
        })
        .filter(|name| !wanted.iter().any(|w| &w.sub == name))
        .collect();
    out.sort();
    out
}
