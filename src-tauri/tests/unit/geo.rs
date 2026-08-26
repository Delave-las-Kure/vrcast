//! T164 — the rules for placing a viewer's address.
//!
//! What is checked here is what is ours to get wrong. Searching a table is a well-worn
//! library's business, and testing it would be testing somebody else's binary search; what
//! matters is **which addresses are never looked up at all**, and that an absent answer
//! stays absent instead of being filled in.

use std::net::IpAddr;

use vrcast_studio_lib::domain::geo::{is_not_public, Place};
use vrcast_studio_lib::store::geo::{month_name, needs_fetching, previous_month, Places};

fn addr(ip: &str) -> IpAddr {
    ip.parse().expect("a bad address in the test itself")
}

#[test]
fn an_address_from_the_next_room_is_never_looked_up() {
    // Tables do hold rows for these. Answering out of them would put somebody watching from
    // the next room in a country, and it would look exactly like knowledge.
    for ip in [
        "127.0.0.1",
        "10.10.0.3",
        "192.168.1.5",
        "172.16.0.1",
        "169.254.1.1",
        "100.64.0.1",
        "0.0.0.0",
        "255.255.255.255",
        "198.51.100.7",
        "203.0.113.9",
        "::1",
        "::",
        "fe80::1",
        "fd00::1",
        "2001:db8::5",
        "::ffff:192.168.1.5",
    ] {
        assert!(
            is_not_public(&addr(ip)),
            "{ip} would be looked up, and no table can speak for it"
        );
    }
}

#[test]
fn an_ordinary_address_is_looked_up() {
    for ip in ["8.8.8.8", "77.88.55.88", "81.2.69.7", "2a00:1450::5"] {
        assert!(
            !is_not_public(&addr(ip)),
            "{ip} would be refused, and it is an ordinary public address"
        );
    }
}

#[test]
fn knowing_only_the_country_is_not_the_same_as_knowing_nothing() {
    let only_country = Place {
        country: Some(String::from("FR")),
        ..Place::default()
    };
    assert!(
        !only_country.is_empty(),
        "a known country was treated as nothing known"
    );
    assert!(Place::default().is_empty());
}

#[test]
fn with_no_tables_everything_is_not_determined_and_nothing_falls_over() {
    // This is the state the application ships in — the tables arrive later, or never, if
    // there is no network. Every viewer is then "not determined", which is the truth, and
    // the rest of the screen goes on working.
    let places = Places::default();
    assert!(places.is_empty());

    for ip in ["8.8.8.8", "2a00:1450::5", "10.0.0.1", "not-an-address", ""] {
        assert_eq!(
            places.look_up(ip),
            Place::default(),
            "{ip} was answered for out of tables that are not there"
        );
    }
}

#[test]
fn tables_that_are_not_there_are_asked_for() {
    let dir = std::env::temp_dir().join("vrcast-geo-absent");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        needs_fetching(&dir, "2026-08"),
        "with nothing on disk the tables would never be fetched"
    );
}

#[test]
fn the_month_before_january_is_last_december() {
    // A month's file appears a little way into it, so the previous one is asked for next.
    // Getting the turn of the year wrong here would leave a person without a table every
    // January — and only every January, which is the hardest kind of fault to notice.
    assert_eq!(previous_month(2027, 1), (2026, 12));
    assert_eq!(previous_month(2026, 9), (2026, 8));
    assert_eq!(month_name(2026, 8), "2026-08");
    assert_eq!(month_name(2027, 12), "2027-12");
}
