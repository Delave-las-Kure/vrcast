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

/// Where a shortened description is written, under the serving directory.
pub fn slow_master_path(video_dir: &str, slug: &str) -> String {
    format!(
        "{}/{SLOW_DIR}/{slug}/master.m3u8",
        video_dir.trim_end_matches('/')
    )
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
