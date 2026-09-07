//! T529 — what a quality set's heaviest rung actually is, read back off the server.
//!
//! The library shows a set as one entry (FR-015), and FR-012 asks for it to carry the same
//! particulars an ordinary file does: resolution, bitrate, length. None of that lives in the
//! catalogue — only the path of the description — so it is read back from what the set
//! itself already carries: the master playlist names every rung's resolution and bitrate
//! (`domain::hls_master::parse`), and the cutting step's own `.facts` file names every
//! segment's length (`domain::hls_package::read_facts`).
//!
//! **Best effort throughout, deliberately.** A set built by an older version of this
//! application, or laid down by something else entirely, may have a master with no
//! `.facts` beside it — the file this reads did not exist before the cutting step was
//! written. Failing the whole library over one set's missing particulars would be worse
//! than showing that one set with blanks, the same choice `probe_moov` already makes for an
//! ordinary file whose header will not parse.

use crate::domain::hls_master::{self, Variant};
use crate::domain::hls_package;
use crate::ssh::Connection;

/// What the heaviest rung of a set turned out to be.
pub struct TopRung {
    pub width: u32,
    pub height: u32,
    pub bitrate_bps: u64,
    /// `None` when there is no `.facts` file to read it from.
    pub duration_s: Option<f64>,
}

/// Read the master playlist and the heaviest rung's own facts.
///
/// `None` when the master cannot be read or does not parse as one — the set is shown with
/// blanks for its particulars rather than not shown at all; `exists_on_server` already says
/// whether the directory itself is there.
pub async fn top_rung(conn: &Connection, video_dir: &str, slug: &str) -> Option<TopRung> {
    let master_path = format!("{}/{slug}/master.m3u8", video_dir.trim_end_matches('/'));
    let text = conn
        .exec(&format!(
            "cat {} 2>/dev/null || true",
            crate::server::shell_quote(&master_path)
        ))
        .await
        .ok()?
        .stdout;
    let variants = hls_master::parse(&text).ok()?;
    // The heaviest, not the first: today's builder writes the top rung first, but that is
    // an accident of how it works and not a promise the description itself makes.
    let top: &Variant = variants.iter().max_by_key(|v| v.bandwidth)?;

    let duration_s = facts_duration(conn, video_dir, slug, &top.path).await;

    Some(TopRung {
        width: top.width,
        height: top.height,
        // The average, to answer the same question `probe_moov` answers for an ordinary
        // file's `bitrate_bps`. The peak already lives in the master, for a player to size
        // its connection by, and repeating it under this name would answer a different
        // question with the same word.
        bitrate_bps: top.average_bandwidth,
        duration_s,
    })
}

/// The rung's own length, summed from the segments its `.facts` file names.
///
/// `None`, not zero, when the file is not there: a length of zero reads as "this set is
/// instant", and the honest answer is that nothing here says how long it runs.
async fn facts_duration(
    conn: &Connection,
    video_dir: &str,
    slug: &str,
    variant_path: &str,
) -> Option<f64> {
    let sub = variant_path.rsplit_once('/').map(|(dir, _)| dir)?;
    let path = format!("{}/{slug}/{sub}/.facts", video_dir.trim_end_matches('/'));
    let out = conn
        .exec(&format!(
            "cat {} 2>/dev/null || true",
            crate::server::shell_quote(&path)
        ))
        .await
        .ok()?;
    if out.stdout.trim().is_empty() {
        return None;
    }
    let facts = hls_package::read_facts(&out.stdout).ok()?;
    let total: f64 = facts.segments.iter().map(|s| s.duration_s).sum();
    (total > 0.0).then_some(total)
}
