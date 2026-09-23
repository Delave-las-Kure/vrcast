//! T203, T204 — the shortened description a limited viewer is given (FR-062, FR-067).
//!
//! **Why a shortened description at all.** A player takes the best variant it is shown and
//! nothing will talk it out of that. The only way to bring a viewer down to a rung their
//! line can hold is to **stop showing them the ones it cannot** — so they are handed a
//! description with the upper rungs left out.
//!
//! **The segments are not copied.** Only the description is made; the variants it points at
//! are the same files everyone else is served (SC-007: the disk must not grow by more than
//! a hundredth).
//!
//! Ported from the project's own recorded practice (R-14, `vrcast-hls`), including the
//! mistake that practice was bought with — see [`shorten`].

use serde::{Deserialize, Serialize};

use super::hls_master::{build, Variant};

/// Where the serving keeps the shortened descriptions.
///
/// Beside the media rather than inside it: a viewer with no limit must never stumble into
/// one, and a directory of its own is also what the substitution rule rewrites onto.
pub const SLOW_DIR: &str = "_slow";

/// What came of shortening a description.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shortened {
    /// The description itself, ready to be written.
    pub text: String,
    /// The variants left in it, top first.
    pub kept: Vec<Variant>,
    /// The cap could not be met: even the lightest rung is above it.
    ///
    /// **The lightest is given anyway.** An empty description leaves a viewer with no video
    /// at all, which is worse than video they cannot quite hold — and a person setting the
    /// limit is told, so they can go and build a lighter rung if they want one (FR-067).
    pub below_lightest: bool,
}

/// Shorten a description to a ceiling.
///
/// `serving_prefix` is where the serving root sits in an address — `/videos` on this
/// project's servers. `slug` is the medium's own directory under it.
///
/// **The paths in the result are absolute, and that is the whole of the recorded mistake.**
/// A shortened description lives in a directory of its own, so a relative `v10/stream.m3u8`
/// would send the player looking for the segments *inside that directory*, where there are
/// none. It is written `/videos/<slug>/v10/stream.m3u8` and it points at the same files
/// everybody else gets.
pub fn shorten(variants: &[Variant], cap_bps: u64, serving_prefix: &str, slug: &str) -> Shortened {
    let mut sorted: Vec<Variant> = variants.to_vec();
    sorted.sort_by_key(|v| std::cmp::Reverse(v.bandwidth));

    let within: Vec<Variant> = sorted
        .iter()
        .filter(|v| v.bandwidth <= cap_bps)
        .cloned()
        .collect();

    let below_lightest = within.is_empty() && !sorted.is_empty();
    let kept: Vec<Variant> = if below_lightest {
        // The lightest there is. Not nothing.
        sorted.last().cloned().into_iter().collect()
    } else {
        within
    };

    let absolute: Vec<Variant> = kept
        .iter()
        .map(|v| Variant {
            path: absolute_path(serving_prefix, slug, &v.path),
            ..v.clone()
        })
        .collect();

    Shortened {
        text: build(&absolute),
        kept: absolute,
        below_lightest,
    }
}

/// Where the shortened description for one ceiling on one medium sits, relative to the
/// serving root: `_slow/<slug>/<cap_bps>/master.m3u8` (T602).
///
/// **One file per ceiling, not one per medium.** Every rule of a medium used to rewrite
/// onto the same `_slow/<slug>/master.m3u8`, so the last limit set on a medium decided what
/// every limited viewer of it was shown — A capped low and B capped high both got whichever
/// was written last, while the list of limits said otherwise. Viewers with the same ceiling
/// on the same medium are shown the same thing, so they share a file.
///
/// The one place this shape is written down: the file on disk (`slow_master_path`) and the
/// address a rule rewrites onto (`slow_master_address`) are both made from it, so the two
/// cannot drift apart.
pub fn slow_master_relative(slug: &str, cap_bps: u64) -> String {
    format!("{SLOW_DIR}/{slug}/{cap_bps}/master.m3u8")
}

/// Where a shortened description is written, under the serving directory.
pub fn slow_master_path(video_dir: &str, slug: &str, cap_bps: u64) -> String {
    format!(
        "{}/{}",
        video_dir.trim_end_matches('/'),
        slow_master_relative(slug, cap_bps)
    )
}

/// The address a limited viewer's request is rewritten onto, e.g.
/// `/videos/_slow/demo/6000000/master.m3u8`.
pub fn slow_master_address(serving_prefix: &str, slug: &str, cap_bps: u64) -> String {
    format!(
        "{}/{}",
        serving_prefix.trim_end_matches('/'),
        slow_master_relative(slug, cap_bps)
    )
}

/// Where the application wrote a medium's one shortened description before T602:
/// `<video_dir>/_slow/<slug>/master.m3u8`.
///
/// Only ever removed, never written: the first change of the rules after an upgrade moves
/// every rule onto `slow_master_path`, and this file is then reached by nothing.
pub fn legacy_slow_master_path(video_dir: &str, slug: &str) -> String {
    format!(
        "{}/{SLOW_DIR}/{slug}/master.m3u8",
        video_dir.trim_end_matches('/')
    )
}

/// The directory of one medium's shortened descriptions: `<video_dir>/_slow/<slug>`.
pub fn slow_slug_dir(video_dir: &str, slug: &str) -> String {
    format!("{}/{SLOW_DIR}/{slug}", video_dir.trim_end_matches('/'))
}

/// The directory of one ceiling's description: `<video_dir>/_slow/<slug>/<cap_bps>`.
pub fn slow_cap_dir(video_dir: &str, slug: &str, cap_bps: u64) -> String {
    format!("{}/{cap_bps}", slow_slug_dir(video_dir, slug))
}

/// Whether a short name read out of the rules can be used as one part of a path.
///
/// The rules file is ours but the server is not: a hand-edited rule could name `..` or
/// `a/b`, and the plan below removes files by the names it makes from these. Narrower than
/// `media::validate_slug` on purpose — this is about not leaving `_slow/`, not about what a
/// good short name looks like, and a medium named before that check existed must still be
/// looked after.
pub fn slug_is_safe_in_a_path(slug: &str) -> bool {
    !slug.is_empty() && slug != "." && slug != ".." && !slug.contains(['/', '\\', '\0', '\n', '\r'])
}

/// What one change of the rules does under `_slow/` (T602).
///
/// Worked out from the rules in force before the change and the rules that will be in
/// force after it, and nothing else — so it can be checked without a server.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SlowPlan {
    /// Every distinct (medium, ceiling) the rules after the change name, in order. Each
    /// must have its description written before the rules go in: a rule pointing at a
    /// file that is not there serves the limited viewer nothing.
    pub descriptions: Vec<(String, u64)>,
    /// Files that are reached by nothing once the change is in force: the descriptions of
    /// ceilings no rule names any more, and every pre-T602 `_slow/<slug>/master.m3u8` of a
    /// medium named before or after. Removed only **after** the new rules are loaded and
    /// the serving has answered — until then the rules still in force point at them.
    pub remove_files: Vec<String>,
    /// Directories to remove afterwards if they are empty, deepest first: the ceilings'
    /// then the media's. `_slow/` itself is never among them.
    pub remove_dirs_if_empty: Vec<String>,
}

/// Work out `SlowPlan` for one change of the rules.
///
/// Rules naming a medium that cannot be used in a path (`slug_is_safe_in_a_path`) are left
/// out entirely: nothing is written or removed for them.
pub fn plan(
    video_dir: &str,
    before: &[super::limits_conf::Limit],
    after: &[super::limits_conf::Limit],
) -> SlowPlan {
    use std::collections::BTreeSet;

    let pairs = |limits: &[super::limits_conf::Limit]| -> BTreeSet<(String, u64)> {
        limits
            .iter()
            .filter(|l| slug_is_safe_in_a_path(&l.slug))
            .map(|l| (l.slug.clone(), l.cap_bps))
            .collect()
    };
    let before = pairs(before);
    let after = pairs(after);

    let gone: Vec<&(String, u64)> = before.difference(&after).collect();
    let slugs: BTreeSet<&String> = before.iter().chain(after.iter()).map(|(s, _)| s).collect();

    let mut remove_files: Vec<String> = gone
        .iter()
        .map(|(slug, cap)| slow_master_path(video_dir, slug, *cap))
        .collect();
    remove_files.extend(
        slugs
            .iter()
            .map(|slug| legacy_slow_master_path(video_dir, slug)),
    );

    let mut remove_dirs_if_empty: Vec<String> = gone
        .iter()
        .map(|(slug, cap)| slow_cap_dir(video_dir, slug, *cap))
        .collect();
    remove_dirs_if_empty.extend(slugs.iter().map(|slug| slow_slug_dir(video_dir, slug)));

    SlowPlan {
        descriptions: after.into_iter().collect(),
        remove_files,
        remove_dirs_if_empty,
    }
}

/// Turn a variant's own path into one that works from anywhere.
fn absolute_path(serving_prefix: &str, slug: &str, path: &str) -> String {
    if path.starts_with('/') {
        return path.to_owned();
    }
    let prefix = serving_prefix.trim_end_matches('/');
    let prefix = if prefix.starts_with('/') {
        prefix.to_owned()
    } else {
        format!("/{prefix}")
    };
    format!("{prefix}/{slug}/{path}")
}
