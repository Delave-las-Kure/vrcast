//! T222 — a viewer's address in writing.
//!
//! The decision, made by the owner on 2026-08-26: a stable pseudonym everywhere. What is
//! checked here is that it really is stable, that it really does not carry the address, and
//! that the scrubbing catches the shapes a server's own tools write addresses in.

use vrcast_studio_lib::domain::pseudonym::{pseudonym, scrub_addresses};

const KEY: &str = "a-key-made-once-on-this-machine";

#[test]
fn the_same_viewer_is_the_same_name_every_time() {
    // The whole point. Without this a person cannot see that the viewer who complained an
    // hour ago is the one they are looking at now, and the log becomes a list of strangers.
    let first = pseudonym("203.0.113.10", KEY);
    assert_eq!(first, pseudonym("203.0.113.10", KEY));
    // And written differently is still the same viewer: an address is not case-sensitive,
    // and a stray space is not a different person.
    assert_eq!(first, pseudonym("  203.0.113.10  ", KEY));
    assert_eq!(
        pseudonym("2A00:1450:4001:0::1", KEY),
        pseudonym("2a00:1450:4001:0::1", KEY)
    );
}

#[test]
fn two_viewers_are_two_names() {
    let a = pseudonym("203.0.113.10", KEY);
    let b = pseudonym("203.0.113.11", KEY);
    assert_ne!(a, b, "neighbouring addresses came out as one viewer");
    assert_ne!(a, pseudonym("2a00:1450::1", KEY));
}

#[test]
fn the_name_carries_no_part_of_the_address() {
    // A token holding even a piece of the address would defeat the point for the ordinary
    // case — a person reading a log over somebody's shoulder.
    for ip in ["203.0.113.10", "198.51.100.7", "2a00:1450:4001:0::1"] {
        let name = pseudonym(ip, KEY);
        for piece in ip.split(['.', ':']).filter(|p| p.len() > 1) {
            assert!(!name.contains(piece), "{name} carries {piece} out of {ip}");
        }
    }
}

#[test]
fn a_different_machine_gives_different_names_for_the_same_viewer() {
    // Two people's logs cannot be laid side by side to find a common viewer, which is what
    // makes this a pseudonym rather than a nickname.
    assert_ne!(
        pseudonym("203.0.113.10", KEY),
        pseudonym("203.0.113.10", "a-key-from-somebody-else's-machine")
    );
}

#[test]
fn the_scrubbing_catches_the_shapes_a_server_writes_addresses_in() {
    // This is the connection table's own shape: `ss` writes an address and a port together,
    // and half-scrubbing one would leave the address in the log with a colon after it.
    let table = "\
ESTAB 0 0 192.0.2.5:443 203.0.113.10:51234
ESTAB 0 0 192.0.2.5:443 [2a00:1450:4001:0::1]:51235
";
    let scrubbed = scrub_addresses(table, KEY);
    for address in ["203.0.113.10", "2a00:1450:4001:0::1", "192.0.2.5"] {
        assert!(
            !scrubbed.contains(address),
            "{address} survived the scrubbing:\n{scrubbed}"
        );
    }
    // The ports and the rest of the line stay: they are what makes the line readable.
    assert!(scrubbed.contains(":443"));
    assert!(scrubbed.contains("ESTAB"));
    assert_eq!(
        scrubbed.matches("viewer#").count(),
        4,
        "not every address became a name:\n{scrubbed}"
    );
}

#[test]
fn the_same_address_scrubs_to_the_same_name_within_one_piece_of_text() {
    let text = "203.0.113.10 asked, then 203.0.113.10 asked again, and 198.51.100.7 once";
    let scrubbed = scrub_addresses(text, KEY);
    let names: Vec<&str> = scrubbed
        .split_whitespace()
        .filter(|w| w.starts_with("viewer#"))
        .collect();
    assert_eq!(names.len(), 3);
    assert_eq!(names[0], names[1], "one viewer came out as two");
    assert_ne!(names[0], names[2]);
}

#[test]
fn text_with_no_addresses_comes_back_as_it_was() {
    // A scrubber that mangles ordinary text would be paid for on every line of every log.
    for text in [
        "the connection table would not be read",
        "ffmpeg version n8.1.2-44-g7c533d0f86",
        "caddy 2.11.4 reloaded",
        "a file at F:/films/film.mp4, 4.2 GB",
        "",
    ] {
        assert_eq!(
            scrub_addresses(text, KEY),
            text,
            "the scrubbing changed text that holds no address"
        );
    }
}

#[test]
fn an_address_at_the_very_edges_of_the_text_is_still_found() {
    assert!(!scrub_addresses("203.0.113.10", KEY).contains("203.0.113"));
    assert!(!scrub_addresses("from 203.0.113.10.", KEY).contains("203.0.113"));
    assert!(scrub_addresses("from 203.0.113.10.", KEY).ends_with('.'));
}
