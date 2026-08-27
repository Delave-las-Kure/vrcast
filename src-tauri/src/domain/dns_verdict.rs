//! T258 — does the domain point at this server, and does it agree with the IPv6 choice?
//!
//! Asked **before** anything is changed (FR-137). A record that points somewhere else costs
//! nothing to find here and costs a half-configured server to find at the verifying step —
//! by which time packages are installed, the firewall is on and password logins are off.
//!
//! The IPv6 half is the part that goes wrong quietly. A deployment where IPv6 was kept but
//! the AAAA record leads elsewhere comes up, serves, and works for everybody except viewers
//! whose connection prefers IPv6 — which is most home connections in some countries and none
//! in others. Nobody reports it as "the server is broken"; they report that it is slow, or
//! that it does not work for them, and it looks like their problem.
//!
//! Nothing here resolves anything: it is handed the records and the addresses and judges.
//! Finding them out — with a growing pause, and going round the negative cache (FR-138,
//! FR-139) — is `net::dns`'s work.

use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, Ipv6Addr};

/// What the person chose about IPv6 (FR-135).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ipv6Choice {
    /// Keep it on, and protect it exactly as IPv4 is protected (FR-136).
    Keep,
    /// Turn it off, so that the serving does not answer on it at all.
    Disable,
}

/// What the domain's records say.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Records {
    pub a: Vec<Ipv4Addr>,
    pub aaaa: Vec<Ipv6Addr>,
}

/// Where the server actually is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerAddresses {
    pub v4: Option<Ipv4Addr>,
    /// `None` when the machine has no IPv6 address of its own. Then there is nothing to keep
    /// and nothing an AAAA record could correctly point at.
    pub v6: Option<Ipv6Addr>,
}

/// Which record is being talked about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecordKind {
    A,
    Aaaa,
}

/// What is wrong with the IPv6 side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ipv6Problem {
    /// IPv6 is to be kept, and there is no AAAA record.
    Missing,
    /// IPv6 is to be kept, and the AAAA record leads elsewhere.
    PointsElsewhere { to: Vec<String> },
    /// IPv6 is to be turned off, and an AAAA record exists. Left there, the domain goes on
    /// promising an address that will stop answering, and the failure looks like the
    /// viewer's.
    ShouldNotExist { to: Vec<String> },
    /// The server has no IPv6 address at all, and an AAAA record exists. Whatever it points
    /// at, it is not this machine.
    ServerHasNone { to: Vec<String> },
}

/// The answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    Ok,
    /// Nothing at all resolves. Said as "the domain is not attached to the server" rather
    /// than as a name-resolution error (FR-140): a person buying their first server gets
    /// nothing from `NXDOMAIN`.
    NotPointed,
    /// It resolves, and not to here.
    PointsElsewhere {
        record: RecordKind,
        to: Vec<String>,
    },
    /// The records do not agree with the choice that was made.
    Ipv6Mismatch {
        problem: Ipv6Problem,
    },
}

fn shown<T: ToString>(addresses: &[T]) -> Vec<String> {
    addresses.iter().map(ToString::to_string).collect()
}

/// Judge the records against the server and the choice.
///
/// The order of the questions is the order a person can act on: the ordinary record first,
/// because without it nothing works for anybody, and the IPv6 one after, because it decides
/// whether it works for some people and not others.
pub fn judge(records: &Records, server: &ServerAddresses, choice: Ipv6Choice) -> Verdict {
    if records.a.is_empty() && records.aaaa.is_empty() {
        return Verdict::NotPointed;
    }

    // The ordinary record. Absent, the domain is not attached in the sense a person means —
    // even if an AAAA exists, because most of the way to the server is over IPv4.
    match server.v4 {
        Some(ours) if records.a.contains(&ours) => {}
        _ if records.a.is_empty() => return Verdict::NotPointed,
        _ => {
            return Verdict::PointsElsewhere {
                record: RecordKind::A,
                to: shown(&records.a),
            }
        }
    }

    // The IPv6 half.
    match (server.v6, choice, records.aaaa.is_empty()) {
        // Nothing to keep and nothing claimed: fine.
        (_, Ipv6Choice::Disable, true) => Verdict::Ok,
        (None, Ipv6Choice::Keep, true) => Verdict::Ok,

        // An AAAA record on a machine with no IPv6 address. Whatever it leads to, it is not
        // here — and it is the likeliest shape of the quiet failure: a leftover record from
        // the domain's previous life.
        (None, _, false) => Verdict::Ipv6Mismatch {
            problem: Ipv6Problem::ServerHasNone {
                to: shown(&records.aaaa),
            },
        },

        // Turning IPv6 off while the domain still promises it.
        (Some(_), Ipv6Choice::Disable, false) => Verdict::Ipv6Mismatch {
            problem: Ipv6Problem::ShouldNotExist {
                to: shown(&records.aaaa),
            },
        },

        // Keeping IPv6 with nothing to reach it by.
        (Some(_), Ipv6Choice::Keep, true) => Verdict::Ipv6Mismatch {
            problem: Ipv6Problem::Missing,
        },

        (Some(ours), Ipv6Choice::Keep, false) => {
            if records.aaaa.contains(&ours) {
                Verdict::Ok
            } else {
                Verdict::Ipv6Mismatch {
                    problem: Ipv6Problem::PointsElsewhere {
                        to: shown(&records.aaaa),
                    },
                }
            }
        }
    }
}

impl Verdict {
    /// Whether a deployment may begin.
    pub fn may_begin(&self) -> bool {
        matches!(self, Self::Ok)
    }
}
