//! T265 — asking what a domain points at, without believing a stale "no".
//!
//! **Why not the machine's own resolver.** Two reasons, and the second was found by running
//! it rather than by thinking about it.
//!
//! The first is the one FR-139 names: a stub resolver caches the *negative* answer. A person
//! creates the record, presses "check", and is told the domain does not exist — because we
//! asked a minute before they created it, and the answer is held for as long as the zone's
//! settings say, commonly an hour. They see a failure where everything is in order, and the
//! only cure is waiting without knowing what for.
//!
//! The second: **a stub resolver may not answer the questions this needs at all.** The
//! resolver on the machine this was written on (172.19.0.2, handed out under WSL) returns
//! nothing for NS and nothing for SOA. That is not a fault, it is ordinary — a great many
//! stubs and home routers forward A and AAAA and nothing else. Building on one would work
//! here and fail on somebody else's machine, with a message about their domain that was
//! really about their router.
//!
//! So this walks down from the root servers itself, the way every real resolver does. It is
//! not a third-party service: the root is where all resolution starts, and the alternative —
//! sending the person's domain to somebody's public resolver — would be one.
//!
//! **What was tried and dropped.** The first shape of this also worked out which zone the
//! name belongs to, so a refusal could say "add a record to remingston.ru" rather than name
//! the whole domain. Two ways were tried and both failed on their own terms: an NS query
//! through the recursor comes back as a referral rather than an answer, and the library's
//! negative answer carries neither the SOA nor the authority section (`soa: None`,
//! `authorities: None` on every case measured). The zone turned out not to be needed: the
//! person is told the exact record to create and the exact value to give it, and that is
//! actionable without naming the zone it lives in.
//!
//! Nothing here judges. Whether the records point at the right place, and whether they agree
//! with the choice about IPv6, is `domain::dns_verdict`'s work — and being separate is what
//! lets that be tested without a network.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;
use std::time::{Duration, Instant};

use hickory_resolver::proto::op::Query;
use hickory_resolver::proto::rr::{Name, RData, RecordType};
use hickory_resolver::recursor::{Recursor, RecursorError, RecursorOptions};

use crate::domain::dns_verdict::Records;

/// How long to keep asking before saying the domain is not attached.
///
/// Thirty seconds, and not more, on purpose. A record can take minutes to spread, and waiting
/// that out inside one command would look like the application had hung. The screen offers to
/// ask again instead (FR-138, T293) — which is the same waiting, with the person able to see
/// it and to stop.
pub const DEFAULT_PATIENCE: Duration = Duration::from_secs(30);

/// The first pause between attempts. Each is twice the last, up to [`LONGEST_PAUSE`].
const FIRST_PAUSE: Duration = Duration::from_secs(1);
const LONGEST_PAUSE: Duration = Duration::from_secs(8);

/// The root name servers.
///
/// Written down rather than fetched, because fetching them would need a resolver, which is
/// the thing being built. This is the ordinary way — every resolver ships this list — and the
/// addresses move perhaps once a decade; the last change was b.root-servers.net in 2023.
/// Should one of them be unreachable, the others answer.
///
/// Both families on purpose: a machine with no IPv4 route reaches the v6 ones and the other
/// way about, and a list of one family would fail on perfectly good networks.
const ROOT_SERVERS: [&str; 26] = [
    "198.41.0.4",
    "2001:503:ba3e::2:30", // a
    "170.247.170.2",
    "2801:1b8:10::b", // b
    "192.33.4.12",
    "2001:500:2::c", // c
    "199.7.91.13",
    "2001:500:2d::d", // d
    "192.203.230.10",
    "2001:500:a8::e", // e
    "192.5.5.241",
    "2001:500:2f::f", // f
    "192.112.36.4",
    "2001:500:12::d0d", // g
    "198.97.190.53",
    "2001:500:1::53", // h
    "192.36.148.17",
    "2001:7fe::53", // i
    "192.58.128.30",
    "2001:503:c27::2:30", // j
    "193.0.14.129",
    "2001:7fd::1", // k
    "199.7.83.42",
    "2001:500:9f::42", // l
    "202.12.27.33",
    "2001:dc3::35", // m
];

/// Why nothing could be found out.
///
/// A code with what is needed to explain it; the wordings live in the interface's
/// dictionaries. Note what is *not* here: "there is no such record" is not a problem, it is an
/// answer — an empty [`Records`] — and it is judged with the rest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// The name is not one that can be looked up at all.
    NotAName { detail: String },
    /// The asking went wrong — a network in the way, or servers that are down. Told apart
    /// from "there is no such record" deliberately: one is the person's to fix and the other
    /// is not, and sending them to edit a record that was never wrong is the worse mistake.
    NoAnswer { detail: String },
}

fn name_of(text: &str) -> Result<Name, Problem> {
    Name::from_str(&format!("{}.", text.trim().trim_end_matches('.'))).map_err(|e| {
        Problem::NotAName {
            detail: format!("{text}: {e}"),
        }
    })
}

fn roots() -> Vec<IpAddr> {
    ROOT_SERVERS
        .iter()
        .filter_map(|text| text.parse().ok())
        .collect()
}

/// Walk down from the root and ask the servers that hold the zone.
///
/// An empty result is a result: it means the record is not there, which is what the person
/// has to be told and what they can act on.
pub async fn look_up(domain: &str, patience: Duration) -> Result<Records, Problem> {
    let wanted = name_of(domain)?;

    // **The point of the whole exercise.** With a cache of answers, our own second attempt
    // would be answered by our own first one, and the growing pause below would be thirty
    // seconds of asking ourselves. The cache of name servers is left alone: those are
    // positive, long-lived records, and re-walking from the root every time would be slow and
    // would tell us nothing new.
    let options = RecursorOptions {
        response_cache_size: 0,
        ..RecursorOptions::default()
    };

    let recursor = Recursor::with_options(
        &roots(),
        options,
        hickory_resolver::net::runtime::TokioRuntimeProvider::default(),
    )
    .map_err(|e| Problem::NoAnswer {
        detail: format!("the walk from the root could not be set up: {e}"),
    })?;

    // Keep asking while there is nothing — a record spreads through the network in minutes,
    // and one "no" is not an answer (FR-138).
    let deadline = Instant::now() + patience;
    let mut pause = FIRST_PAUSE;
    let mut trouble: Option<String> = None;

    loop {
        let mut a: Vec<Ipv4Addr> = Vec::new();
        let mut aaaa: Vec<Ipv6Addr> = Vec::new();

        for kind in [RecordType::A, RecordType::AAAA] {
            match ask(&recursor, wanted.clone(), kind).await {
                Said::Addresses {
                    a: found4,
                    aaaa: found6,
                } => {
                    a.extend(found4);
                    aaaa.extend(found6);
                }
                Said::Nothing => {}
                Said::Trouble { detail } => trouble = Some(detail),
            }
        }

        if !a.is_empty() || !aaaa.is_empty() {
            return Ok(Records { a, aaaa });
        }

        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        tokio::time::sleep(pause.min(left)).await;
        pause = (pause * 2).min(LONGEST_PAUSE);
    }

    // Out of patience with nothing found. If every attempt ran into trouble rather than into
    // an honest "there is no such record", that is a different thing and must not be reported
    // as an unattached domain.
    if let Some(detail) = trouble {
        return Err(Problem::NoAnswer { detail });
    }
    Ok(Records::default())
}

/// What one query came back with.
enum Said {
    Addresses {
        a: Vec<Ipv4Addr>,
        aaaa: Vec<Ipv6Addr>,
    },
    /// There is no such record. An answer, not a failure.
    Nothing,
    Trouble {
        detail: String,
    },
}

/// One query, and what it means.
async fn ask<P: hickory_resolver::ConnectionProvider>(
    recursor: &Recursor<P>,
    name: Name,
    kind: RecordType,
) -> Said {
    match recursor
        .resolve(Query::query(name, kind), Instant::now(), false)
        .await
    {
        Ok(message) => {
            let mut a = Vec::new();
            let mut aaaa = Vec::new();
            for record in &message.answers {
                match &record.data {
                    RData::A(found) => a.push(found.0),
                    RData::AAAA(found) => aaaa.push(found.0),
                    // An answer reached through a chain of aliases carries the aliases in the
                    // same section. They are not addresses and are not what was asked about.
                    _ => {}
                }
            }
            if a.is_empty() && aaaa.is_empty() {
                Said::Nothing
            } else {
                Said::Addresses { a, aaaa }
            }
        }
        // The one failure that is really an answer. Everything else — a timeout, a network
        // fault, a recursion limit — is trouble, and trouble reported as an absence sends the
        // person off to edit a record that was never wrong.
        Err(RecursorError::Negative(_)) => Said::Nothing,
        Err(e) => Said::Trouble {
            detail: e.to_string(),
        },
    }
}
