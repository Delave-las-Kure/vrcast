//! T032 — viewer links (FR-016).
//!
//! Serving hands out files from the video directory under `/videos/…` — that is how
//! the working server is arranged, and the application must produce the same links
//! that work today.
//!
//! Two links rather than one: the origin is served by the server itself, the CDN by an
//! intermediary. When a CDN is configured the choice is left to the person (FR-016),
//! because the options cost different things: the origin is not blocked in Russia, the
//! CDN is faster but depends on someone else.

/// The part of the serving path the video directory sits under.
pub const VIDEOS_PREFIX: &str = "videos";

/// The finished links for one file.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Links {
    /// The link through the server itself. Always there.
    pub origin: String,
    /// The link through the CDN. Absent when no CDN is set in the profile.
    pub cdn: Option<String>,
}

impl Links {
    /// The default link — the one that works without an intermediary.
    pub fn preferred(&self) -> &str {
        &self.origin
    }
}

/// T694 (the owner's decision of 2026-10-09) — a link to one quality of a set, held steady:
/// the rung's own HLS playlist (`{slug}/v9/stream.m3u8`). For when everybody's connection is
/// slow and a steady low bitrate serves better than the player's own choosing. Nothing new
/// is made on the server for it: the playlist is the one the set's master already names.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct QualityLink {
    pub width: u32,
    pub height: u32,
    /// The rung's own bitrate, as its plan and its directory name it (`v9` → 9 Mbit/s).
    pub bitrate_bps: u64,
    pub origin: String,
    pub cdn: Option<String>,
}

/// The whole megabits a rung's playlist path names (`v9/stream.m3u8` → 9), if it does.
pub fn mbit_of_playlist(path: &str) -> Option<u64> {
    let sub = path.split('/').next()?;
    let n = sub.strip_prefix('v')?;
    if n.is_empty() || !n.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    n.parse().ok()
}

/// A link per quality of the set `slug`, from what its master names (T694), the heaviest
/// first. A path the master writes absolute or as a full address is not a rung of this set
/// served beside its master, and is left out rather than linked wrongly.
pub fn qualities_of(
    domain: &str,
    cdn_base: Option<&str>,
    slug: &str,
    variants: &[super::hls_master::Variant],
) -> Vec<QualityLink> {
    let mut out: Vec<QualityLink> = variants
        .iter()
        .filter(|v| !v.path.starts_with('/') && !v.path.contains("://"))
        .map(|v| {
            let links = for_path(domain, cdn_base, &format!("{slug}/{}", v.path));
            QualityLink {
                width: v.width,
                height: v.height,
                bitrate_bps: mbit_of_playlist(&v.path)
                    .map(|m| m * 1_000_000)
                    .unwrap_or(v.average_bandwidth),
                origin: links.origin,
                cdn: links.cdn,
            }
        })
        .collect();
    out.sort_by(|a, b| b.bitrate_bps.cmp(&a.bitrate_bps));
    out
}

/// Build the links for a served file.
///
/// `rel_path` is relative to the video directory: `Backrooms_22.mp4` or
/// `backrooms/master.m3u8`. `domain` is expected to be normalised already (see
/// `server_profile::normalize_domain`), but the scheme and a trailing slash are
/// stripped here too: this function is also called with data that came out of the
/// database from earlier versions.
pub fn for_path(domain: &str, cdn_base: Option<&str>, rel_path: &str) -> Links {
    let host = super::server_profile::normalize_domain(domain);
    let path = encode_path(rel_path);

    Links {
        origin: format!("https://{host}/{VIDEOS_PREFIX}/{path}"),
        cdn: cdn_base
            .map(str::trim)
            .filter(|b| !b.is_empty())
            .map(|base| {
                let base = base.trim_end_matches('/');
                format!("{base}/{VIDEOS_PREFIX}/{path}")
            }),
    }
}

/// Encode a path for a link, keeping the directory separators.
///
/// The encoding is not pedantry: file names on the server can be anything at all —
/// with spaces, with Cyrillic, with a hash sign. An unencoded hash turns the rest of
/// the name into a fragment, and the link leads nowhere, quietly.
fn encode_path(rel_path: &str) -> String {
    rel_path
        .trim_matches('/')
        .split('/')
        .filter(|seg| !seg.is_empty())
        .map(encode_segment)
        .collect::<Vec<_>>()
        .join("/")
}

/// Undo [`encode_path`] — read a path back out of a link or a log line.
///
/// It lives here, next to the encoder, deliberately: a decoder that drifts away from its
/// encoder is the quietest kind of fault. The two are checked against each other on the
/// same names — with spaces, with Cyrillic, with a hash sign.
///
/// What will not decode is left as it stands. A log holds whatever was asked for, and
/// somebody may have asked for a `%` that is not the start of anything: throwing the name
/// away over that would lose a real viewer, while leaving it is at worst a name that
/// matches nothing.
pub fn decode_path(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(byte) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    // Names on a server are not obliged to be valid text, and a name that is not is still
    // not a reason to lose the viewer who asked for it.
    String::from_utf8_lossy(&out).into_owned()
}

/// Percent-encoding for one segment of a path.
///
/// Only the "unreserved" characters (RFC 3986) are left alone: Latin letters, digits
/// and `-._~`. Everything else is encoded. That is slightly stricter than necessary,
/// but it saves having to remember which characters are safe in which part of a URL;
/// in an ordinary name like `Backrooms_22.mp4` there is nothing to encode, and the
/// link stays readable.
fn encode_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.as_bytes() {
        let c = *byte as char;
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '~') {
            out.push(c);
        } else {
            out.push('%');
            out.push_str(&format!("{byte:02X}"));
        }
    }
    out
}
