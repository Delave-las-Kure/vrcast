//! T304 — a stretch of the serving's log, reduced to what a person can read (FR-071).
//!
//! **The recorded trap: a request longer than thirty seconds is usually fine.** It is a long
//! range fetch — a player asking for a big piece of a film and taking its time over it — and
//! on a healthy server most of the long requests are exactly that. Counting them as faults
//! produces a screen full of red on a machine that is working perfectly, and after the second
//! time nobody reads the screen. So length alone is never a complaint here: it becomes one
//! only together with a delivered rate too low to be anything else.
//!
//! **206 should dominate.** Every player asks in ranges, so a log where 200 outnumbers 206 is
//! saying that ranges are not being served — the film plays and seeking does not, which is a
//! complaint that arrives as "it is broken" and has nothing to do with the network.
//!
//! The counting is over what was *parsed*: lines that yielded nothing are counted separately
//! rather than dropped, because a log of nothing but unreadable lines means the serving is
//! writing something else entirely, and that has to be sayable.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use super::access_log::Request;

/// Above this, a request counts as long. Carried over from the skill unchanged.
pub const LONG_REQUEST_S: f64 = 30.0;

/// Below this delivered rate, in megabits per second, a long request stops being ordinary.
///
/// **A choice, and a deliberately timid one.** The lightest rung the application will ever
/// build has a floor of one megabit (`domain::ladder`), so anything delivering below that is
/// slower than the slowest thing there is to serve — no reading of it is innocent. A tighter
/// figure would catch more, and would also catch players that are simply pacing themselves,
/// which is the mistake this whole module is written around.
pub const SLOW_DELIVERY_MBIT: f64 = 1.0;

/// How many entries the "top" lists hold. Enough to see a pattern, short enough to read.
pub const TOP_N: usize = 5;

/// A path and how often it was asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counted {
    pub what: String,
    pub times: usize,
}

/// A request that failed, and how many like it there were.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    pub status: u16,
    pub path: String,
    pub times: usize,
}

/// A request that took a long time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LongRequest {
    pub client_ip: String,
    pub path: String,
    pub seconds: f64,
    pub bytes: u64,
    /// What it actually delivered while it ran.
    pub mbit_s: f64,
    /// Whether this one is worth a look. See the module note: length alone is not enough.
    pub slow: bool,
}

/// What a stretch of log adds up to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Digest {
    /// How many lines went in, readable or not.
    pub lines: usize,
    /// How many of them turned into a request.
    pub requests: usize,
    /// How many yielded nothing. See the module note.
    pub unreadable: usize,
    pub by_status: BTreeMap<u16, usize>,
    /// How many different addresses appeared. Not "viewers": sifting the ones that are not
    /// viewers is `domain::stalls`'s job, and doing it in two places would mean two answers.
    pub addresses: usize,
    pub top_paths: Vec<Counted>,
    pub top_addresses: Vec<Counted>,
    pub failures: Vec<Failure>,
    pub long_requests: Vec<LongRequest>,
    pub bytes_out: u64,
    /// The first and last moment the stretch covers, by the server's clock.
    pub from: Option<OffsetDateTime>,
    pub to: Option<OffsetDateTime>,
}

impl Digest {
    /// Whether ranges are being served at all — the thing 206 dominating actually means.
    ///
    /// `None` when nothing succeeded, which is not the same as "no" and must not read as it.
    pub fn ranges_dominate(&self) -> Option<bool> {
        let ranged = self.by_status.get(&206).copied().unwrap_or(0);
        let whole = self.by_status.get(&200).copied().unwrap_or(0);
        if ranged + whole == 0 {
            return None;
        }
        Some(ranged > whole)
    }

    /// How many long requests are worth a look, as opposed to merely long.
    pub fn slow_long_requests(&self) -> usize {
        self.long_requests.iter().filter(|r| r.slow).count()
    }

    /// Everything that came back 4xx or 5xx.
    pub fn failed(&self) -> usize {
        self.by_status
            .iter()
            .filter(|(status, _)| **status >= 400)
            .map(|(_, times)| times)
            .sum()
    }
}

/// Reduce a stretch of log.
///
/// `unreadable` is passed in rather than worked out here: the reading of lines happens where
/// the file is, and a line that failed to parse never reaches this layer as anything.
pub fn digest(requests: &[Request], unreadable: usize) -> Digest {
    let mut by_status: BTreeMap<u16, usize> = BTreeMap::new();
    let mut paths: BTreeMap<String, usize> = BTreeMap::new();
    let mut addresses: BTreeMap<String, usize> = BTreeMap::new();
    let mut failures: BTreeMap<(u16, String), usize> = BTreeMap::new();
    let mut long_requests = Vec::new();
    let mut bytes_out: u64 = 0;
    let mut from: Option<OffsetDateTime> = None;
    let mut to: Option<OffsetDateTime> = None;

    for r in requests {
        *by_status.entry(r.status).or_default() += 1;
        *paths.entry(r.path.clone()).or_default() += 1;
        *addresses.entry(r.client_ip.clone()).or_default() += 1;
        bytes_out = bytes_out.saturating_add(r.bytes);

        if r.status >= 400 {
            *failures.entry((r.status, r.path.clone())).or_default() += 1;
        }

        from = Some(from.map_or(r.at, |f| if r.at < f { r.at } else { f }));
        to = Some(to.map_or(r.at, |t| if r.at > t { r.at } else { t }));

        if r.duration_s > LONG_REQUEST_S {
            let mbit_s = if r.duration_s > 0.0 {
                r.bytes as f64 * 8.0 / r.duration_s / 1_000_000.0
            } else {
                0.0
            };
            long_requests.push(LongRequest {
                client_ip: r.client_ip.clone(),
                path: r.path.clone(),
                seconds: r.duration_s,
                bytes: r.bytes,
                mbit_s,
                slow: mbit_s < SLOW_DELIVERY_MBIT,
            });
        }
    }

    // Longest first: if there is anything to look at here, it is at the top of that order.
    long_requests.sort_by(|a, b| b.seconds.total_cmp(&a.seconds));
    long_requests.truncate(TOP_N);

    let mut failures: Vec<Failure> = failures
        .into_iter()
        .map(|((status, path), times)| Failure {
            status,
            path,
            times,
        })
        .collect();
    failures.sort_by(|a, b| b.times.cmp(&a.times).then(a.status.cmp(&b.status)));

    Digest {
        lines: requests.len() + unreadable,
        requests: requests.len(),
        unreadable,
        by_status,
        addresses: addresses.len(),
        top_paths: top(paths),
        top_addresses: top(addresses),
        failures,
        long_requests,
        bytes_out,
        from,
        to,
    }
}

/// The busiest few, most first. Ties broken by name so the same log always reduces the same
/// way — a list that reorders itself between two identical runs reads as movement.
fn top(counts: BTreeMap<String, usize>) -> Vec<Counted> {
    let mut all: Vec<Counted> = counts
        .into_iter()
        .map(|(what, times)| Counted { what, times })
        .collect();
    all.sort_by(|a, b| b.times.cmp(&a.times).then(a.what.cmp(&b.what)));
    all.truncate(TOP_N);
    all
}
