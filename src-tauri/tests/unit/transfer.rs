//! T081 — tests for the upload's pure logic (US3).
//!
//! What is checked is what an upload breaks on in real life: carrying on after a break, a
//! source that was swapped, the rate limiter over a long stretch, and the time estimate
//! after a pause.

use std::time::{Duration, Instant};
use vrcast_studio_lib::domain::progress_estimate::ProgressEstimate;
use vrcast_studio_lib::domain::rate_limit::RateLimiter;
use vrcast_studio_lib::domain::remote_name::{self, NameVerdict};
use vrcast_studio_lib::domain::transfer::{self, ResumeDecision, ResumeToken, WINDOW_BYTES};

// ---------- where to carry on from (T077) ----------

#[test]
fn an_empty_staged_file_means_starting_over() {
    assert_eq!(
        transfer::decide_resume(0, 1_000_000, WINDOW_BYTES),
        ResumeDecision::FromStart
    );
}

#[test]
fn carrying_on_steps_back_by_one_window() {
    // The last write may have broken off midway: its tail is in the file already but is not
    // whole. Rewriting the window is cheaper than guessing.
    let temp = 100 * 1024 * 1024;
    match transfer::decide_resume(temp, 500 * 1024 * 1024, WINDOW_BYTES) {
        ResumeDecision::Continue { offset } => {
            assert_eq!(offset, temp - WINDOW_BYTES);
        }
        other => panic!("got {other:?}"),
    }
}

#[test]
fn the_step_back_does_not_go_past_the_start_of_the_file() {
    // Less than a window was sent: there is nowhere to step back to, so we start over.
    assert_eq!(
        transfer::decide_resume(1024, 10_000_000, WINDOW_BYTES),
        ResumeDecision::FromStart
    );
}

#[test]
fn a_fully_sent_file_is_not_sent_again() {
    // The checksum comparison and the entry into serving are left — but not the transfer.
    assert_eq!(
        transfer::decide_resume(1_000_000, 1_000_000, WINDOW_BYTES),
        ResumeDecision::AlreadyComplete
    );
}

#[test]
fn a_staged_file_larger_than_the_source_is_not_almost_done() {
    // A sign that the source was swapped, or that the wrong file lies on the server.
    // Carrying on would glue two different files together, and it would only be found at
    // the comparison — when the time has already been spent.
    match transfer::decide_resume(2_000_000, 1_000_000, WINDOW_BYTES) {
        ResumeDecision::Mismatch { temp, total } => {
            assert_eq!(temp, 2_000_000);
            assert_eq!(total, 1_000_000);
        }
        other => panic!("a divergence was taken for a carry-on: {other:?}"),
    }
}

#[test]
fn a_resume_position_survives_writing_and_reading() {
    let token = ResumeToken {
        remote_temp: String::from("/var/lib/.vrcast-uploads/t1.film.part"),
        remote_name: String::from("film_22.mp4"),
        local_path: Some(String::from("F:/video/film 22.mp4")),
        source_size: 32_000_000_000,
        source_modified: Some(String::from("1756108800")),
        media_id: Some(String::from("m-42")),
    };
    let back = ResumeToken::parse(&token.to_json()).expect("the position would not read");
    assert_eq!(back, token);
}

#[test]
fn a_position_holding_the_upload_s_old_speed_cap_still_reads() {
    // T717: an upload had a cap of its own (`limit_bps`) until it was folded into the one
    // shared «Send speed». A position written then still carries the field; it reads as before.
    let old = r#"{"remote_temp":"/tmp/x.part","remote_name":"film.mp4","local_path":"F:/film.mp4",
        "media_id":null,"limit_bps":2097152,"source_size":1000,"source_modified":null}"#;
    let back = ResumeToken::parse(old).expect("a position with a cap stopped reading");
    assert_eq!(back.local_path.as_deref(), Some("F:/film.mp4"));
    assert!(!back.to_json().contains("limit_bps"));
}

#[test]
fn a_position_from_an_earlier_version_reads_without_a_path_to_the_source() {
    // Records made by earlier versions sit in the databases of people already using the
    // application. Failing to parse them would mean losing unfinished transfers on an
    // upgrade — quietly, because the parse returns `None`.
    //
    // The record is deliberately taken as it stands: it holds none of the fields that came
    // later, and it does hold `uploaded_hint`, which no longer exists. Both must survive.
    let old = r#"{"remote_temp":"/tmp/x.part","remote_name":"film.mp4",
        "source_size":1000,"source_modified":null,"uploaded_hint":500}"#;
    let back = ResumeToken::parse(old).expect("an earlier record stopped reading");
    assert_eq!(back.local_path, None);
    assert_eq!(back.remote_name, "film.mp4");
}

#[test]
fn a_swapped_source_is_noticed_before_the_transfer() {
    // Otherwise carrying on appends the tail of one file to the beginning of another, and it
    // only comes to light at the checksum comparison — after an hour of transferring.
    let token = ResumeToken {
        remote_temp: String::from("/tmp/x.part"),
        remote_name: String::from("film.mp4"),
        local_path: Some(String::from("/home/u/film.mp4")),
        source_size: 1_000,
        source_modified: Some(String::from("1756108800")),
        media_id: None,
    };

    assert!(token.matches_source(1_000, Some("1756108800")));
    assert!(
        !token.matches_source(2_000, Some("1756108800")),
        "a different size was taken for the same file"
    );
    assert!(
        !token.matches_source(1_000, Some("1756195200")),
        "the file was rebuilt to the same size, and it went unnoticed"
    );
    // The time is unknown — we make do with the size, but do not invent a divergence.
    assert!(token.matches_source(1_000, None));
}

// ---------- the rate limit (T078) ----------

#[test]
fn with_no_limit_there_is_nothing_to_wait_for() {
    let mut r = RateLimiter::new(None);
    let now = Instant::now();
    assert_eq!(r.delay_for(100_000_000, now), Duration::ZERO);
}

#[test]
fn a_limit_of_zero_counts_as_no_limit() {
    // Zero as a limit would mean "never send anything" — which is not what a person means by
    // leaving the field empty.
    let mut r = RateLimiter::new(Some(0));
    assert_eq!(r.limit_bps(), None);
    assert_eq!(r.delay_for(1_000_000, Instant::now()), Duration::ZERO);
}

#[test]
fn the_average_speed_stays_within_the_limit_over_a_long_stretch() {
    // The limiter's main property. Checked against modelled time: there is no point waiting
    // ten real seconds for a test.
    let limit = 1_000_000u64; // bytes per second
    let mut r = RateLimiter::new(Some(limit));

    let start = Instant::now();
    let mut now = start;
    let chunk = 64 * 1024u64;
    let mut sent = 0u64;

    // Sent without pauses, moving the clock on by exactly what the limiter asks — as a real
    // sender would.
    for _ in 0..400 {
        let wait = r.delay_for(chunk, now);
        now += wait;
        sent += chunk;
    }

    let seconds = now.saturating_duration_since(start).as_secs_f64();
    let actual = sent as f64 / seconds.max(0.001);
    assert!(
        actual <= limit as f64 * 1.15,
        "the average speed of {actual:.0} bytes/s went over the limit of {limit}"
    );
    assert!(
        actual > limit as f64 * 0.5,
        "the limiter throttles harder than asked: {actual:.0} instead of {limit}"
    );
}

#[test]
fn a_short_idle_spell_does_not_turn_into_lost_speed() {
    // The allowance lets what has built up go out right after a short pause — otherwise a
    // transfer would move in jerks, strictly by the clock.
    let mut r = RateLimiter::new(Some(1_000_000));
    let now = Instant::now();
    r.delay_for(1_000_000, now);

    let later = now + Duration::from_millis(900);
    assert_eq!(
        r.delay_for(500_000, later),
        Duration::ZERO,
        "after a pause it had to wait although the allowance had built up"
    );
}

// ---------- speed and time left (T079) ----------

#[test]
fn the_speed_is_counted_over_the_last_few_seconds() {
    let mut e = ProgressEstimate::new(Duration::from_secs(10));
    let start = Instant::now();

    for i in 0..=10u64 {
        e.record(start + Duration::from_secs(i), i * 1_000_000);
    }

    let speed = e.speed_bps().expect("the speed was not worked out");
    assert!(
        (900_000..=1_100_000).contains(&speed),
        "speed {speed} instead of roughly a million"
    );
}

#[test]
fn four_hundred_hours_are_not_shown_after_a_pause() {
    // The rule this module exists for. What built up before a pause no longer describes what
    // is happening: not thrown away, the estimate turns monstrous and a person decides
    // everything is broken.
    let mut e = ProgressEstimate::new(Duration::from_secs(10));
    let start = Instant::now();

    for i in 0..=5u64 {
        e.record(start + Duration::from_secs(i), i * 1_000_000);
    }

    // Half an hour of idleness.
    let after_pause = start + Duration::from_secs(1805);
    e.record(after_pause, 5_000_000);
    assert_eq!(
        e.speed_bps(),
        None,
        "right after a pause the speed was invented out of thin air"
    );

    // Off again — the speed is counted over the new readings, not over the whole history.
    for i in 1..=5u64 {
        e.record(
            after_pause + Duration::from_secs(i),
            5_000_000 + i * 2_000_000,
        );
    }
    let speed = e
        .speed_bps()
        .expect("the speed was not worked out after carrying on");
    assert!(
        (1_800_000..=2_200_000).contains(&speed),
        "speed {speed} was counted with the idle spell included"
    );
}

#[test]
fn the_time_left_is_not_invented_when_the_speed_is_unknown() {
    let e = ProgressEstimate::default();
    assert_eq!(e.eta(1_000_000), None);
}

/// T659, QA-24A №10: a capped link, one four-megabyte window every twelve seconds. Every
/// sample used to be taken for the first after a pause and thrown away, and the speed and
/// time left never appeared.
#[test]
fn a_slow_link_one_window_every_twelve_seconds_gets_a_speed_and_a_time_left() {
    let mut e = ProgressEstimate::default();
    let start = Instant::now();
    const MIB4: u64 = 4 * 1024 * 1024;
    // The transfer's own starting point, as `transfer_once` records it.
    e.record(start, 0);
    for i in 1..=5u64 {
        e.record(start + Duration::from_secs(i * 12), i * MIB4);
        let speed = e
            .speed_bps()
            .unwrap_or_else(|| panic!("no speed after window {i} on a slow link"));
        let expected = MIB4 / 12;
        assert!(
            speed.abs_diff(expected) <= expected / 100,
            "window {i}: speed {speed}, expected about {expected}"
        );
        let left = e
            .eta(30_000_000_000)
            .unwrap_or_else(|| panic!("no time left after window {i}"));
        assert!(left > Duration::from_secs(80_000) && left < Duration::from_secs(90_000));
    }
}

/// The QA probe's own shape: no starting point, samples only after each window. The first
/// window cannot say anything (there is nothing to measure against); from the second on,
/// the speed is known.
#[test]
fn slow_samples_without_a_starting_point_give_a_speed_from_the_second_on() {
    let mut e = ProgressEstimate::default();
    let now = Instant::now();
    for i in 1..=5u64 {
        e.record(now + Duration::from_secs(i * 12), i * 4 * 1024 * 1024);
        if i == 1 {
            assert_eq!(e.speed_bps(), None);
        } else {
            assert!(e.speed_bps().is_some(), "no speed after sample {i}");
            assert!(
                e.eta(30_000_000_000).is_some(),
                "no time left after sample {i}"
            );
        }
    }
}

/// A stall longer than any window can take is still not measured across — the transfer
/// has given the connection up by then, and the gap is not transfer time.
#[test]
fn a_gap_longer_than_any_window_is_not_measured_across() {
    let mut e = ProgressEstimate::default();
    let start = Instant::now();
    e.record(start, 0);
    e.record(start + Duration::from_secs(12), 4_000_000);
    e.record(
        start + Duration::from_secs(12) + Duration::from_secs(601),
        4_100_000,
    );
    assert_eq!(e.speed_bps(), None);
}

#[test]
fn too_short_a_stretch_does_not_make_gigabits_believable() {
    // Dividing by thousandths of a second turns any jitter into an unbelievable number.
    let mut e = ProgressEstimate::default();
    let start = Instant::now();
    e.record(start, 0);
    e.record(start + Duration::from_millis(3), 5_000_000);
    assert_eq!(e.speed_bps(), None);
}

// ---------- names (T080) ----------

#[test]
fn the_file_is_staged_beside_the_serving_rather_than_inside_it() {
    // A web server hands out everything it sees in the serving directory. A file still being
    // downloaded must not lie there for a second.
    // лежать не должен ни секунды.
    let staging = remote_name::staging_dir("/var/lib/vrcast/videos").expect("nowhere to stage");
    assert_eq!(staging, "/var/lib/vrcast/.vrcast-uploads");
    assert!(
        !staging.starts_with("/var/lib/vrcast/videos"),
        "the staging happens inside the serving directory"
    );
}

#[test]
fn a_serving_directory_at_the_root_leaves_no_room_to_stage_in() {
    // Putting what is still downloading into the serving itself will not do, and beside it
    // there is nowhere. An honest refusal beats quietly breaking the main rule.
    assert_eq!(remote_name::staging_dir("/videos"), None);
}

#[test]
fn the_staged_file_s_name_depends_only_on_the_final_name() {
    // The whole resume scheme rests on this: the position is the staged file's size, and it
    // has to be findable before the task is created (the pre-start checks) and after the
    // application restarts. Tying it to the task id would break that.
    let dir = "/var/lib/vrcast/.vrcast-uploads";
    let a = remote_name::staging_file(dir, "film.mp4");
    let b = remote_name::staging_file(dir, "film.mp4");
    assert_eq!(a, b, "one and the same name gave different staged files");
    assert!(a.ends_with(".part"));

    // Different final names — different staged files.
    assert_ne!(a, remote_name::staging_file(dir, "other.mp4"));

    // Dangerous characters are defused here too: the staged path also goes into a command.
    let dangerous = remote_name::staging_file(dir, "../../etc/passwd");
    assert!(
        dangerous.starts_with(dir),
        "the staged file went outside the staging directory: {dangerous}"
    );
}

#[test]
fn dangerous_characters_in_a_name_are_defused() {
    // Properties are checked rather than the exact look of the result. Just how a defused
    // name looks is a matter of taste and may change; what matters is that one cannot escape
    // into another directory, hide a file, or tear a command apart on the server.
    for dangerous_name in [
        "../../etc/passwd",
        "film\nrm -rf /.mp4",
        "  .hidden.mp4  ",
        "C:\\Windows\\system32",
        ".",
        "..",
    ] {
        let clean = remote_name::sanitize(dangerous_name);

        assert!(
            !clean.contains('/') && !clean.contains('\\'),
            "a path separator survived in \"{clean}\" (from \"{dangerous_name}\")"
        );
        assert!(
            !clean.starts_with('.'),
            "the name stayed hidden: \"{clean}\" (from \"{dangerous_name}\")"
        );
        assert!(
            !clean.contains('\n') && !clean.contains('\r') && !clean.contains('\0'),
            "a newline survived in \"{clean}\""
        );
        assert_eq!(
            clean.trim(),
            clean,
            "spaces were left at the edges: \"{clean}\""
        );
    }

    // An ordinary name passes untouched: defusing must not spoil what is already fine.
    // The non-Latin case is deliberate — people's titles are written in their own language.
    assert_eq!(
        remote_name::sanitize("Backrooms_22.mp4"),
        "Backrooms_22.mp4"
    );
    assert_eq!(
        remote_name::sanitize("Фильм — финал.mp4"),
        "Фильм — финал.mp4"
    );
    // Two dots inside a name are legitimate, and there is no reason to touch them.
    assert_eq!(remote_name::sanitize("film..final.mp4"), "film..final.mp4");
}

#[test]
fn a_name_already_taken_is_a_warning_rather_than_a_bar() {
    // Replacing is legitimate, but it has consequences, and a person must know them before
    // rather than after their viewers complain (FR-039).
    let existing = vec![String::from("film.mp4")];

    assert_eq!(
        remote_name::check_name("film.mp4", &existing, true),
        NameVerdict::Exists { cdn_cached: true },
        "with a CDN set, the cache was not mentioned"
    );
    assert_eq!(
        remote_name::check_name("film.mp4", &existing, false),
        NameVerdict::Exists { cdn_cached: false }
    );
    assert_eq!(
        remote_name::check_name("other.mp4", &existing, false),
        NameVerdict::Ok
    );
}

#[test]
fn the_serving_s_housekeeping_names_cannot_be_taken() {
    assert_eq!(
        remote_name::check_name("library.json", &[], false),
        NameVerdict::Reserved
    );
    assert_eq!(
        remote_name::check_name("   ", &[], false),
        NameVerdict::Empty
    );
}

/// T701 — the one cap on sending a video's rungs: shared, changed while a send is on its way,
/// and «none» or zero holding nothing back. On its own instance: the application's one
/// (`SENDING`) is the process's, and the other tests here send through it.
#[test]
fn the_sending_cap_is_one_for_all_and_changes_at_once() {
    use vrcast_studio_lib::domain::rate_limit::SharedLimit;
    let cap = SharedLimit::default();
    let t0 = Instant::now();
    assert_eq!(cap.limit_bps(), None);
    assert_eq!(cap.delay_for(10_000_000, t0), Duration::ZERO);

    // 1000 bytes a second: a second's allowance, then waiting.
    cap.set(Some(1000));
    assert_eq!(cap.limit_bps(), Some(1000));
    assert_eq!(cap.delay_for(1000, t0), Duration::ZERO);
    // A second sender shares it: no fresh allowance of its own.
    assert_eq!(cap.delay_for(1000, t0), Duration::from_secs(1));

    // The same cap set again keeps what was earned and owed.
    cap.set(Some(1000));
    assert_eq!(cap.delay_for(1000, t0), Duration::from_secs(1));

    // A change holds from the next block.
    cap.set(Some(4000));
    assert_eq!(cap.limit_bps(), Some(4000));
    assert_eq!(cap.delay_for(4000, t0), Duration::ZERO);
    assert_eq!(cap.delay_for(2000, t0), Duration::from_millis(500));

    for none in [None, Some(0)] {
        cap.set(none);
        assert_eq!(cap.limit_bps(), None);
        assert_eq!(cap.delay_for(10_000_000, t0), Duration::ZERO);
    }
}
