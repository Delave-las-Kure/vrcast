//! T104 — when to report that a task finished (FR-084).
//!
//! Only the decision "report or keep quiet" is checked: showing a notification is the
//! system's business and cannot be portrayed here. The decision matters more than the
//! showing: a notification about every trifle teaches people to dismiss them unread, and
//! then the important one goes by along with the rest.

use vrcast_studio_lib::commands::events::long_enough;
use vrcast_studio_lib::store::db::parse_rfc3339;

#[test]
fn short_tasks_are_not_reported() {
    // Examining a source fits in a second and keeps nobody waiting.
    assert!(!long_enough("2026-08-25T10:00:00Z", "2026-08-25T10:00:01Z"));
}

#[test]
fn long_ones_are_reported() {
    // A thirty-gigabyte upload runs for hours: a person has time to go and do something
    // else and forget about the task.
    assert!(long_enough("2026-08-25T10:00:00Z", "2026-08-25T12:34:00Z"));
}

#[test]
fn exactly_half_a_minute_counts_as_long() {
    assert!(long_enough("2026-08-25T10:00:00Z", "2026-08-25T10:00:30Z"));
    assert!(!long_enough("2026-08-25T10:00:00Z", "2026-08-25T10:00:29Z"));
}

#[test]
fn an_unparsed_timestamp_counts_as_short() {
    // Keeping quiet once too often beats signalling once too often: needless
    // notifications teach people not to read them at all.
    assert!(!long_enough("nonsense", "2026-08-25T12:00:00Z"));
    assert!(!long_enough("2026-08-25T10:00:00Z", ""));
}

#[test]
fn a_timestamp_parses_back() {
    let now = vrcast_studio_lib::store::db::now_rfc3339();
    let back = parse_rfc3339(&now).expect("our own timestamp would not parse");
    // Certainly later than the epoch: parsing that quietly gives zero would show
    // half-century spans where there are none.
    assert!(back > 1_700_000_000);
}

#[test]
fn the_time_zone_does_not_shift_the_span() {
    // The timestamps come out of the database in UTC, but parsing must treat one and the
    // same moment as the same however it was written down.
    let utc = parse_rfc3339("2026-08-25T10:00:00Z").unwrap();
    let msk = parse_rfc3339("2026-08-25T13:00:00+03:00").unwrap();
    assert_eq!(utc, msk);
}
