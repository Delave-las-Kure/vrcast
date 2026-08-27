//! T155, T156 — the serving's access log: what was asked for, by whom, and how it went.
//!
//! One of the two sources the list of viewers is assembled from (R-02). This one says
//! **what** is being watched; the other — the server's table of connections — says who is
//! pulling right now and how fast. Neither is enough on its own, and that is not a
//! preference but the shape of the problem: when a film is served as a single file, one
//! request lasts the whole showing, and its line appears in the log **only once the
//! watching has ended**. A list of viewers built on the log alone would show the people who
//! have already finished and nobody who is watching now.
//!
//! Only the parsing and the rules are here. Following the file itself lives in
//! `server::access_log_watch`.

use serde::Deserialize;
use time::OffsetDateTime;

/// One request, as the serving recorded it.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    /// Who asked. This is what a viewer is known by: the server has nothing else about
    /// them (assumption in the specification, FR-060).
    pub client_ip: String,
    /// What was asked for: the path, decoded, without the query.
    pub path: String,
    pub status: u16,
    /// How much went out.
    pub bytes: u64,
    /// How long the answer took.
    pub duration_s: f64,
    /// When, **by the server's clock**.
    ///
    /// Deliberately not this machine's. The two disagree — the specification names that
    /// among its edge cases — and mixing them would make a viewer look as though they had
    /// arrived in the future or left an hour ago.
    pub at: OffsetDateTime,
}

/// Why a line yielded nothing.
///
/// Told apart rather than lumped together as "a bad line": a line the tail of the file was
/// caught mid-write is normal and expected, whereas a whole file of foreign lines means the
/// serving is configured to write something else, and that is worth saying out loud.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineProblem {
    /// Not JSON at all — or JSON caught halfway through being written.
    NotJson,
    /// JSON, but not a record of a request: Caddy writes its own working notes into the
    /// same stream.
    NotARequest,
    /// A record of a request with a field missing that everything else rests on.
    Incomplete(&'static str),
}

/// The shape Caddy writes. Unknown fields are ignored on purpose: the serving may be a
/// newer version, and a new field there is no reason to stop reading the log.
#[derive(Debug, Deserialize)]
struct RawLine {
    #[serde(default)]
    ts: Option<f64>,
    #[serde(default)]
    msg: Option<String>,
    #[serde(default)]
    request: Option<RawRequest>,
    #[serde(default)]
    status: Option<u16>,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    duration: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct RawRequest {
    /// The address the request came from, as the serving worked it out.
    #[serde(default)]
    client_ip: Option<String>,
    /// The same thing under its older name. Both are read: which of them a given version
    /// writes is not worth depending on.
    #[serde(default)]
    remote_ip: Option<String>,
    #[serde(default)]
    uri: Option<String>,
}

/// The line Caddy marks a served request with.
const HANDLED: &str = "handled request";

/// Parse one line of the log.
pub fn parse_line(line: &str) -> Result<Request, LineProblem> {
    let line = line.trim();
    if line.is_empty() {
        return Err(LineProblem::NotJson);
    }
    let raw: RawLine = serde_json::from_str(line).map_err(|_| LineProblem::NotJson)?;

    if raw.msg.as_deref() != Some(HANDLED) {
        return Err(LineProblem::NotARequest);
    }
    let request = raw.request.ok_or(LineProblem::Incomplete("request"))?;
    let client_ip = request
        .client_ip
        .or(request.remote_ip)
        .filter(|ip| !ip.is_empty())
        .ok_or(LineProblem::Incomplete("client_ip"))?;
    let uri = request.uri.ok_or(LineProblem::Incomplete("uri"))?;
    let ts = raw.ts.ok_or(LineProblem::Incomplete("ts"))?;

    Ok(Request {
        client_ip,
        path: decode_path(&uri),
        status: raw.status.ok_or(LineProblem::Incomplete("status"))?,
        // A request that sent nothing writes no size, and that is not damage: a 304 or a
        // refusal is a normal answer. Zero is the truth here, not a stand-in for it.
        bytes: raw.size.unwrap_or(0),
        duration_s: raw.duration.unwrap_or(0.0),
        at: from_unix_seconds(ts).ok_or(LineProblem::Incomplete("ts"))?,
    })
}

/// Turn the seconds Caddy writes into a point in time.
fn from_unix_seconds(ts: f64) -> Option<OffsetDateTime> {
    if !ts.is_finite() {
        return None;
    }
    OffsetDateTime::from_unix_timestamp_nanos((ts * 1e9) as i128).ok()
}

/// Strip the query and undo the percent-encoding.
///
/// Both matter. A file uploaded before the application existed may be named anything at
/// all — with spaces, with Cyrillic — and its name reaches the log encoded; left that way
/// it would match nothing in the library, and the viewer would be shown watching an unknown
/// something. A query is not part of what was asked for: `?v=2` on the end must not make it
/// a different file.
fn decode_path(uri: &str) -> String {
    let without_query = uri.split(['?', '#']).next().unwrap_or("");
    super::links::decode_path(without_query)
}

// ---------- what exactly was asked for ----------

/// What the path points at.
///
/// The three ways of serving are told apart because they behave differently in the log
/// (R-02): a direct file leaves one line at the end of the watching, a quality set leaves a
/// line per segment throughout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Asked {
    /// A file served as itself — no quality set.
    DirectFile { name: String },
    /// The description of a quality set: the list of rungs.
    SetDescription {
        slug: String,
        /// Whether this is the shortened description handed to a viewer under a limit
        /// (Phase 6). Told apart so that such a viewer does not look like someone asking
        /// for something unrecognised.
        shortened: bool,
    },
    /// The playlist of one rung.
    RungPlaylist { slug: String, rung: String },
    /// A segment of one rung. This is what a watching of a quality set consists of.
    Segment { slug: String, rung: String },
    /// The stream headers a fragmented set begins with.
    ///
    /// Told apart from a segment although it sits in the same directory and is fetched in the
    /// same breath. It carries no film, so counting it as a segment inflates every count of
    /// how much a viewer received — and a healthy player fetches it exactly once a session,
    /// which makes a second fetch a fact worth having (`domain::stalls`).
    SetInit { slug: String, rung: String },
    /// Something outside the serving, or a shape not known here.
    Other,
}

impl Asked {
    /// The name the library knows this by: a file's name, or a set's short name.
    pub fn library_key(&self) -> Option<&str> {
        match self {
            Self::DirectFile { name } => Some(name),
            Self::SetDescription { slug, .. }
            | Self::RungPlaylist { slug, .. }
            | Self::Segment { slug, .. }
            | Self::SetInit { slug, .. } => Some(slug),
            Self::Other => None,
        }
    }

    /// Which rung is being received, when it is a quality set.
    pub fn rung(&self) -> Option<&str> {
        match self {
            Self::RungPlaylist { rung, .. }
            | Self::Segment { rung, .. }
            | Self::SetInit { rung, .. } => Some(rung),
            _ => None,
        }
    }

    /// Whether this is the pulling of the video itself, rather than asking what there is.
    ///
    /// A description is asked for once at the start; a person who only asked for it is not
    /// yet watching. What the watching is made of is segments — and, for a direct file, the
    /// one long request for the file itself.
    pub fn is_pulling_video(&self) -> bool {
        matches!(
            self,
            Self::DirectFile { .. } | Self::Segment { .. } | Self::SetInit { .. }
        )
    }
}

/// The directory the shortened descriptions live in (R-14).
pub const SHORTENED_DIR: &str = "_slow";

/// The name the description of a quality set is served under.
pub const SET_DESCRIPTION: &str = "master.m3u8";

/// The name a rung's playlist is served under.
pub const RUNG_PLAYLIST: &str = "stream.m3u8";

/// The name the stream headers of a fragmented set are served under.
///
/// The literal name, because that is what our own packaging writes
/// (`hls_package`: `-hls_fmp4_init_filename init.mp4`). A set built by something else may
/// name it otherwise, and then it reads as a segment — which is the same answer as before
/// this variant existed, and no worse.
pub const SET_INIT: &str = "init.mp4";

/// Work out what a path points at.
pub fn what_was_asked_for(path: &str) -> Asked {
    let rest = match path
        .trim_start_matches('/')
        .strip_prefix(super::links::VIDEOS_PREFIX)
    {
        Some(rest) => rest.trim_start_matches('/'),
        None => return Asked::Other,
    };
    let parts: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();

    // The arms are ordered from the most particular downwards, and the order is load
    // bearing: a shortened description is three parts long, and so is a segment.
    match parts.as_slice() {
        // /videos/film.mp4
        [name] => Asked::DirectFile {
            name: (*name).to_owned(),
        },
        // /videos/_slow/<slug>/master.m3u8
        [dir, slug, tail] if *dir == SHORTENED_DIR && *tail == SET_DESCRIPTION => {
            Asked::SetDescription {
                slug: (*slug).to_owned(),
                shortened: true,
            }
        }
        // /videos/<slug>/master.m3u8
        [slug, tail] if *tail == SET_DESCRIPTION => Asked::SetDescription {
            slug: (*slug).to_owned(),
            shortened: false,
        },
        // /videos/<slug>/<rung>/stream.m3u8
        [slug, rung, tail] if *tail == RUNG_PLAYLIST => Asked::RungPlaylist {
            slug: (*slug).to_owned(),
            rung: (*rung).to_owned(),
        },
        // /videos/<slug>/<rung>/init.mp4
        [slug, rung, tail] if *tail == SET_INIT => Asked::SetInit {
            slug: (*slug).to_owned(),
            rung: (*rung).to_owned(),
        },
        // /videos/<slug>/<rung>/<segment>
        [slug, rung, _segment] => Asked::Segment {
            slug: (*slug).to_owned(),
            rung: (*rung).to_owned(),
        },
        _ => Asked::Other,
    }
}
