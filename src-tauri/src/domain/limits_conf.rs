//! T205, T206 — the file of substitution rules the application owns.
//!
//! **This file, and nothing else** (R-03). The main serving configuration is not rewritten
//! after deployment: it is a person's own, it may hold things we know nothing about, and a
//! mistake in it costs the whole of the serving — including a showing that is happening at
//! that moment. Everything to do with quality limits goes in here, which the main
//! configuration merely imports.
//!
//! Ported from the project's own recorded practice (`vrcast-hls`), **including both of the
//! mistakes it was bought with**:
//!
//!   1. the caching rule for a description has to be **narrowed**, never declared a second
//!      time — see [`CACHE_NOTE`];
//!   2. a viewer's address over HTTP is not always their address over SSH, so the address a
//!      rule is written for comes from the access log and from nowhere else. That one lives
//!      where the address is chosen rather than here, but it belongs to the same lesson.

use serde::{Deserialize, Serialize};

use super::slow_master::SLOW_DIR;

/// One limit in force.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limit {
    /// The viewer's address **as the serving sees it**.
    pub ip: String,
    /// The medium's own directory.
    pub slug: String,
    /// What they are allowed, in bits per second.
    pub cap_bps: u64,
    /// When it was put in place, so a person can see what is old.
    pub set_at: String,
}

/// Why the caching rule is written the way it is.
///
/// **A second `header Cache-Control` does not beat the first.** The serving declares a
/// blanket `immutable` for everything under the media directory, and a rule that simply
/// declares its own does not replace it — the blanket one stays, a limited viewer's
/// personal description is cached by their own player for thirty days, and lifting the
/// limit changes nothing they can see. Found on a live server; the recorded fix was to
/// write an exception *inside* the main configuration's own block.
///
/// **That fix is not open to us**: the main configuration belongs to the person and is
/// not rewritten after deployment (R-03). So the question was put to Caddy itself, in the
/// container, on 2026-08-26 — four ways of narrowing a header set deeper in the chain:
///
/// | written in the imported file | what came back for the description |
/// |---|---|
/// | a plain set | `public, max-age=2592000, immutable` — the blanket rule, untouched |
/// | a delete, then a set | **no header at all** |
/// | a **deferred** set | `no-cache` — what is wanted |
/// | a deferred delete and set | **no header at all** |
///
/// So: deferred, and setting only. `defer` is what makes the operation happen after the
/// handler that set the blanket rule has had its say. A delete anywhere in the block wins
/// over the set beside it and leaves the description with no caching rule — which is not
/// better than the wrong one: a player left to its own judgement caches what it likes.
///
/// In every case the segments kept the blanket rule, which is right — they really are
/// immutable.
pub const CACHE_NOTE: &str =
    "deferred, and setting only: a plain set loses to the blanket rule, a delete leaves none";
/// The line every generated file starts with.
const HEADER: &str =
    "# The quality-limit rules. This file belongs to VRCast Studio: it is rewritten whole\n\
                      # on every change, and anything added here by hand will be lost.";

/// The marker a rule's own line carries, so the file can be read back.
///
/// **Read back from the server rather than kept only here** (FR-064): a local note goes
/// stale the moment somebody edits the server by hand, and a list of limits that does not
/// match the server is worse than no list.
const MARK: &str = "# vrcast-limit";

/// Build the whole file.
///
/// Whole, never appended to: a file assembled from what is wanted now cannot drift, and
/// drift in a serving configuration is not the kind of thing anybody notices early.
pub fn build(limits: &[Limit], serving_prefix: &str) -> String {
    let prefix = serving_prefix.trim_end_matches('/');
    let mut out = String::from(HEADER);
    out.push('\n');

    // The caching exception first, and once, whatever the limits are.
    //
    // It is here even with no limits in force: a description of a quality set should never
    // have been cached for thirty days in the first place, and the day a limit appears is
    // too late to start — the players of everyone who watched before it will still be
    // holding the old answer.
    out.push_str(&format!(
        "\n# {CACHE_NOTE}\n\
         @vrcast_master path {prefix}/*/master.m3u8\n\
         header @vrcast_master {{\n\
         \tdefer\n\
         \tCache-Control \"no-cache\"\n\
         }}\n"
    ));

    for limit in limits {
        // A name of its own for each rule: Caddy matchers share one namespace, and two
        // rules under one name would silently become one.
        let key = matcher_name(&limit.ip, &limit.slug);
        out.push_str(&format!(
            "\n{MARK} {ip} {slug} {cap} {at}\n\
             @{key} {{\n\
             \tpath {prefix}/{slug}/master.m3u8\n\
             \tremote_ip {ip}\n\
             }}\n\
             rewrite @{key} {prefix}/{SLOW_DIR}/{slug}/master.m3u8\n",
            ip = limit.ip,
            slug = limit.slug,
            cap = limit.cap_bps,
            at = limit.set_at,
        ));
    }
    out
}

/// Read the limits back out of a file taken from the server.
pub fn parse(text: &str) -> Vec<Limit> {
    text.lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix(MARK)?;
            let mut parts = rest.split_whitespace();
            let ip = parts.next()?.to_owned();
            let slug = parts.next()?.to_owned();
            let cap_bps = parts.next()?.parse().ok()?;
            let set_at = parts.next().unwrap_or("").to_owned();
            Some(Limit {
                ip,
                slug,
                cap_bps,
                set_at,
            })
        })
        .collect()
}

/// The matcher's name for one limit.
///
/// Made of the address and the medium, with everything that is not a letter or a digit
/// turned into an underscore: a matcher name may not hold dots or colons, and an address
/// is mostly those.
pub fn matcher_name(ip: &str, slug: &str) -> String {
    let safe = |s: &str| -> String {
        s.chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect()
    };
    format!("vrcast_{}_{}", safe(ip), safe(slug))
}
