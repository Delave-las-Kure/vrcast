//! T262 — the domain against the server and against the IPv6 choice.
//!
//! Four cases, and the fourth is the quiet one: the domain resolves, the ordinary record is
//! right, the deployment runs to the end and works — for everybody except viewers whose
//! connection prefers IPv6. They do not report a broken server. They report that it is slow,
//! or that it does not work for them, and it reads as their problem.

use std::net::{Ipv4Addr, Ipv6Addr};

use vrcast_studio_lib::domain::dns_verdict::{
    judge, Ipv6Choice, Ipv6Problem, RecordKind, Records, ServerAddresses, Verdict,
};
use vrcast_studio_lib::domain::wording::DetailCode;

fn v4(s: &str) -> Ipv4Addr {
    s.parse().expect("a bad IPv4 address in the test")
}

fn v6(s: &str) -> Ipv6Addr {
    s.parse().expect("a bad IPv6 address in the test")
}

fn server() -> ServerAddresses {
    ServerAddresses {
        v4: Some(v4("203.0.113.10")),
        v6: Some(v6("2001:db8::10")),
    }
}

#[test]
fn a_domain_nobody_created_is_not_attached() {
    // Said as "the domain is not attached to the server" rather than as a resolution error
    // (FR-140). A person buying their first server gets nothing at all from NXDOMAIN, and
    // what they need is the name of the record to create.
    let verdict = judge(&Records::default(), &server(), Ipv6Choice::Keep);
    assert_eq!(verdict, Verdict::NotPointed);
    assert!(!verdict.may_begin());
}

#[test]
fn a_domain_leading_somewhere_else_names_where() {
    // Naming it matters: the commonest cause is a record left from the domain's previous
    // life, and seeing the old address is what makes that obvious.
    let records = Records {
        a: vec![v4("198.51.100.7")],
        ..Records::default()
    };
    assert_eq!(
        judge(&records, &server(), Ipv6Choice::Disable),
        Verdict::PointsElsewhere {
            record: RecordKind::A,
            to: vec![String::from("198.51.100.7")]
        }
    );
}

#[test]
fn the_ordinary_record_pointing_here_with_ipv6_off_is_enough() {
    let records = Records {
        a: vec![v4("203.0.113.10")],
        ..Records::default()
    };
    assert_eq!(judge(&records, &server(), Ipv6Choice::Disable), Verdict::Ok);
}

#[test]
fn keeping_ipv6_needs_the_ipv6_record_to_lead_here_too() {
    let here = Records {
        a: vec![v4("203.0.113.10")],
        aaaa: vec![v6("2001:db8::10")],
    };
    assert_eq!(judge(&here, &server(), Ipv6Choice::Keep), Verdict::Ok);

    // **The quiet one.** Everything works except for people on IPv6.
    let elsewhere = Records {
        a: vec![v4("203.0.113.10")],
        aaaa: vec![v6("2001:db8::999")],
    };
    assert_eq!(
        judge(&elsewhere, &server(), Ipv6Choice::Keep),
        Verdict::Ipv6Mismatch {
            problem: Ipv6Problem::PointsElsewhere {
                to: vec![String::from("2001:db8::999")]
            }
        }
    );

    // And keeping it with nothing to reach it by.
    let missing = Records {
        a: vec![v4("203.0.113.10")],
        ..Records::default()
    };
    assert_eq!(
        judge(&missing, &server(), Ipv6Choice::Keep),
        Verdict::Ipv6Mismatch {
            problem: Ipv6Problem::Missing
        }
    );
}

#[test]
fn turning_ipv6_off_while_the_domain_still_promises_it_is_refused() {
    // FR-137 in as many words: with IPv6 turned off the record must lead nowhere. Left there,
    // the domain goes on handing out an address that has stopped answering, and every client
    // that prefers IPv6 tries it first.
    let records = Records {
        a: vec![v4("203.0.113.10")],
        aaaa: vec![v6("2001:db8::10")],
    };
    assert_eq!(
        judge(&records, &server(), Ipv6Choice::Disable),
        Verdict::Ipv6Mismatch {
            problem: Ipv6Problem::ShouldNotExist {
                to: vec![String::from("2001:db8::10")]
            }
        }
    );
}

#[test]
fn an_ipv6_record_on_a_server_without_ipv6_is_caught_whatever_was_chosen() {
    // The likeliest shape of a leftover record. Whatever it leads to, it is not this machine,
    // and no choice about IPv6 can make it right.
    let no_v6 = ServerAddresses {
        v4: Some(v4("203.0.113.10")),
        v6: None,
    };
    let records = Records {
        a: vec![v4("203.0.113.10")],
        aaaa: vec![v6("2001:db8::10")],
    };
    for choice in [Ipv6Choice::Keep, Ipv6Choice::Disable] {
        assert_eq!(
            judge(&records, &no_v6, choice),
            Verdict::Ipv6Mismatch {
                problem: Ipv6Problem::ServerHasNone {
                    to: vec![String::from("2001:db8::10")]
                }
            },
            "with {choice:?}"
        );
    }

    // And with no AAAA record such a server is perfectly fine.
    let plain = Records {
        a: vec![v4("203.0.113.10")],
        ..Records::default()
    };
    assert_eq!(judge(&plain, &no_v6, Ipv6Choice::Keep), Verdict::Ok);
}

#[test]
fn only_an_ipv6_record_is_still_a_domain_that_is_not_attached() {
    // Most of the way to a server is over IPv4. A domain with an AAAA record and no A one
    // resolves — so it is not "no such domain" — but for the person it is not attached, and
    // that is what they have to be told.
    let records = Records {
        aaaa: vec![v6("2001:db8::10")],
        ..Records::default()
    };
    assert_eq!(
        judge(&records, &server(), Ipv6Choice::Keep),
        Verdict::NotPointed
    );
}

// ---------- what the person is told to go and do (T266, FR-140) ----------

/// The value a detail carries, as the interface would read it.
fn said(detail: &vrcast_studio_lib::domain::wording::Detail, name: &str) -> String {
    detail
        .params
        .get(name)
        .map(|v| v.as_str().unwrap_or_default().to_owned())
        .unwrap_or_default()
}

#[test]
fn a_domain_that_is_not_attached_is_told_which_record_and_what_value() {
    // **Not "the domain does not resolve".** Somebody who has just bought their first
    // server gets nothing from that. The record's type, its exact name and the exact value
    // are what they can act on.
    let detail = judge(&Records::default(), &server(), Ipv6Choice::Disable)
        .what_to_do("stream.example.com", &server())
        .expect("nothing to do was named for a domain that is not attached");

    assert_eq!(detail.key, DetailCode::DomainAddRecord);
    assert_eq!(said(&detail, "record"), "A");
    assert_eq!(said(&detail, "name"), "stream.example.com");
    assert_eq!(said(&detail, "value"), "203.0.113.10");
}

#[test]
fn a_record_leading_elsewhere_is_told_where_it_leads_now() {
    // The commonest cause is a record left from the domain's previous life, and seeing the
    // old address is what makes that obvious. Without it the person looks at a record that
    // seems fine to them and cannot see why we disagree.
    let records = Records {
        a: vec![v4("198.51.100.7")],
        ..Records::default()
    };
    let detail = judge(&records, &server(), Ipv6Choice::Disable)
        .what_to_do("stream.example.com", &server())
        .expect("nothing to do was named");

    assert_eq!(detail.key, DetailCode::DomainFixRecord);
    assert_eq!(said(&detail, "to"), "198.51.100.7");
    assert_eq!(said(&detail, "value"), "203.0.113.10");
}

#[test]
fn a_domain_that_is_in_order_has_nothing_to_do_about_it() {
    let records = Records {
        a: vec![v4("203.0.113.10")],
        ..Records::default()
    };
    assert!(judge(&records, &server(), Ipv6Choice::Disable)
        .what_to_do("stream.example.com", &server())
        .is_none());
}

#[test]
fn a_server_reachable_only_over_ipv6_is_not_asked_for_an_ipv4_record() {
    // **Found while writing the refusal's wording** (T266). Demanding an A record of a
    // machine that has no IPv4 address sends its owner to create a record pointing at
    // nothing — and the deployment would then refuse the record it asked for.
    let only_v6 = ServerAddresses {
        v4: None,
        v6: Some(v6("2001:db8::10")),
    };

    // Right: the IPv6 record leads here and there is no IPv4 record to disagree with.
    let right = Records {
        aaaa: vec![v6("2001:db8::10")],
        ..Records::default()
    };
    assert_eq!(judge(&right, &only_v6, Ipv6Choice::Keep), Verdict::Ok);

    // Nothing at all: the record to ask for is the AAAA one, with the address the machine
    // actually has.
    let detail = judge(&Records::default(), &only_v6, Ipv6Choice::Keep)
        .what_to_do("stream.example.com", &only_v6)
        .expect("nothing to do was named");
    assert_eq!(detail.key, DetailCode::DomainAddRecord);
    assert_eq!(said(&detail, "record"), "AAAA");
    assert_eq!(said(&detail, "value"), "2001:db8::10");

    // And an IPv4 record on such a machine leads somewhere that is not it.
    let stray = Records {
        a: vec![v4("198.51.100.7")],
        aaaa: vec![v6("2001:db8::10")],
    };
    assert_eq!(
        judge(&stray, &only_v6, Ipv6Choice::Keep),
        Verdict::PointsElsewhere {
            record: RecordKind::A,
            to: vec![String::from("198.51.100.7")]
        }
    );
}
