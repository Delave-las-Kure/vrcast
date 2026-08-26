//! T158, T159, T160 — the two sources brought together into a list of viewers.
//!
//! The rule is R-02's: a connection is attributed to a medium by the address and that
//! address's most recent request. Neither source can do it alone — the log knows what is
//! being watched but not that it is being watched now; the connection table knows who is
//! pulling but not what.
//!
//! **Where this cannot know, and says so.** A film served as a single file is one request
//! that lasts the whole showing, and the serving writes its line only when it ends. Until
//! some request from that address has been recorded, the viewer is in the list with what
//! they are watching still unknown. That is the honest state, and it is shown as itself:
//! the alternative would be either hiding a viewer who is plainly there, or naming a medium
//! we have not been told. In practice players ask for the first bytes before settling in to
//! pull, and that request is recorded at once.
//!
//! Only the rules are here. Asking the server lives in `server::viewers`.

use std::collections::HashMap;
use std::collections::VecDeque;

use time::{Duration, OffsetDateTime};

use super::access_log::{Asked, Request};
use super::connections::ConnectionRow;

/// How long after its last sign of life an address stops counting as a viewer.
///
/// Thirty seconds, from the specification's assumptions, and adjustable in the settings
/// (FR-055). The two ways of being wrong are not symmetrical: too short a threshold puts
/// out a viewer who has paused, and they come back a moment later — a list that flickers
/// is read as broken. Too long leaves someone who has gone, which merely dates the figure.
/// So the default errs towards patience.
pub const DEFAULT_ACTIVITY_THRESHOLD_S: u64 = 30;

/// Over how long a stretch the delivered speed is worked out.
///
/// Not between two neighbouring polls: at a few seconds apart the difference is mostly
/// noise, and a viewer would be marked as struggling and healthy by turns. Not over the
/// whole showing either — an hour of good delivery would hide a link that went bad five
/// minutes ago.
const SPEED_WINDOW_S: u64 = 30;

/// The shortest stretch a speed may be worked out over.
///
/// Below this the figure means nothing, and no speed at all is shown rather than a made-up
/// one. A viewer who has just appeared has no speed yet, and saying so is honest.
const SPEED_MIN_SPAN_S: f64 = 5.0;

/// What share of the segments sent has to be sent again before the link counts as lossy.
///
/// Every link loses something; a flag that fires on any loss at all would be lit for
/// everyone and would mean nothing.
const RETRANSMIT_SHARE: f64 = 0.02;

/// Above this share of the busy time spent waiting for the far end's window, the flow is
/// the player's doing rather than the link's — see [`Problem::SlowLink`].
const RECEIVER_LIMITED: f64 = 0.5;

/// How long the pulling may stand still before it counts as stuck.
const STALL_S: i64 = 15;

/// Why a viewer is marked as having trouble (FR-053).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Problem {
    /// Getting less than what they are being sent needs.
    SlowLink,
    /// A lossy link: much of what is sent has to be sent again.
    Retransmits,
    /// The pulling has stopped although the connection is open.
    Stalls,
}

/// What the library knows about the thing being served.
///
/// Supplied from outside because the library lives on the server, and nothing here is
/// allowed to go and look.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VariantFacts {
    /// Which medium it belongs to.
    pub media_id: Option<String>,
    /// What to call the variant to a person: the file, or the rung.
    pub variant: Option<String>,
    /// What the variant needs to arrive in time. Compared against the delivered speed —
    /// this is the whole of FR-053.
    pub required_bps: Option<u64>,
}

/// Where the library facts are asked for.
pub trait VariantLookup {
    fn facts(&self, asked: &Asked) -> VariantFacts;
}

impl<F: Fn(&Asked) -> VariantFacts> VariantLookup for F {
    fn facts(&self, asked: &Asked) -> VariantFacts {
        self(asked)
    }
}

/// A viewer, as shown.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Viewer {
    pub ip: String,
    /// Where from. `None` means "not determined" and is shown as that — never guessed at
    /// (FR-052).
    pub country: Option<String>,
    pub city: Option<String>,
    pub asn_org: Option<String>,
    /// What is being watched. `None` while no request from this address has been recorded.
    pub media_id: Option<String>,
    pub variant: Option<String>,
    /// The speed it is arriving at. `None` until there is enough to work it out from.
    pub delivery_bps: Option<u64>,
    /// The speed it needs.
    pub required_bps: Option<u64>,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub last_seen_at: OffsetDateTime,
    pub problems: Vec<Problem>,
}

impl Viewer {
    /// How long they have been watching.
    pub fn watching_for(&self) -> Duration {
        self.last_seen_at - self.started_at
    }
}

/// One measurement of a connection.
#[derive(Debug, Clone, Copy)]
struct Sample {
    at: OffsetDateTime,
    bytes_acked: u64,
    segs_out: u64,
    retrans_total: u64,
}

/// What is being kept about one address while it watches.
#[derive(Debug)]
struct Tracked {
    started_at: OffsetDateTime,
    last_seen_at: OffsetDateTime,
    /// What the address most recently asked for. This is the attribution rule (R-02).
    facts: VariantFacts,
    /// The measurements the speed and the losses are worked out from.
    samples: VecDeque<Sample>,
    /// The most recent share of time spent waiting for the far end.
    receiver_limited_share: Option<f64>,
    /// Where from. Filled in once — the address does not move.
    country: Option<String>,
    city: Option<String>,
    asn_org: Option<String>,
}

/// The viewers of one session.
///
/// The active ones are held in memory, the departed in the session's history — nothing here
/// is written down anywhere lasting (data model, `Viewer`).
#[derive(Debug)]
pub struct Session {
    threshold: Duration,
    tracked: HashMap<String, Tracked>,
    history: Vec<Viewer>,
}

impl Default for Session {
    fn default() -> Self {
        Self::new(Duration::seconds(DEFAULT_ACTIVITY_THRESHOLD_S as i64))
    }
}

impl Session {
    pub fn new(threshold: Duration) -> Self {
        Self {
            threshold,
            tracked: HashMap::new(),
            history: Vec::new(),
        }
    }

    /// Change the threshold without losing who is being watched.
    pub fn set_threshold(&mut self, threshold: Duration) {
        self.threshold = threshold;
    }

    /// Take in a line from the access log.
    ///
    /// A failed request tells us the address is alive but not what they are getting: naming
    /// a medium on the strength of a refusal would put a viewer in front of a film they
    /// were never shown.
    pub fn note_request(&mut self, request: &Request, lookup: &impl VariantLookup) {
        let asked = super::access_log::what_was_asked_for(&request.path);
        let entry = self.entry(&request.client_ip, request.at);
        entry.last_seen_at = entry.last_seen_at.max(request.at);

        if (200..400).contains(&request.status) && asked.library_key().is_some() {
            entry.facts = lookup.facts(&asked);
        }
    }

    /// Take in one poll of the connection table.
    ///
    /// `at` is the server's own clock, read in the same command as the table. This machine's
    /// clock will not do: the two disagree, and a viewer would come out as having started
    /// watching in the future.
    pub fn note_connections(&mut self, rows: &[ConnectionRow], at: OffsetDateTime) {
        // Several connections from one address are one viewer: a player opens more than one,
        // and showing them as several people watching the same thing would be a lie about
        // how many there are.
        let mut totals: HashMap<&str, (u64, u64, u64, Option<f64>)> = HashMap::new();
        for row in rows {
            let slot = totals.entry(row.peer_ip.as_str()).or_default();
            slot.0 += row.bytes_acked;
            slot.1 += row.segs_out;
            slot.2 += row.retrans_total;
            // The worst of them: if any one connection is being held up by the player, the
            // low speed is not the link's doing.
            slot.3 = match (slot.3, row.receiver_limited_share) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (a, b) => a.or(b),
            };
        }

        for (ip, (bytes_acked, segs_out, retrans_total, limited)) in totals {
            let entry = self.entry(ip, at);
            entry.last_seen_at = entry.last_seen_at.max(at);
            entry.receiver_limited_share = limited;
            entry.samples.push_back(Sample {
                at,
                bytes_acked,
                segs_out,
                retrans_total,
            });
            let window = Duration::seconds(SPEED_WINDOW_S as i64);
            while entry
                .samples
                .front()
                .is_some_and(|first| at - first.at > window)
                && entry.samples.len() > 2
            {
                entry.samples.pop_front();
            }
        }
    }

    fn entry(&mut self, ip: &str, at: OffsetDateTime) -> &mut Tracked {
        self.tracked
            .entry(ip.to_owned())
            .or_insert_with(|| Tracked {
                started_at: at,
                last_seen_at: at,
                facts: VariantFacts::default(),
                samples: VecDeque::new(),
                receiver_limited_share: None,
                country: None,
                city: None,
                asn_org: None,
            })
    }

    /// Attach where an address is from. Done once per address: it does not move.
    pub fn note_place(
        &mut self,
        ip: &str,
        country: Option<String>,
        city: Option<String>,
        asn_org: Option<String>,
    ) {
        if let Some(entry) = self.tracked.get_mut(ip) {
            entry.country = country;
            entry.city = city;
            entry.asn_org = asn_org;
        }
    }

    /// Whose place is not yet known.
    pub fn without_place(&self) -> Vec<String> {
        self.tracked
            .iter()
            .filter(|(_, t)| t.country.is_none() && t.city.is_none() && t.asn_org.is_none())
            .map(|(ip, _)| ip.clone())
            .collect()
    }

    /// Those watching now.
    pub fn active(&self, now: OffsetDateTime) -> Vec<Viewer> {
        let mut viewers: Vec<Viewer> = self
            .tracked
            .iter()
            .filter(|(_, t)| now - t.last_seen_at <= self.threshold)
            .map(|(ip, t)| t.as_viewer(ip, now))
            .collect();
        // A settled order: the list refreshes itself, and one that reshuffled on every
        // refresh could not be read, let alone clicked on.
        viewers.sort_by(|a, b| a.started_at.cmp(&b.started_at).then(a.ip.cmp(&b.ip)));
        viewers
    }

    /// Move those who have gone into the session's history (FR-055).
    ///
    /// Comes back with how many left, so that whoever calls can say something changed
    /// without comparing the lists.
    pub fn retire_gone(&mut self, now: OffsetDateTime) -> usize {
        let threshold = self.threshold;
        let gone: Vec<String> = self
            .tracked
            .iter()
            .filter(|(_, t)| now - t.last_seen_at > threshold)
            .map(|(ip, _)| ip.clone())
            .collect();
        for ip in &gone {
            if let Some(t) = self.tracked.remove(ip) {
                self.history.push(t.as_viewer(ip, now));
            }
        }
        gone.len()
    }

    /// Those who watched earlier in this session.
    pub fn history(&self) -> &[Viewer] {
        &self.history
    }
}

impl Tracked {
    fn as_viewer(&self, ip: &str, now: OffsetDateTime) -> Viewer {
        let delivery_bps = self.delivery_bps();
        Viewer {
            ip: ip.to_owned(),
            country: self.country.clone(),
            city: self.city.clone(),
            asn_org: self.asn_org.clone(),
            media_id: self.facts.media_id.clone(),
            variant: self.facts.variant.clone(),
            delivery_bps,
            required_bps: self.facts.required_bps,
            started_at: self.started_at,
            last_seen_at: self.last_seen_at,
            problems: self.problems(delivery_bps, now),
        }
    }

    /// The speed it is really arriving at, from the growth of what has been confirmed.
    ///
    /// Not from `delivery_rate`: on an application-limited flow that reports how fast the
    /// channel carries a burst, not how much reaches the viewer — see `connections`.
    fn delivery_bps(&self) -> Option<u64> {
        let first = self.samples.front()?;
        let last = self.samples.back()?;
        let span = (last.at - first.at).as_seconds_f64();
        if span < SPEED_MIN_SPAN_S {
            return None;
        }
        // A connection that was replaced by a new one starts counting from zero, and the
        // difference goes negative. That is not a speed of any kind, so nothing is shown
        // until the window has moved past it.
        let grown = last.bytes_acked.checked_sub(first.bytes_acked)?;
        Some(((grown as f64 * 8.0) / span) as u64)
    }

    fn problems(&self, delivery_bps: Option<u64>, now: OffsetDateTime) -> Vec<Problem> {
        let mut problems = Vec::new();
        let receiver_limited = self
            .receiver_limited_share
            .is_some_and(|share| share > RECEIVER_LIMITED);

        // Slow link. The exemption is the point of it: a player with a full buffer stops
        // reading, and the delivered speed drops right off — with nothing wrong at all.
        // Marking that as a bad link would light the flag for healthy viewers, and a flag
        // that cries wolf is worse than no flag.
        if let (Some(delivery), Some(required)) = (delivery_bps, self.facts.required_bps) {
            if delivery < required && !receiver_limited {
                problems.push(Problem::SlowLink);
            }
        }

        // A lossy link, measured over the window rather than over all time: losses from an
        // hour ago say nothing about how the viewer is doing now.
        if let (Some(first), Some(last)) = (self.samples.front(), self.samples.back()) {
            let sent = last.segs_out.saturating_sub(first.segs_out);
            let again = last.retrans_total.saturating_sub(first.retrans_total);
            if sent > 0 && (again as f64 / sent as f64) > RETRANSMIT_SHARE {
                problems.push(Problem::Retransmits);
            }
        }

        // Stuck: the connection is open, we are not being held up by the player, and
        // nothing has moved for a while.
        if let Some(last) = self.samples.back() {
            let moved_recently = self
                .samples
                .iter()
                .rev()
                .find(|s| s.bytes_acked < last.bytes_acked)
                .map(|s| s.at);
            let standing_since =
                moved_recently.unwrap_or(self.samples.front().map_or(now, |s| s.at));
            if !receiver_limited
                && self.samples.len() > 1
                && (now - standing_since).whole_seconds() >= STALL_S
            {
                problems.push(Problem::Stalls);
            }
        }

        problems
    }
}
