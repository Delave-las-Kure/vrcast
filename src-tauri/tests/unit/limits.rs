//! T207 — the pure logic of limiting a viewer's quality.

use vrcast_studio_lib::domain::hls_master::Variant;
use vrcast_studio_lib::domain::limits_conf::{build, matcher_name, parse, Limit};
use vrcast_studio_lib::domain::slow_master::{
    legacy_slow_master_path, plan, shorten, slow_master_address, slow_master_path, SlowPlan,
};

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
    // except through the rule that rewrites onto it. One per ceiling (T602): two viewers
    // of one medium with different ceilings must not share a file.
    assert_eq!(
        slow_master_path("/var/lib/vrcast/videos", "demo", 6_000_000),
        "/var/lib/vrcast/videos/_slow/demo/6000000/master.m3u8"
    );
    assert_eq!(
        slow_master_path("/var/lib/vrcast/videos/", "demo", 6_000_000),
        "/var/lib/vrcast/videos/_slow/demo/6000000/master.m3u8"
    );
    assert_ne!(
        slow_master_path("/var/lib/vrcast/videos", "demo", 6_000_000),
        slow_master_path("/var/lib/vrcast/videos", "demo", 13_000_000)
    );
    // The address a rule rewrites onto is the same path under the serving prefix.
    assert_eq!(
        slow_master_address("/videos", "demo", 6_000_000),
        "/videos/_slow/demo/6000000/master.m3u8"
    );
    assert_eq!(
        slow_master_address("/videos/", "demo", 6_000_000),
        "/videos/_slow/demo/6000000/master.m3u8"
    );
}

#[test]
fn the_description_from_before_t602_is_still_named_so_it_can_be_removed() {
    // Written by earlier clients, one per medium; only ever removed now.
    assert_eq!(
        legacy_slow_master_path("/var/lib/vrcast/videos", "demo"),
        "/var/lib/vrcast/videos/_slow/demo/master.m3u8"
    );
    assert_eq!(
        legacy_slow_master_path("/var/lib/vrcast/videos/", "demo"),
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
    let text = build(&[], "/videos", 0);
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
        0,
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
    let read = parse(&build(&limits, "/videos", 0));
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
    let one = build(&[a_limit("203.0.113.10", "demo", 12_000_000)], "/videos", 0);
    let none = build(&[], "/videos", 0);
    assert!(one.len() > none.len());
    assert_eq!(parse(&none).len(), 0);
    // And what is written always says whose file it is: a person who opens it should not
    // have to guess why their own lines vanished.
    assert!(none.contains("VRCast Studio"));
}

// ---------- T600: the generation counter ----------

#[test]
fn a_file_with_no_generation_line_at_all_reads_as_generation_zero() {
    // Backward compatibility: a server this application already deployed to before T600
    // has a `vrcast-limits.conf` with no such line. Reading that as a format error rather
    // than as "generation zero" would turn the very first `limit_set`/`limit_clear` call
    // after an application upgrade into a false conflict on a server nobody else ever
    // touched concurrently — see `Manifest::empty()`, which follows the identical rule for
    // an absent catalogue.
    use vrcast_studio_lib::domain::limits_conf::read_generation;

    let text = "# something a person wrote\n\
                # vrcast-limit 203.0.113.10 demo 12000000 2026-08-26T10:00:00Z\n";
    assert_eq!(read_generation(text), 0);
    assert_eq!(read_generation(""), 0);
}

#[test]
fn the_generation_written_is_read_back_unchanged() {
    use vrcast_studio_lib::domain::limits_conf::read_generation;

    let text = build(&[], "/videos", 42);
    assert_eq!(read_generation(&text), 42);
    // And the rules themselves are unaffected by the generation line sitting beside them.
    let with_rules = build(&[a_limit("203.0.113.10", "demo", 12_000_000)], "/videos", 7);
    assert_eq!(read_generation(&with_rules), 7);
    assert_eq!(parse(&with_rules).len(), 1);
}

#[test]
fn the_generation_marker_is_never_mistaken_for_a_rule() {
    // The generation line must not be picked up by `parse()`, which looks for a different,
    // deliberately non-overlapping marker (see `GENERATION_MARK`'s own doc comment for why
    // the two markers share no prefix in either direction).
    let text = build(&[a_limit("203.0.113.10", "demo", 12_000_000)], "/videos", 5);
    assert_eq!(
        parse(&text).len(),
        1,
        "the generation line was misread as an extra rule:\n{text}"
    );
}

// ---------- T602: one shortened description per ceiling ----------

/// The `rewrite` target of the rule for one address, read out of a built file.
fn rewrite_for(text: &str, ip: &str, slug: &str) -> String {
    let key = matcher_name(ip, slug);
    text.lines()
        .find_map(|l| l.strip_prefix(&format!("rewrite @{key} ")))
        .unwrap_or_else(|| panic!("no rewrite for {ip}/{slug} in:\n{text}"))
        .trim()
        .to_owned()
}

#[test]
fn two_viewers_of_one_medium_with_different_ceilings_are_rewritten_to_different_files() {
    // The bug: every rule of a medium rewrote onto one `_slow/<slug>/master.m3u8`, and the
    // last limit written decided what every limited viewer of that medium got.
    let text = build(
        &[
            a_limit("203.0.113.10", "demo", 2_000_000),
            a_limit("203.0.113.11", "demo", 5_000_000),
        ],
        "/videos",
        3,
    );
    let low = rewrite_for(&text, "203.0.113.10", "demo");
    let high = rewrite_for(&text, "203.0.113.11", "demo");
    assert_eq!(low, "/videos/_slow/demo/2000000/master.m3u8");
    assert_eq!(high, "/videos/_slow/demo/5000000/master.m3u8");
    assert_ne!(low, high);
    // Built from the one function the file path is built from, so they cannot drift.
    assert_eq!(low, slow_master_address("/videos", "demo", 2_000_000));
}

#[test]
fn two_viewers_with_the_same_ceiling_share_one_file() {
    let text = build(
        &[
            a_limit("203.0.113.10", "demo", 2_000_000),
            a_limit("203.0.113.11", "demo", 2_000_000),
        ],
        "/videos/",
        3,
    );
    assert_eq!(
        rewrite_for(&text, "203.0.113.10", "demo"),
        rewrite_for(&text, "203.0.113.11", "demo")
    );
    assert_eq!(
        rewrite_for(&text, "203.0.113.10", "demo"),
        "/videos/_slow/demo/2000000/master.m3u8"
    );
}

#[test]
fn the_ceiling_survives_a_round_trip_through_the_file() {
    let limits = vec![
        a_limit("203.0.113.10", "demo", 2_000_000),
        a_limit("203.0.113.11", "demo", 5_000_000),
        a_limit("203.0.113.12", "demo", 5_000_000),
    ];
    let text = build(&limits, "/videos", 9);
    assert_eq!(parse(&text), limits);
    // And what is read back rewrites onto the same files again.
    assert_eq!(build(&parse(&text), "/videos", 9), text);
}

const VIDEOS: &str = "/var/lib/vrcast/videos";

fn file(slug: &str, cap: u64) -> String {
    slow_master_path(VIDEOS, slug, cap)
}
fn legacy(slug: &str) -> String {
    legacy_slow_master_path(VIDEOS, slug)
}
fn cap_dir(slug: &str, cap: u64) -> String {
    format!("{VIDEOS}/_slow/{slug}/{cap}")
}
fn slug_dir(slug: &str) -> String {
    format!("{VIDEOS}/_slow/{slug}")
}

#[test]
fn taking_off_one_of_two_different_ceilings_removes_only_its_file() {
    let before = [
        a_limit("203.0.113.10", "demo", 2_000_000),
        a_limit("203.0.113.11", "demo", 5_000_000),
    ];
    let after = [a_limit("203.0.113.11", "demo", 5_000_000)];
    let p = plan(VIDEOS, &before, &after);
    assert_eq!(
        p,
        SlowPlan {
            descriptions: vec![(String::from("demo"), 5_000_000)],
            remove_files: vec![file("demo", 2_000_000), legacy("demo")],
            remove_dirs_if_empty: vec![cap_dir("demo", 2_000_000), slug_dir("demo")],
        }
    );
    assert!(!p.remove_files.contains(&file("demo", 5_000_000)));
}

#[test]
fn taking_off_one_of_two_equal_ceilings_removes_nothing_the_other_uses() {
    let before = [
        a_limit("203.0.113.10", "demo", 2_000_000),
        a_limit("203.0.113.11", "demo", 2_000_000),
    ];
    let after = [a_limit("203.0.113.11", "demo", 2_000_000)];
    let p = plan(VIDEOS, &before, &after);
    assert_eq!(p.descriptions, vec![(String::from("demo"), 2_000_000)]);
    assert_eq!(p.remove_files, vec![legacy("demo")]);
    // The medium's directory is only a candidate; it holds the other's file and stays.
    assert_eq!(p.remove_dirs_if_empty, vec![slug_dir("demo")]);

    // And taking off the last one removes the ceiling's file and both directories.
    let p = plan(VIDEOS, &after, &[]);
    assert!(p.descriptions.is_empty());
    assert_eq!(
        p.remove_files,
        vec![file("demo", 2_000_000), legacy("demo")]
    );
    assert_eq!(
        p.remove_dirs_if_empty,
        vec![cap_dir("demo", 2_000_000), slug_dir("demo")]
    );
}

#[test]
fn changing_one_viewers_ceiling_writes_the_new_file_and_removes_the_old() {
    let before = [a_limit("203.0.113.10", "demo", 2_000_000)];
    let after = [a_limit("203.0.113.10", "demo", 5_000_000)];
    let p = plan(VIDEOS, &before, &after);
    assert_eq!(p.descriptions, vec![(String::from("demo"), 5_000_000)]);
    assert_eq!(
        p.remove_files,
        vec![file("demo", 2_000_000), legacy("demo")]
    );
    assert!(!p.remove_files.contains(&file("demo", 5_000_000)));
}

#[test]
fn every_medium_named_before_or_after_loses_its_description_from_before_t602() {
    // The file is written whole and every rule in it rewrites onto the new paths, so after
    // any change nothing reaches any `_slow/<slug>/master.m3u8` — of the medium being
    // changed or of any other.
    let before = [
        a_limit("203.0.113.10", "demo", 2_000_000),
        a_limit("203.0.113.30", "other", 1_000_000),
        a_limit("203.0.113.40", "gone", 1_000_000),
    ];
    let after = [
        a_limit("203.0.113.10", "demo", 2_000_000),
        a_limit("203.0.113.30", "other", 1_000_000),
        a_limit("203.0.113.20", "fresh", 3_000_000),
    ];
    let p = plan(VIDEOS, &before, &after);
    assert_eq!(
        p.descriptions,
        vec![
            (String::from("demo"), 2_000_000),
            (String::from("fresh"), 3_000_000),
            (String::from("other"), 1_000_000),
        ]
    );
    for slug in ["demo", "other", "gone", "fresh"] {
        assert!(
            p.remove_files.contains(&legacy(slug)),
            "the pre-T602 file of {slug} is not removed: {:?}",
            p.remove_files
        );
        assert!(p.remove_dirs_if_empty.contains(&slug_dir(slug)));
    }
    assert!(p.remove_files.contains(&file("gone", 1_000_000)));
    // Nothing still named is removed, and `_slow/` itself never is.
    for (slug, cap) in &p.descriptions {
        assert!(!p.remove_files.contains(&file(slug, *cap)));
        assert!(!p.remove_dirs_if_empty.contains(&cap_dir(slug, *cap)));
    }
    assert!(!p
        .remove_dirs_if_empty
        .iter()
        .any(|d| d == &format!("{VIDEOS}/_slow")));
}

#[test]
fn a_rule_naming_a_medium_that_would_leave_the_directory_is_left_alone() {
    // The rules file is ours but the server is not: a hand-edited rule must not make the
    // plan remove anything outside `_slow/<slug>/`.
    let before = [
        a_limit("203.0.113.10", "..", 2_000_000),
        a_limit("203.0.113.11", "a/b", 2_000_000),
    ];
    let p = plan(VIDEOS, &before, &[]);
    assert_eq!(p, SlowPlan::default());
}

// ---------- T618: a lock renewed for as long as the change is alive ----------

use std::time::Duration;

use tokio::io::AsyncReadExt;
use vrcast_studio_lib::server::limits::{
    holder_command, renew_until, Renewal, LOCK_SILENCE, RENEW_EVERY,
};

#[test]
fn the_renewal_period_and_the_silence_it_guards_against_are_what_the_contract_says() {
    // Three renewals may be lost to a slow link before the server lets go, and a change
    // queued behind a silent one still gets the lock within its own two-minute wait.
    assert_eq!(RENEW_EVERY, Duration::from_secs(15));
    assert_eq!(LOCK_SILENCE, Duration::from_secs(60));
    assert!(RENEW_EVERY * 4 <= LOCK_SILENCE);
    assert!(
        LOCK_SILENCE < Duration::from_secs(120),
        "must stay under LOCK_WAIT"
    );
}

#[test]
fn the_holder_waits_for_signs_of_life_rather_than_for_a_fixed_time() {
    let cmd = holder_command("abc123", "/etc/caddy/vrcast-limits.conf.lock");
    assert!(
        !cmd.contains("timeout "),
        "a fixed lease is back — a live change would lose its lock: {cmd}"
    );
    assert!(
        cmd.contains("read -r -t 60"),
        "the holder does not wait on silence: {cmd}"
    );
    assert!(
        cmd.contains("bash -c"),
        "`read -t` needs bash, not sh: {cmd}"
    );
    assert!(cmd.contains("VRCAST_LIMITS_TXN=abc123"));
    assert!(cmd.contains("flock -x -w 120 -E 75 '/etc/caddy/vrcast-limits.conf.lock'"));
    assert!(cmd.contains("echo \"LOCKED $PPID\""), "{cmd}");
}

#[tokio::test]
async fn renewals_keep_coming_until_the_lock_is_given_back() {
    let (ours, mut holder) = tokio::io::duplex(1024);
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let renewing = tokio::spawn(renew_until(ours, Duration::from_millis(50), stopped));

    // The holder's side: a line at a time, and never more than a few periods apart.
    let started = std::time::Instant::now();
    let mut byte = [0u8; 1];
    for _ in 0..6 {
        let n = tokio::time::timeout(Duration::from_millis(500), holder.read(&mut byte))
            .await
            .expect("the holder heard nothing for ten periods")
            .expect("the holder's input failed");
        assert_eq!(n, 1, "the input ended while the change was still alive");
        assert_eq!(byte[0], b'\n');
    }
    assert!(
        started.elapsed() >= Duration::from_millis(250),
        "renewals came faster than asked: {:?}",
        started.elapsed()
    );

    // Giving back: end of input on the holder's side, and a wait for the holder to end.
    stop.send(()).expect("the renewing ended by itself");
    let mut rest = Vec::new();
    holder
        .read_to_end(&mut rest)
        .await
        .expect("the holder's input did not end");
    assert!(rest.iter().all(|b| *b == b'\n'));
    drop(holder); // the holder ended: the server closes the channel
    let renewal = renewing.await.expect("the renewing panicked");
    assert!(renewal.sent >= 6);
    assert!(!renewal.failed);
    assert!(
        renewal.given_back,
        "the channel closed and giving back was not confirmed: {renewal:?}"
    );
}

#[tokio::test]
async fn giving_back_is_not_confirmed_until_the_holder_has_ended() {
    // The holder took the end of input but is still there: no confirmation yet, and none
    // assumed.
    let (ours, mut holder) = tokio::io::duplex(1024);
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let renewing = tokio::spawn(renew_until(ours, Duration::from_millis(20), stopped));
    tokio::time::sleep(Duration::from_millis(70)).await;
    stop.send(()).expect("the renewing ended by itself");
    let mut rest = Vec::new();
    holder
        .read_to_end(&mut rest)
        .await
        .expect("no end of input");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !renewing.is_finished(),
        "giving back was taken as confirmed while the holder was still there"
    );
    drop(holder);
    assert!(renewing.await.expect("panicked").given_back);
}

#[tokio::test]
async fn a_closed_channel_ends_renewing_and_says_so() {
    let (ours, holder) = tokio::io::duplex(1024);
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let renewing = tokio::spawn(renew_until(ours, Duration::from_millis(20), stopped));
    tokio::time::sleep(Duration::from_millis(70)).await;
    drop(holder); // the connection went: nothing more can reach the holder
    tokio::time::sleep(Duration::from_millis(100)).await;
    stop.send(()).expect("the renewing ended by itself");
    let renewal: Renewal = renewing.await.expect("panicked");
    assert!(
        renewal.failed,
        "a write into a closed channel passed: {renewal:?}"
    );
    assert!(!renewal.given_back);
}

#[tokio::test]
async fn an_abandoned_change_lets_the_lock_go_at_once() {
    // `TxnLock` dropped without `release`: the stop signal goes, and the channel with it.
    let (ours, mut holder) = tokio::io::duplex(1024);
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let renewing = tokio::spawn(renew_until(ours, Duration::from_secs(60), stopped));
    drop(stop);
    let renewal = tokio::time::timeout(Duration::from_secs(2), renewing)
        .await
        .expect("an abandoned change kept its channel open")
        .expect("panicked");
    assert!(!renewal.given_back);
    let mut rest = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), holder.read_to_end(&mut rest))
        .await
        .expect("the holder's input never ended")
        .expect("the holder's input failed");
}
