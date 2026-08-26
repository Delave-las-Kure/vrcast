//! T207 — the pure logic of limiting a viewer's quality.

use vrcast_studio_lib::domain::hls_master::Variant;
use vrcast_studio_lib::domain::limits_conf::{build, matcher_name, parse, Limit};
use vrcast_studio_lib::domain::slow_master::{shorten, slow_master_path};

fn variant(path: &str, bandwidth: u64, height: u32) -> Variant {
    Variant {
        path: path.to_owned(),
        bandwidth,
        average_bandwidth: bandwidth * 8 / 10,
        width: height * 16 / 9,
        height,
        fps: Some(24.0),
        codecs: String::from("avc1.640033,mp4a.40.2"),
    }
}

fn ladder() -> Vec<Variant> {
    vec![
        variant("v22/stream.m3u8", 24_000_000, 2160),
        variant("v12/stream.m3u8", 13_000_000, 1440),
        variant("v6/stream.m3u8", 6_600_000, 1080),
    ]
}

// ---------- the shortened description ----------

#[test]
fn only_the_rungs_a_viewer_can_hold_are_shown_to_them() {
    // A player takes the best variant it is shown and nothing talks it out of that. The
    // only way down to a rung a line can hold is to stop showing the ones it cannot.
    let short = shorten(&ladder(), 13_000_000, "/videos", "demo");
    assert_eq!(short.kept.len(), 2);
    assert!(!short.below_lightest);
    assert!(short.text.contains("v12/stream.m3u8"));
    assert!(short.text.contains("v6/stream.m3u8"));
    assert!(
        !short.text.contains("v22/"),
        "a rung above the cap was still offered:\n{}",
        short.text
    );
}

#[test]
fn the_paths_are_absolute_and_that_is_the_whole_of_the_recorded_mistake() {
    // A shortened description lives in a directory of its own. A relative `v6/stream.m3u8`
    // sends the player looking for the segments **inside that directory**, where there are
    // none — and the viewer gets nothing at all while everyone else is served happily.
    let short = shorten(&ladder(), 13_000_000, "/videos", "demo");
    for kept in &short.kept {
        assert!(
            kept.path.starts_with("/videos/demo/"),
            "a relative path survived: {}",
            kept.path
        );
    }
    assert!(short.text.contains("/videos/demo/v6/stream.m3u8"));

    // A prefix given without its leading slash is still written as an address.
    let bare = shorten(&ladder(), 13_000_000, "videos", "demo");
    assert!(bare.kept[0].path.starts_with("/videos/demo/"));
}

#[test]
fn a_cap_below_the_lightest_rung_still_gets_the_lightest_rather_than_nothing() {
    // FR-067. An empty description leaves a viewer with no video at all, which is worse
    // than video they cannot quite hold — and the person setting the limit is told, so they
    // can go and build a lighter rung if they want one.
    let short = shorten(&ladder(), 1_000_000, "/videos", "demo");
    assert!(short.below_lightest);
    assert_eq!(short.kept.len(), 1);
    assert_eq!(short.kept[0].bandwidth, 6_600_000);
    assert!(short.text.contains("/videos/demo/v6/stream.m3u8"));
}

#[test]
fn a_cap_above_everything_keeps_the_whole_ladder() {
    let short = shorten(&ladder(), 100_000_000, "/videos", "demo");
    assert_eq!(short.kept.len(), 3);
    assert!(!short.below_lightest);
}

#[test]
fn the_shortened_description_sits_beside_the_media_rather_than_inside_it() {
    // Inside, a viewer with no limit could stumble into it. Beside, nobody reaches it
    // except through the rule that rewrites onto it.
    assert_eq!(
        slow_master_path("/var/lib/vrcast/videos", "demo"),
        "/var/lib/vrcast/videos/_slow/demo/master.m3u8"
    );
    assert_eq!(
        slow_master_path("/var/lib/vrcast/videos/", "demo"),
        "/var/lib/vrcast/videos/_slow/demo/master.m3u8"
    );
}

// ---------- the file of rules ----------

fn a_limit(ip: &str, slug: &str, cap: u64) -> Limit {
    Limit {
        ip: ip.to_owned(),
        slug: slug.to_owned(),
        cap_bps: cap,
        set_at: String::from("2026-08-26T10:00:00Z"),
    }
}

#[test]
fn the_caching_rule_is_the_one_that_was_measured_to_work() {
    // Measured against Caddy itself: a plain set loses to the blanket rule set deeper in
    // the chain, a delete leaves the description with no caching rule at all, and only a
    // **deferred** set does what is wanted.
    let text = build(&[], "/videos");
    assert!(text.contains("defer"), "the rule is not deferred:\n{text}");
    assert!(text.contains("Cache-Control \"no-cache\""));
    assert!(
        !text.contains("-Cache-Control"),
        "a delete crept in, and a delete leaves no caching rule at all:\n{text}"
    );

    // Present even with nothing limited: a description should never have been cached for
    // thirty days in the first place, and the day a limit appears is too late to start.
    assert!(text.contains("master.m3u8"));
}

#[test]
fn each_limit_gets_a_rule_with_a_name_of_its_own() {
    // Caddy's matchers share one namespace. Two rules under one name quietly become one,
    // and the viewer who lost their rule is the one nobody hears from.
    let text = build(
        &[
            a_limit("203.0.113.10", "demo", 12_000_000),
            a_limit("203.0.113.11", "demo", 6_000_000),
        ],
        "/videos",
    );
    let first = matcher_name("203.0.113.10", "demo");
    let second = matcher_name("203.0.113.11", "demo");
    assert_ne!(first, second);
    assert!(text.contains(&format!("@{first}")));
    assert!(text.contains(&format!("@{second}")));
    assert_eq!(text.matches("rewrite @").count(), 2);

    // A matcher's name may hold neither dots nor colons, and an address is mostly those.
    for name in [first, second, matcher_name("2001:db8::1", "demo")] {
        assert!(
            name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
            "a name Caddy will not take: {name}"
        );
    }
}

#[test]
fn the_rules_can_be_read_back_from_the_server() {
    // FR-064. A local note goes stale the moment somebody edits the server by hand, and a
    // list of limits that does not match the server is worse than no list at all.
    let limits = vec![
        a_limit("203.0.113.10", "demo", 12_000_000),
        a_limit("198.51.100.7", "other-film", 6_000_000),
    ];
    let read = parse(&build(&limits, "/videos"));
    assert_eq!(read, limits);
}

#[test]
fn a_file_somebody_edited_by_hand_is_read_for_what_it_holds_rather_than_refused() {
    // The rules are ours, but the server is theirs. Something unrecognised in the file is
    // not a reason to report no limits: the ones that are there are still in force.
    let text = "# something a person wrote\n\
                # vrcast-limit 203.0.113.10 demo 12000000 2026-08-26T10:00:00Z\n\
                # vrcast-limit not-enough-fields\n\
                header X-Something \"else\"\n";
    let read = parse(text);
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].ip, "203.0.113.10");
    assert_eq!(read[0].cap_bps, 12_000_000);
}

#[test]
fn the_file_is_written_whole_rather_than_added_to() {
    // A file assembled from what is wanted now cannot drift, and drift in a serving
    // configuration is not the kind of thing anybody notices early.
    let one = build(&[a_limit("203.0.113.10", "demo", 12_000_000)], "/videos");
    let none = build(&[], "/videos");
    assert!(one.len() > none.len());
    assert_eq!(parse(&none).len(), 0);
    // And what is written always says whose file it is: a person who opens it should not
    // have to guess why their own lines vanished.
    assert!(none.contains("VRCast Studio"));
}
