//! T164 — looking an address up in the table held on this machine.
//!
//! The table here is four rows rather than the real one. That is the point of keeping the
//! lookup apart from where the data comes from: the rule can be checked exactly, on
//! addresses chosen to sit on the edges, instead of on whatever a hundred megabytes happens
//! to contain this month.

use std::net::IpAddr;

use vrcast_studio_lib::domain::geo::{as_number, GeoTable, Place, Span};

fn number(ip: &str) -> u128 {
    as_number(
        &ip.parse::<IpAddr>()
            .expect("a bad address in the test itself"),
    )
}

fn place(country: &str, city: &str, provider: &str) -> Place {
    Place {
        country: Some(country.to_owned()),
        city: Some(city.to_owned()),
        asn_org: Some(provider.to_owned()),
    }
}

/// Deliberately handed over out of order: the search by halving is silent when its input is
/// unsorted — it does not fail, it answers wrongly for some addresses and rightly for
/// others, which is the hardest kind of fault to notice.
///
/// The addresses are ordinary public ones, and not the ranges set aside for writing
/// examples in (`198.51.100.0/24`, `203.0.113.0/24`, `2001:db8::/32`). Those were what this
/// test reached for first, and every lookup came back "not determined" — rightly, since the
/// code refuses them along with the private ones. A table cannot be checked on addresses no
/// table is allowed to answer for.
fn table() -> GeoTable {
    GeoTable::new(vec![
        Span {
            first: number("77.88.55.0"),
            last: number("77.88.55.255"),
            place: place("NL", "Amsterdam", "Example Networks"),
        },
        Span {
            first: number("8.8.8.0"),
            last: number("8.8.8.255"),
            place: place("US", "Mountain View", "Example Cloud"),
        },
        Span {
            first: number("2a00:1450::"),
            last: number("2a00:1450::ffff"),
            place: place("DE", "Frankfurt", "Example GmbH"),
        },
        // A row that knows the country and nothing else. Common in a free table, and it
        // must not be mistaken for knowing nothing.
        Span {
            first: number("81.2.69.0"),
            last: number("81.2.69.255"),
            place: Place {
                country: Some(String::from("FR")),
                ..Place::default()
            },
        },
    ])
}

#[test]
fn an_address_inside_a_span_is_found() {
    let table = table();
    let found = table
        .look_up("8.8.8.44")
        .expect("the address was not found");
    assert_eq!(found.country.as_deref(), Some("US"));
    assert_eq!(found.city.as_deref(), Some("Mountain View"));
    assert_eq!(found.asn_org.as_deref(), Some("Example Cloud"));
}

#[test]
fn both_ends_of_a_span_belong_to_it() {
    // The ends are where an off-by-one hides, and it hides well: everything in the middle
    // keeps working.
    let table = table();
    assert_eq!(
        table.look_up("8.8.8.0").and_then(|p| p.country.as_deref()),
        Some("US")
    );
    assert_eq!(
        table
            .look_up("8.8.8.255")
            .and_then(|p| p.country.as_deref()),
        Some("US")
    );
    assert!(
        table.look_up("8.8.7.255").is_none(),
        "one below the span was claimed"
    );
    assert!(
        table.look_up("8.8.9.0").is_none(),
        "one above the span was claimed"
    );
}

#[test]
fn an_address_between_two_spans_belongs_to_neither() {
    // The nearest row is not the answer. Filling a gap in from a neighbour would put a
    // viewer in a city they have never been to, and it would look exactly like knowledge.
    assert!(table().look_up("9.9.9.9").is_none());
}

#[test]
fn ipv6_is_looked_up_on_the_same_ruler_as_ipv4() {
    let table = table();
    assert_eq!(
        table
            .look_up("2a00:1450::5")
            .and_then(|p| p.country.as_deref()),
        Some("DE")
    );
    // And an IPv4 address dressed as IPv6 is still that IPv4 address.
    assert_eq!(
        table
            .look_up("::ffff:8.8.8.44")
            .and_then(|p| p.country.as_deref()),
        Some("US")
    );
}

#[test]
fn an_address_from_the_next_room_is_not_determined_rather_than_invented() {
    // No table can speak for these. A table that answers for one is answering about its own
    // reserved rows, and the person watching from the next room would be shown in a
    // country.
    for ip in [
        "127.0.0.1",
        "10.10.0.3",
        "192.168.1.5",
        "172.16.0.1",
        "169.254.1.1",
        "100.64.0.1",
        "::1",
        "fe80::1",
        "fd00::1",
        "::ffff:192.168.1.5",
    ] {
        assert!(
            table().look_up(ip).is_none(),
            "{ip} was answered for, and it is an address nobody can answer for"
        );
    }
}

#[test]
fn knowing_only_the_country_still_counts_as_knowing_something() {
    let table = table();
    let found = table
        .look_up("81.2.69.7")
        .expect("a row that knows only the country was treated as knowing nothing");
    assert_eq!(found.country.as_deref(), Some("FR"));
    assert_eq!(found.city, None, "a city was invented");
    assert_eq!(found.asn_org, None, "a provider was invented");
}

#[test]
fn nonsense_is_not_determined_rather_than_a_failure() {
    // Whatever is in the log is what was asked for, and a viewer is not to be lost over a
    // line somebody sent by hand.
    assert!(table().look_up("not-an-address").is_none());
    assert!(table().look_up("").is_none());
}

#[test]
fn an_empty_table_answers_nothing_rather_than_falling_over() {
    // Until the table is put into the package (T162) this is the state the application
    // ships in, and "not determined" everywhere is the right behaviour for it.
    let empty = GeoTable::default();
    assert!(empty.is_empty());
    assert!(empty.look_up("8.8.8.8").is_none());
}
