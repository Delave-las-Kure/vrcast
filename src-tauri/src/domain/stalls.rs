//! T305–T309 — why a viewer's picture stops, worked out from the log alone.
//!
//! Every number here was bought on a real complaint, and the shape of the answer follows the
//! shape of that afternoon (principle VI — these are carried over, not re-derived):
//!
//! - **How much content against real time** is the one measure that decides everything else:
//!   `segments × segment length ÷ elapsed`. Below 1.0 the viewer is not keeping up. The
//!   recorded case came to **0.53×** — one second of film for every two seconds lived.
//! - **The viewer's speed is measured by the wall clock, not by how long the requests took.**
//!   Those are different numbers and only one of them is the viewer's link: inside the
//!   downloads that viewer was getting 18.6 Mbit/s, and counting the pauses between segments
//!   as well, 15.9. The second is their link. The instantaneous `delivery_rate` out of `ss`
//!   said 9.4 — half — and is not relied on anywhere.
//! - **Gaps in the segment numbers** (9, 11, 14, 16) are the player jumping forward past its
//!   own playhead. It is a sign of starving, not of a network fault, and the two get opposite
//!   advice.
//! - **A gap between requests from a fast viewer is normal**: the buffer is full and the
//!   player is waiting. This is why nothing here complains about timing until the content
//!   ratio says the viewer is behind — the check that fires on healthy viewers is as useless
//!   as the one that stays quiet on sick ones, and rather more annoying.
//!
//! **Who is not a viewer.** A cache node takes one to three segments and leaves; our own
//! checks come from the server's own address. Without setting those aside the busiest
//! "viewer" in the report is us, and the conclusion is drawn about ourselves. They are sifted
//! by **behaviour rather than by a list of addresses**: a list of some CDN's ranges is a
//! hardcoded third-party server (FR-004) that goes stale the week they add a range.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use super::access_log::{what_was_asked_for, Asked, Request};
use super::hls_package::SEGMENT_SECONDS;
use super::wording::{Detail, DetailCode};

/// Below this ratio of content received to time lived, the viewer is not keeping up.
///
/// One exactly, and it is arithmetic rather than a threshold: getting less film than time is
/// passing is the definition of falling behind.
pub const KEEPING_UP: f64 = 1.0;

/// At most this many segments taken means it was a cache filling itself, not a watching.
///
/// Carried over from the skill: cache nodes pull one to three segments. Somebody who has only
/// just arrived looks exactly the same from here, which is why what is said about such an
/// address is "too little to judge" rather than "a cache" — see [`NotAViewer::TooLittle`].
pub const SEGMENTS_TO_BE_A_VIEWER: usize = 4;

/// The shortest stretch a wall-clock speed may be worked out over.
///
/// Below this the figure is noise dressed as a measurement: two segments a second apart give
/// a number that swings by a factor of three depending on which second the log was cut in.
pub const SHORTEST_SPAN_S: f64 = 5.0;

/// What our own packaging names segments with (`hls_package`: `seg_%05d`).
pub const SEGMENT_PREFIX: &str = "seg_";

/// Above this share of the server's capacity going out, the server's own link is the limit.
///
/// **A choice.** Below four fifths there is room for another viewer; above it, adding one
/// takes from the rest, and the honest answer stops being "your viewer's link".
pub const SERVER_LINK_BUSY: f64 = 0.80;

/// What one address did during the stretch of log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Watcher {
    pub client_ip: String,
    /// What they were watching, as the library knows it.
    pub watching: Option<String>,
    /// Segments of the video itself. A direct file counts as one long pull, not as segments.
    pub segments: usize,
    pub bytes: u64,
    pub first: OffsetDateTime,
    pub last: OffsetDateTime,
    /// Time lived between the first request and the last, in seconds.
    pub elapsed_s: f64,
    /// Content received against time lived. `None` when the stretch is too short to say.
    pub content_ratio: Option<f64>,
    /// **Their link**: everything delivered, over the wall clock, pauses included.
    pub mbit_s: Option<f64>,
    /// What it looked like *inside* the downloads. Higher, and not their link — kept only so
    /// that the two can be shown side by side, since the difference is the whole lesson.
    pub in_download_mbit_s: Option<f64>,
    /// Segment numbers that were never asked for, in order. The player jumping its playhead.
    pub skipped: Vec<u32>,
    /// How many times the set description was read after the first. A healthy player reads it
    /// once a session; more than that is the player restarting.
    pub restarts: usize,
    /// How many times the initialisation piece was asked for again. Same meaning.
    pub reinits: usize,
    /// Requests that came back 4xx or 5xx.
    pub failures: usize,
}

impl Watcher {
    /// Whether they are falling behind. The one question the rest is built on.
    pub fn starving(&self) -> bool {
        self.content_ratio.is_some_and(|r| r < KEEPING_UP)
    }
}

/// Why an address was set aside.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotAViewer {
    /// The server's own address: these are our own checks, and judging them is judging
    /// ourselves.
    OurOwnCheck,
    /// One to three segments and gone. A cache filling itself — or somebody who arrived a
    /// moment ago, which from here is the same picture. Either way there is nothing to say
    /// about them yet, and saying it anyway is how the loudest line in the report ends up
    /// being about a machine that was never watching.
    TooLittle { segments: usize },
}

/// An address that was set aside, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetAside {
    pub client_ip: String,
    pub why: NotAViewer,
}

/// The log, sorted into viewers and everybody else.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sifted {
    pub watchers: Vec<Watcher>,
    pub set_aside: Vec<SetAside>,
}

/// What the server itself was doing while the viewer hung.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Load {
    /// Busy share of the processor, 0 to 1.
    pub cpu_busy: f64,
    pub disk_read_mb_s: f64,
    pub out_mbit_s: f64,
    /// What the link can do at all. Measured for the machine, never assumed: the skill's
    /// ~940 Mbit/s belongs to one particular VPS and its provider's shaper.
    pub capacity_mbit_s: f64,
    /// Whether the serving cache is small — the disk is being read instead of memory.
    pub cache_small: bool,
}

/// What the file being served looks like, when it is known (T315).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FileShape {
    pub average_mbit: f64,
    /// The peak over a ten-second window. This is the one that hangs a player.
    pub peak_10s_mbit: f64,
}

/// What is most likely at fault.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cause {
    /// Nothing is. Said out loud rather than left as an empty answer.
    NothingWrong,
    /// The viewer's own link cannot carry what they are being sent.
    ViewerLink,
    /// The server is sending as much as its link can carry.
    ServerLink,
    /// The disk is being read instead of the serving cache.
    Disk,
    /// The link is wide enough on average and the file's peaks are not.
    TheFileItself,
    /// Not enough to say. Never dressed up as one of the above.
    Unclear,
}

/// A conclusion, with the figures it rests on.
///
/// The figures are not decoration: a conclusion nobody can check is a conclusion nobody can
/// argue with, and this one is sometimes wrong (FR-072).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    pub cause: Cause,
    pub say: Detail,
}

/// Sort a stretch of log into viewers and everybody else.
///
/// `server_addresses` are the machine's own, as `machine::look` reports them.
pub fn sift(requests: &[Request], server_addresses: &[String]) -> Sifted {
    let mut by_address: BTreeMap<String, Vec<&Request>> = BTreeMap::new();
    for r in requests {
        by_address.entry(r.client_ip.clone()).or_default().push(r);
    }

    let mut watchers = Vec::new();
    let mut set_aside = Vec::new();

    for (client_ip, mine) in by_address {
        if server_addresses.contains(&client_ip) {
            set_aside.push(SetAside {
                client_ip,
                why: NotAViewer::OurOwnCheck,
            });
            continue;
        }
        let watcher = assemble(&client_ip, &mine);
        if watcher.segments < SEGMENTS_TO_BE_A_VIEWER && watcher.bytes > 0 {
            set_aside.push(SetAside {
                client_ip,
                why: NotAViewer::TooLittle {
                    segments: watcher.segments,
                },
            });
            continue;
        }
        watchers.push(watcher);
    }

    // Busiest first: whoever pulled the most is who the person came to look at.
    watchers.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.client_ip.cmp(&b.client_ip)));
    Sifted {
        watchers,
        set_aside,
    }
}

fn assemble(client_ip: &str, mine: &[&Request]) -> Watcher {
    let mut segments = 0usize;
    let mut bytes = 0u64;
    let mut in_download_s = 0.0f64;
    let mut restarts = 0usize;
    let mut reinits = 0usize;
    let mut failures = 0usize;
    let mut numbers: Vec<u32> = Vec::new();
    let mut watching: Option<String> = None;
    let mut first = mine[0].at;
    let mut last = mine[0].at;

    for r in mine {
        if r.at < first {
            first = r.at;
        }
        if r.at > last {
            last = r.at;
        }
        if r.status >= 400 {
            failures += 1;
            // A failed request delivered nothing; counting its bytes or its seconds towards
            // the viewer's speed would make a broken serving look like a slow viewer.
            continue;
        }
        bytes = bytes.saturating_add(r.bytes);
        in_download_s += r.duration_s;

        let asked = what_was_asked_for(&r.path);
        if let Some(key) = asked.library_key() {
            watching.get_or_insert_with(|| key.to_owned());
        }
        match asked {
            Asked::Segment { .. } => {
                segments += 1;
                if let Some(n) = segment_number(&r.path) {
                    numbers.push(n);
                }
            }
            // One long pull of a whole film. Not segments, and the ratio below says nothing
            // about it — which is why a direct file gets no starving verdict at all.
            Asked::DirectFile { .. } => {}
            Asked::SetDescription { .. } => restarts += 1,
            Asked::SetInit { .. } => reinits += 1,
            Asked::RungPlaylist { .. } | Asked::Other => {}
        }
    }

    // The first reading of each is the normal one; only what comes after it means anything.
    let restarts = restarts.saturating_sub(1);
    let reinits = reinits.saturating_sub(1);

    let elapsed_s = (last - first).as_seconds_f64();
    let long_enough = elapsed_s >= SHORTEST_SPAN_S;

    let content_ratio = (long_enough && segments > 0)
        .then(|| segments as f64 * f64::from(SEGMENT_SECONDS) / elapsed_s);
    // **The wall clock.** See the module note: this is the figure that is their link.
    let mbit_s = long_enough.then(|| bytes as f64 * 8.0 / elapsed_s / 1_000_000.0);
    let in_download_mbit_s =
        (in_download_s > 0.0).then(|| bytes as f64 * 8.0 / in_download_s / 1_000_000.0);

    Watcher {
        client_ip: client_ip.to_owned(),
        watching,
        segments,
        bytes,
        first,
        last,
        elapsed_s,
        content_ratio,
        mbit_s,
        in_download_mbit_s,
        skipped: gaps(&mut numbers),
        restarts,
        reinits,
        failures,
    }
}

/// The segment number out of a path: `.../seg_00012.m4s` is 12.
///
/// Read off the name rather than counted, because counting cannot see what is missing — and
/// what is missing is the entire point.
pub fn segment_number(path: &str) -> Option<u32> {
    let name = path.rsplit('/').next()?;
    // Anchored on the prefix rather than on "the first digit in the name". `init.mp4` has a
    // digit in it, and reading that one made every fragmented set look as though it had asked
    // for segment four — a gap of three at the start of every single session.
    let after = name.strip_prefix(SEGMENT_PREFIX)?;
    let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// Which numbers between the first and the last were never asked for.
fn gaps(numbers: &mut Vec<u32>) -> Vec<u32> {
    if numbers.len() < 2 {
        return Vec::new();
    }
    numbers.sort_unstable();
    numbers.dedup();
    let mut missing = Vec::new();
    for pair in numbers.windows(2) {
        for n in (pair[0] + 1)..pair[1] {
            missing.push(n);
        }
    }
    missing
}

/// Work out what is most likely at fault, in the order the skill records.
///
/// **The order is the method.** First: is the server asleep? Low processor, little read from
/// the disk, a small amount going out — with a viewer hanging, that is the server saying it
/// is not the one at fault, and it comes first because it is the cheapest question and it
/// removes the largest suspect. Only then the viewer's link, and the file itself last of all.
///
/// `load` and `file` may both be absent; then the answer says less rather than guessing more.
pub fn explain(watcher: &Watcher, load: Option<&Load>, file: Option<&FileShape>) -> Verdict {
    let Some(ratio) = watcher.content_ratio else {
        return Verdict {
            cause: Cause::Unclear,
            say: Detail::new(DetailCode::StallsTooShort).with("seconds", watcher.elapsed_s),
        };
    };

    if ratio >= KEEPING_UP {
        // A gap between requests here is a full buffer, not a stall. Saying so plainly is
        // what keeps this from being the check that cries wolf on healthy viewers.
        return Verdict {
            cause: Cause::NothingWrong,
            say: Detail::new(DetailCode::StallsKeepingUp)
                .with("ratio", round2(ratio))
                .with("mbit_s", watcher.mbit_s.map(round2)),
        };
    }

    if let Some(load) = load {
        if load.capacity_mbit_s > 0.0 && load.out_mbit_s / load.capacity_mbit_s > SERVER_LINK_BUSY {
            return Verdict {
                cause: Cause::ServerLink,
                say: Detail::new(DetailCode::StallsServerLink)
                    .with("out_mbit_s", round2(load.out_mbit_s))
                    .with("capacity_mbit_s", round2(load.capacity_mbit_s)),
            };
        }
        // The disk reading hard while the cache is small is viewers spread out along the
        // timeline: each one needs a different part and none of it is in memory.
        if load.cache_small && load.disk_read_mb_s > 0.0 {
            return Verdict {
                cause: Cause::Disk,
                say: Detail::new(DetailCode::StallsDisk)
                    .with("disk_read_mb_s", round2(load.disk_read_mb_s))
                    .with("ratio", round2(ratio)),
            };
        }
    }

    // The file, but only when the link is demonstrably wide enough for the average and not
    // for the peaks. Reached last, and only with both numbers in hand.
    if let (Some(file), Some(mbit)) = (file, watcher.mbit_s) {
        if mbit >= file.average_mbit && mbit < file.peak_10s_mbit {
            return Verdict {
                cause: Cause::TheFileItself,
                say: Detail::new(DetailCode::StallsFilePeaks)
                    .with("mbit_s", round2(mbit))
                    .with("average_mbit", round2(file.average_mbit))
                    .with("peak_10s_mbit", round2(file.peak_10s_mbit)),
            };
        }
    }

    Verdict {
        cause: Cause::ViewerLink,
        say: Detail::new(DetailCode::StallsViewerLink)
            .with("ratio", round2(ratio))
            .with("mbit_s", watcher.mbit_s.map(round2))
            .with("in_download_mbit_s", watcher.in_download_mbit_s.map(round2))
            .with("skipped", watcher.skipped.len() as u64)
            .with("restarts", watcher.restarts as u64),
    }
}

/// Two places after the point. The core still hands over a number, not a string — which of
/// the two separators to write is the interface's business, and rounding here only keeps
/// 15.899999999999999 out of the report.
fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
