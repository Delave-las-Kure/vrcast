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
    /// The rung of a quality set they pulled most segments from (`v9`), when they watched a
    /// set (T705).
    #[serde(default)]
    pub rung: Option<String>,
    /// What that rung of **that** film needs, in Mbit/s — its BANDWIDTH in the set's own
    /// description on the server (T705). Filled in by the command, which can read the server;
    /// `None` when the film or rung is not known, or its description could not be read.
    ///
    /// The one figure a verdict about the viewer's link may stand on. Before it, a viewer
    /// behind real time with nothing else explaining it was told their link was too thin —
    /// with no number for what the link would have had to carry (QA-26 №12).
    #[serde(default)]
    pub need_mbit: Option<f64>,
    /// What their own connection said while the diagnosis was asked (T711) — `None` when they
    /// had none open in both readings, or the server could not be asked.
    #[serde(default)]
    pub live: Option<LiveLink>,
}

/// T711 — a viewer's link as their live connection shows it: the `ss -tin` the Viewers
/// screen reads, taken twice, [`crate::server::health::SAMPLE_S`] apart.
///
/// **What the log cannot tell, this can.** The log says how fast the server put a piece into
/// its socket buffers, not when the viewer had it — measured 2026-10-09, a viewer held to
/// 0.4 Mbit/s showed 6.16 "inside the downloads". The connection itself says what the viewer's
/// side confirmed, how long there was something on its way to them, and how much of that time
/// their side was full and taking nothing more. A full side is the player not taking what
/// arrived; a connection working all the time and carrying less than the film needs is the
/// link.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LiveLink {
    /// Seconds between the two readings.
    pub span_s: f64,
    /// What reached the viewer over the stretch, by what their side confirmed, Mbit/s.
    pub mbit_s: f64,
    /// What the connection carried while it had something to carry, Mbit/s. `None` when it
    /// had nothing on its way at all, or `ss` did not say.
    pub busy_mbit_s: Option<f64>,
    /// Share of the stretch with something on its way to them, 0 to 1.
    pub busy_share: Option<f64>,
    /// Share of that busy time their side was full, 0 to 1.
    pub held_share: Option<f64>,
    /// Share of what was sent in the stretch that had to be sent again, 0 to 1.
    pub resent_share: Option<f64>,
}

/// Above this share of the stretch with something on its way, the connection was working,
/// and what it carried is an answer about the link (T711). **A choice**: below it the viewer
/// was mostly not asking — a full buffer now, perhaps, while the log's minutes say behind —
/// and the live reading says nothing either way; the log's reasoning stands.
pub const LIVE_BUSY: f64 = 0.5;

/// Below this share of the stretch with something on its way, the connection hardly sent at
/// all, and how much of its sending time the viewer's side was full is too little to go on
/// (T711). **A choice**: half a second in the five.
pub const LIVE_SENDING: f64 = 0.1;

/// Above this share of the busy time with the viewer's side full, it is the player that is
/// not taking what arrives. The Viewers screen's own threshold for the same field.
pub const LIVE_HELD: f64 = 0.5;

/// Each address's [`LiveLink`] from two readings of the connection table (T711).
///
/// Only connections present in **both** are counted, matched by address and port: a
/// connection opened or closed in between has no difference to take, and summing what the
/// two readings saw would count a new connection's whole life as five seconds' delivery.
pub fn live_links(
    before: &super::connections::Poll,
    after: &super::connections::Poll,
) -> BTreeMap<String, LiveLink> {
    let span_s = (after.at - before.at).as_seconds_f64();
    let mut out = BTreeMap::new();
    if span_s < 1.0 {
        return out;
    }
    #[derive(Default)]
    struct Sum {
        bytes: u64,
        segs: u64,
        resent: u64,
        busy_ms: Option<u64>,
        held_ms: Option<u64>,
        unknown_busy: bool,
    }
    let mut sums: BTreeMap<&str, Sum> = BTreeMap::new();
    for now in &after.rows {
        let Some(then) = before
            .rows
            .iter()
            .find(|r| r.peer_ip == now.peer_ip && r.peer_port == now.peer_port)
        else {
            continue;
        };
        let sum = sums.entry(now.peer_ip.as_str()).or_default();
        sum.bytes += now.bytes_acked.saturating_sub(then.bytes_acked);
        sum.segs += now.segs_out.saturating_sub(then.segs_out);
        sum.resent += now.retrans_total.saturating_sub(then.retrans_total);
        match (then.busy_ms, now.busy_ms) {
            (Some(a), Some(b)) => {
                *sum.busy_ms.get_or_insert(0) += b.saturating_sub(a);
                let held = now
                    .rwnd_limited_ms
                    .unwrap_or(0)
                    .saturating_sub(then.rwnd_limited_ms.unwrap_or(0));
                *sum.held_ms.get_or_insert(0) += held;
            }
            _ => sum.unknown_busy = true,
        }
    }
    for (ip, sum) in sums {
        let busy_ms = sum.busy_ms.filter(|_| !sum.unknown_busy);
        let busy_s = busy_ms.map(|ms| ms as f64 / 1000.0);
        out.insert(
            ip.to_owned(),
            LiveLink {
                span_s,
                mbit_s: sum.bytes as f64 * 8.0 / span_s / 1_000_000.0,
                busy_mbit_s: busy_s
                    .filter(|s| *s > 0.0)
                    .map(|s| sum.bytes as f64 * 8.0 / s / 1_000_000.0),
                // Several connections may each be busy at once; the stretch is not longer
                // for it.
                busy_share: busy_s.map(|s| (s / span_s).min(1.0)),
                held_share: busy_ms
                    .filter(|ms| *ms > 0)
                    .map(|ms| (sum.held_ms.unwrap_or(0) as f64 / ms as f64).min(1.0)),
                resent_share: (sum.segs > 0).then(|| sum.resent as f64 / sum.segs as f64),
            },
        );
    }
    out
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileShape {
    pub average_mbit: f64,
    /// The peak over a ten-second window. This is the one that hangs a player.
    pub peak_10s_mbit: f64,
    /// Which film on the server the measured file is (T705). The shape is a fact about one
    /// film, and is applied to the viewers of that film only — never to everybody in the log:
    /// a file measured on this computer says nothing about a viewer watching something else.
    /// `None` — not said — and then it is applied to nobody.
    #[serde(default)]
    pub slug: Option<String>,
}

impl FileShape {
    /// Whether this measurement is about what `watcher` is watching.
    pub fn is_about(&self, watcher: &Watcher) -> bool {
        matches!((&self.slug, &watcher.watching), (Some(a), Some(b)) if a == b)
    }
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
    /// Not the link at all: it carries what the film needs whenever it is carrying anything,
    /// and the viewer is not asking in between (T482).
    ///
    /// ⚠ **Seen from the server's side only, and said so** (T706). Measured on the container
    /// 2026-10-09: a viewer held to 50 kB/s (0.4 Mbit/s) showed 6.16 Mbit/s "inside the
    /// downloads" against a rung needing 2 — the server finishes a request when the last byte
    /// is in the socket buffers, not when the viewer has it. A slow link and a player that
    /// stops asking look the same from the log, and the wording names both.
    ThePlayer,
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
    let mut by_rung: BTreeMap<String, usize> = BTreeMap::new();
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
            Asked::Segment { ref rung, .. } => {
                segments += 1;
                *by_rung.entry(rung.clone()).or_default() += 1;
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
        // The rung most of their segments came from — ties to the name, so the answer does
        // not change from one run to the next.
        rung: by_rung
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
            .map(|(rung, _)| rung),
        need_mbit: None,
        live: None,
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

    // T711 — the player, seen on the viewer's own connection: whenever it had something on
    // its way to them, their side was full and taking nothing for most of it. What arrived
    // was not taken, so neither the link nor the file's peaks are what holds them up. Before
    // the file: the peaks verdict presumes the link is the narrow place.
    //
    // Asked of any connection that sent at all ([`LIVE_SENDING`]), not only of a busy one:
    // measured 2026-10-09, a slow reader's connection was busy a fifth of the stretch and
    // held for nine tenths of that — a full side stops the sending, which is why it is
    // rarely busy.
    let sending = watcher
        .live
        .as_ref()
        .filter(|l| l.busy_share.is_some_and(|b| b >= LIVE_SENDING));
    if let Some(held) = sending
        .and_then(|l| l.held_share)
        .filter(|h| *h > LIVE_HELD)
    {
        return Verdict {
            cause: Cause::ThePlayer,
            say: Detail::new(DetailCode::StallsPlayerLive)
                .with("ratio", round2(ratio))
                .with("held_pct", (held * 100.0).round() as u64)
                .with("live_mbit", sending.map(|l| round2(l.mbit_s))),
        };
    }
    let live = watcher
        .live
        .as_ref()
        .filter(|l| l.busy_share.is_some_and(|b| b >= LIVE_BUSY));

    // From here on only a measurement **of the film this viewer is watching** may be used
    // (T705): a file measured on this computer is applied to the viewers of that film and to
    // nobody else.
    let file = file.filter(|f| f.is_about(watcher));

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

    // What this viewer's film needs: the rung they are on, read off that film's own set on
    // the server (T705); failing that, the measured file of that same film. Without either
    // there is no number for what their link would have to carry, and no verdict about the
    // link or the player can stand.
    let need = watcher.need_mbit.or(file.map(|f| f.average_mbit));

    // T711 — the link, seen on the viewer's own connection: it had something on its way most
    // of the stretch and their side was not the one holding it, so what it carried while
    // busy is what their link carries. Against what the rung needs, that settles the
    // question the log could only put as "the player, or a link slower than we can see".
    if let (Some(live), Some(need)) = (live, need) {
        if let Some(carried) = live.busy_mbit_s {
            let say = |code| {
                Detail::new(code)
                    .with("ratio", round2(ratio))
                    .with("live_mbit", round2(carried))
                    .with("need_mbit", round2(need))
            };
            return if carried >= need {
                Verdict {
                    cause: Cause::ThePlayer,
                    say: say(DetailCode::StallsLinkFineLive),
                }
            } else {
                Verdict {
                    cause: Cause::ViewerLink,
                    say: say(DetailCode::StallsViewerLinkLive).with(
                        "resent_pct",
                        // Nothing sent in the stretch is nothing sent again.
                        round2(live.resent_share.unwrap_or(0.0) * 100.0),
                    ),
                }
            };
        }
    }

    // ⚠ **Not the link, when the link demonstrably carries it.** Measured on the stand
    // 2026-09-04: a viewer at a ratio of 0.39 was told their link was too thin while the
    // speed *inside* their downloads was 30.35 Mbit/s against a film needing 4 — a hundred
    // and sixty times over. The link was fine and they were not asking; the answer sent them
    // to argue with their provider. There was no cause for this at all, and the catch-all
    // took it.
    //
    // **The comparison is the film's own figure, not a threshold of ours.** If what arrives
    // while anything is arriving would keep up with the film, the shortfall is in the gaps —
    // a player that has stopped, a decoder that cannot keep pace, somebody who pressed pause.
    if let (Some(need), Some(in_download)) = (need, watcher.in_download_mbit_s) {
        if in_download >= need {
            return Verdict {
                cause: Cause::ThePlayer,
                say: Detail::new(DetailCode::StallsThePlayer)
                    .with("ratio", round2(ratio))
                    .with("mbit_s", watcher.mbit_s.map(round2))
                    .with("in_download_mbit_s", round2(in_download))
                    .with("average_mbit", round2(need))
                    .with("restarts", watcher.restarts as u64)
                    .with("skipped", watcher.skipped.len() as u64),
            };
        }
    }

    // ⚠ **The link, only with the number it fails to reach** (T705, QA-26 №12). This was the
    // catch-all: any viewer behind real time whom nothing else explained was told their link
    // was not enough — a viewer with a speed of 0 measured over two requests among them. Now
    // it takes what the film needs, and their speed measured under it.
    let speed = watcher.in_download_mbit_s.or(watcher.mbit_s);
    let (Some(need), Some(true)) = (need, speed.map(|s| need.is_some_and(|n| s < n))) else {
        return Verdict {
            cause: Cause::Unclear,
            say: Detail::new(DetailCode::StallsUnclear)
                .with("ratio", round2(ratio))
                .with("mbit_s", watcher.mbit_s.map(round2))
                .with("need_mbit", need.map(round2)),
        };
    };
    Verdict {
        cause: Cause::ViewerLink,
        say: Detail::new(DetailCode::StallsViewerLink)
            .with("ratio", round2(ratio))
            .with("mbit_s", watcher.mbit_s.map(round2))
            .with("in_download_mbit_s", watcher.in_download_mbit_s.map(round2))
            .with("need_mbit", round2(need))
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
