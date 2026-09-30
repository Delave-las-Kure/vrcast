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
    assert!(cmd.contains("VRCAST_LIMITS_TXN="), "{cmd}");
    assert!(
        cmd.contains("env VRCAST_LIMITS_HELD='abc123' bash -c"),
        "{cmd}"
    );
    assert!(cmd.contains("flock -x -w 120 -E 75 '/etc/caddy/vrcast-limits.conf.lock'"));
    assert!(cmd.contains("LOCKED $$ $1"), "{cmd}");
}

// ---------- T628: the lock is let go only once the change's steps have ended ----------

use vrcast_studio_lib::server::limits::{
    read_locked, write_step, Holder, DRAIN_CEILING, WRITE_ENV,
};

#[test]
fn the_holder_waits_for_the_changes_steps_before_it_lets_go() {
    let cmd = holder_command("abc123", "/etc/caddy/vrcast-limits.conf.lock");
    // The very stop of T609, over the change's own mark: the steps going forward first,
    // then — the holder's mark dropped, the same PID — every step of the change.
    assert!(cmd.contains(WRITE_ENV), "{cmd}");
    assert!(
        cmd.contains("vrcast-limits-wait \"$stop\" \"$id.f\" 50 50 600"),
        "{cmd}"
    );
    assert!(
        cmd.contains("exec env -u VRCAST_LIMITS_HELD bash -c \"$drain\" vrcast-limits-last \"$stop\" \"$id\" 50 50 600"),
        "{cmd}"
    );
    // Both waits read the stop's answer (T634), with no limit on the rounds.
    assert!(
        !cmd.contains("600 signal") && !cmd.contains("50 50 600 0") && !cmd.contains("600 3"),
        "{cmd}"
    );
    // The wait comes after the door, never before it.
    let door = cmd.find("vrcast-limits-door").expect("no door");
    let wait = cmd.find("vrcast-limits-wait").expect("no wait");
    let last = cmd.find("vrcast-limits-last").expect("no last wait");
    assert!(door < wait && wait < last, "{cmd}");
    // A client that vanished does not take the holder with it (the holder's script is
    // single-quoted inside the command, so its own quotes come out escaped).
    assert!(cmd.contains(" HUP PIPE"), "{cmd}");
}

#[test]
fn a_stuck_step_is_waited_for_as_long_as_one_command_may_run_and_no_longer() {
    assert_eq!(DRAIN_CEILING, vrcast_studio_lib::ssh::exec::EXEC_CEILING);
}

#[test]
fn every_step_carries_the_changes_mark_from_its_first_instruction() {
    let step = write_step("abc123.f", "echo 'hi'; exit 3");
    assert!(
        step.starts_with("VRCAST_LIMITS_WRITE='abc123.f' \"${SHELL:-/bin/sh}\" -c "),
        "{step}"
    );
    assert!(step.ends_with(r#"'echo '\''hi'\''; exit 3'"#), "{step}");
}

#[test]
fn the_holder_names_its_door_and_itself() {
    assert_eq!(
        read_locked("LOCKED 41 40\n"),
        Some(Holder { door: 41, held: 40 })
    );
    // The T618 holder's line, or anything else, is not a lock of ours.
    assert_eq!(read_locked("LOCKED 41"), None);
    assert_eq!(read_locked("LOCKED 41 40 39"), None);
    assert_eq!(read_locked("LOCKED 0 40"), None);
    assert_eq!(read_locked("NOT_LOCKED 75"), None);
    assert_eq!(read_locked(""), None);
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

// ---------- T634: the holder reads what the stop said, and lets go only on an end ----------
//
// The very drain script the holder runs, run here by bash against a stand-in for the stop
// of T609 that answers, round by round, what the test tells it to.

use vrcast_studio_lib::server::limits::drain_script;

/// A bash to run the script with — never Windows' own `System32\bash.exe`, which is WSL's
/// and which `Command::new("bash")` would find first there.
fn a_bash() -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|dir| {
            !dir.to_string_lossy()
                .to_ascii_lowercase()
                .contains("system32")
        })
        .flat_map(|dir| [dir.join("bash"), dir.join("bash.exe")])
        .find(|p| p.is_file())
}

/// Run the drain against a stop that answers `answers[i]` in round `i` (the last one from
/// then on), and give back what the drain printed and the grace each round was asked for.
fn drain_against(answers: &[&str], rounds: u32) -> Option<(String, Vec<String>)> {
    let Some(bash) = a_bash() else {
        eprintln!("no bash on this machine: the drain script was not run");
        return None;
    };
    let dir = std::env::temp_dir().join(format!("vrcast-drain-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).expect("no scratch directory");
    let dir_s = dir.to_string_lossy().replace('\\', "/");
    let mut cases = String::new();
    for (i, a) in answers.iter().enumerate() {
        if i + 1 == answers.len() {
            cases.push_str(&format!("  *) printf '%s\n' '{a}' ;;\n"));
        } else {
            cases.push_str(&format!("  {i}) printf '%s\n' '{a}' ;;\n"));
        }
    }
    // The stand-in keeps its own count, and notes the grace (`$4`) it was asked for.
    let stop = format!(
        "n=$(cat '{dir_s}/n' 2>/dev/null || echo 0)\n\
         echo $((n + 1)) > '{dir_s}/n'\n\
         echo \"$4 $5\" >> '{dir_s}/asked'\n\
         case $n in\n{cases}esac\n"
    );
    let out = std::process::Command::new(bash)
        .arg("-c")
        .arg(drain_script())
        .arg("vrcast-limits-wait")
        .arg(stop)
        .arg("abc123.f")
        .args(["50", "50", "600", &rounds.to_string()])
        .output()
        .expect("bash would not run");
    let asked = std::fs::read_to_string(dir.join("asked"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    Some((
        String::from_utf8_lossy(&out.stdout).trim().to_owned(),
        asked,
    ))
}

const ALIVE: &str = "VRCAST_STOP alive 4242:4242 groups: 4242 ";

#[test]
fn still_alive_after_kill_is_not_an_end_and_the_stop_is_sent_again() {
    // QA-21 №1: `alive` used to let the lock go like a confirmed end.
    let Some((said, asked)) = drain_against(&[ALIVE, ALIVE, "VRCAST_STOP kill 5012ms"], 0) else {
        return;
    };
    assert_eq!(said, "VRCAST_STOP kill 5012ms");
    // Three rounds: the first waits the step's own time, the rest signal at once.
    assert_eq!(asked, ["600 signal", "0 signal", "0 signal"]);
}

#[test]
fn with_no_limit_on_the_rounds_the_wait_does_not_give_up_while_a_step_is_alive() {
    let answers = [ALIVE, ALIVE, ALIVE, ALIVE, ALIVE, "VRCAST_STOP term 104ms"];
    let Some((said, asked)) = drain_against(&answers, 0) else {
        return;
    };
    assert_eq!(said, "VRCAST_STOP term 104ms");
    assert_eq!(asked.len(), 6, "{asked:?}");
}

#[test]
fn every_confirmed_end_lets_go_at_once() {
    for end in [
        "VRCAST_STOP none",
        "VRCAST_STOP ended 3000ms",
        "VRCAST_STOP term 104ms",
        "VRCAST_STOP kill 5012ms",
    ] {
        let Some((said, asked)) = drain_against(&[end], 0) else {
            return;
        };
        assert_eq!(said, end);
        assert_eq!(asked.len(), 1, "{end}: {asked:?}");
    }
}

#[test]
fn a_proc_that_cannot_be_read_lets_go_as_before() {
    // The owner's decision (2026-09-30): bash < 4.4, or `/proc` hidden — nothing can ever be
    // confirmed there, and the lock is let go as it was before T628.
    let Some((said, asked)) = drain_against(&["VRCAST_STOP unreadable"], 0) else {
        return;
    };
    assert_eq!(said, "VRCAST_STOP unreadable");
    assert_eq!(asked.len(), 1);
}

#[test]
fn an_answer_that_is_no_answer_is_not_an_end() {
    // Where `/proc` is there (it is wherever this test runs), silence or nonsense from the
    // stop is "not confirmed", never "gone".
    let Some((said, asked)) = drain_against(&["", "bash: something broke", "VRCAST_STOP none"], 0)
    else {
        return;
    };
    assert_eq!(said, "VRCAST_STOP none");
    assert_eq!(asked.len(), 3, "{asked:?}");
}

#[test]
fn still_running_is_not_an_end_either() {
    let Some((said, asked)) = drain_against(
        &["VRCAST_STOP running 4242:4242", "VRCAST_STOP kill 10ms"],
        0,
    ) else {
        return;
    };
    assert_eq!(said, "VRCAST_STOP kill 10ms");
    assert_eq!(asked.len(), 2);
}

#[test]
fn a_wait_with_a_limit_on_the_rounds_says_it_did_not_confirm() {
    let Some((said, asked)) = drain_against(&[ALIVE], 2) else {
        return;
    };
    assert!(
        said.starts_with("VRCAST_DRAIN unconfirmed after 2 rounds: VRCAST_STOP alive 4242:4242"),
        "{said}"
    );
    assert_eq!(asked.len(), 2);
}

// ---------- T635: nothing is put back while a step going forward may still be running ----------

use vrcast_studio_lib::server::limits::{
    barrier_command, read_settled, settle_then, unheard, Settled, BARRIER_GRACE, BARRIER_ROUNDS,
};
use vrcast_studio_lib::ssh::{CommandOutput, SshError};

fn said(exit_code: Option<u32>) -> CommandOutput {
    CommandOutput {
        exit_code,
        stdout: String::from("SWAPPED\n"),
        stderr: String::new(),
    }
}

#[test]
fn a_step_is_heard_only_when_the_server_said_it_ended() {
    assert!(!unheard(&Ok(said(Some(0)))));
    // A step that failed and said so has ended: its failure is heard.
    assert!(!unheard(&Ok(said(Some(7)))));
    // The channel closed with no exit status: the step's end was never said.
    assert!(unheard(&Ok(said(None))));
    // Given up on at the ceiling (`EXEC_CEILING`), or the channel failed.
    assert!(unheard(&Err(SshError::Exec(String::from(
        "the command did not finish within 600s and was given up on: …"
    )))));
}

#[tokio::test]
async fn the_barrier_comes_before_the_putting_back() {
    let order = std::sync::Mutex::new(Vec::new());
    let outcome = settle_then(
        || async {
            order.lock().unwrap().push("barrier");
            Ok(())
        },
        || async {
            order.lock().unwrap().push("undo");
            Ok(())
        },
    )
    .await;
    assert_eq!(outcome, Ok(()));
    assert_eq!(*order.lock().unwrap(), ["barrier", "undo"]);
}

#[tokio::test]
async fn an_unconfirmed_barrier_means_the_putting_back_never_starts() {
    let undone = std::sync::atomic::AtomicBool::new(false);
    let outcome = settle_then(
        || async {
            Err(String::from(
                "a step of this change may still be running: 4242:4242",
            ))
        },
        || async {
            undone.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        },
    )
    .await;
    let err = outcome.expect_err("the putting back went ahead unconfirmed");
    assert!(err.contains("putting back was not started"), "{err}");
    assert!(err.contains("4242:4242"), "{err}");
    assert!(!undone.load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn only_a_confirmed_end_lets_the_putting_back_begin() {
    for end in [
        "VRCAST_STOP none\n",
        "VRCAST_STOP ended 3000ms\n",
        "VRCAST_STOP term 104ms\n",
        "VRCAST_STOP kill 5012ms\n",
    ] {
        assert_eq!(read_settled(end), Settled::Confirmed, "{end}");
    }
    assert_eq!(
        read_settled("VRCAST_STOP unreadable\n"),
        Settled::Unreadable
    );
    for not_an_end in [
        "VRCAST_DRAIN unconfirmed after 3 rounds: VRCAST_STOP alive 4242:4242 groups: 4242\n",
        "VRCAST_STOP alive 4242:4242 groups: 4242\n",
        "VRCAST_STOP running 4242:4242\n",
        "",
        "bash: mapfile: -d: invalid option\n",
    ] {
        assert!(
            matches!(read_settled(not_an_end), Settled::NotConfirmed(_)),
            "{not_an_end:?} was taken for an end"
        );
    }
}

#[test]
fn the_barrier_waits_for_the_steps_going_forward_a_little_then_stops_them_a_few_times() {
    let cmd = barrier_command("abc123.f", BARRIER_GRACE.as_secs());
    assert!(cmd.starts_with("bash -c "), "{cmd}");
    assert!(
        cmd.ends_with(&format!(
            "'abc123.f' 50 50 {} {BARRIER_ROUNDS}",
            BARRIER_GRACE.as_secs()
        )),
        "{cmd}"
    );
    // Not a step of the change: the holder must not wait for its own barrier.
    assert!(!cmd.starts_with("VRCAST_LIMITS_WRITE="), "{cmd}");
    // A barrier with no limit on its rounds would never report.
    assert!(!cmd.ends_with(" 0"), "{cmd}");
}

#[test]
fn the_barrier_against_a_step_that_will_not_die_reports_it() {
    // The real drain, with the barrier's limit on the rounds.
    let Some((said, asked)) = drain_against(&[ALIVE], BARRIER_ROUNDS) else {
        return;
    };
    assert!(
        matches!(read_settled(&said), Settled::NotConfirmed(ref s) if s.contains("4242:4242")),
        "{said}"
    );
    assert_eq!(asked.len(), BARRIER_ROUNDS as usize);
}
