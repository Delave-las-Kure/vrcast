//! T304 — the two rules a log digest exists to hold on to.
//!
//! A request longer than thirty seconds is **normally fine**, and 206 outnumbering 200 is
//! what "ranges are being served" actually looks like. Both are easy to get backwards, and
//! both backwards produce a screen of red on a machine that is working.

use vrcast_studio_lib::domain::access_log::parse_line;
use vrcast_studio_lib::domain::log_digest;

fn read() -> Vec<vrcast_studio_lib::domain::access_log::Request> {
    include_str!("../fixtures/logs/session.log")
        .lines()
        .filter_map(|l| parse_line(l).ok())
        .collect()
}

fn line(ip: &str, uri: &str, status: u16, size: u64, duration: f64) -> String {
    format!(
        r#"{{"level":"info","ts":1756000000.0,"msg":"handled request","request":{{"client_ip":"{ip}","uri":"{uri}"}},"status":{status},"size":{size},"duration":{duration}}}"#
    )
}

#[test]
fn the_recorded_session_reduces_to_what_happened_in_it() {
    let requests = read();
    let d = log_digest::digest(&requests, 1);

    assert_eq!(d.requests, requests.len());
    assert_eq!(d.unreadable, 1);
    assert_eq!(d.lines, requests.len() + 1);
    // Four addresses appeared. Which of them were viewers is `domain::stalls`'s question, and
    // answering it in two places would mean two answers.
    assert_eq!(d.addresses, 4);
    assert_eq!(d.ranges_dominate(), Some(true));
    assert_eq!(d.failed(), 0);
    assert!(d.bytes_out > 0);
    assert!(d.from.is_some() && d.to.is_some());
    assert!(d.top_paths.len() <= log_digest::TOP_N);
}

#[test]
fn a_long_request_that_delivered_is_not_a_complaint() {
    // Forty seconds and 200 megabytes: a player taking its time over a big range. This is
    // most of the long requests on a healthy server, and counting them as faults is how a
    // panel teaches people to ignore it.
    let long_and_fast = line("203.0.113.1", "/videos/film.mp4", 206, 200_000_000, 40.0);
    let requests: Vec<_> = [long_and_fast]
        .iter()
        .filter_map(|l| parse_line(l).ok())
        .collect();
    let d = log_digest::digest(&requests, 0);

    assert_eq!(d.long_requests.len(), 1, "the long request was not noticed");
    assert!(
        !d.long_requests[0].slow,
        "a request delivering {} Mbit/s was called slow for lasting 40 seconds",
        d.long_requests[0].mbit_s
    );
    assert_eq!(d.slow_long_requests(), 0);
}

#[test]
fn a_long_request_that_delivered_nothing_is() {
    // Ninety seconds for two megabytes — under a fifth of a megabit. Slower than the lightest
    // rung this application will ever build, so no reading of it is innocent.
    let long_and_slow = line("203.0.113.2", "/videos/film.mp4", 206, 2_000_000, 90.0);
    let requests: Vec<_> = [long_and_slow]
        .iter()
        .filter_map(|l| parse_line(l).ok())
        .collect();
    let d = log_digest::digest(&requests, 0);

    assert!(d.long_requests[0].slow);
    assert_eq!(d.slow_long_requests(), 1);
}

#[test]
fn whole_files_outnumbering_ranges_is_a_finding() {
    let lines: Vec<String> = (0..5)
        .map(|i| line("203.0.113.3", &format!("/videos/f{i}.mp4"), 200, 1000, 0.1))
        .chain(std::iter::once(line(
            "203.0.113.3",
            "/videos/f9.mp4",
            206,
            1000,
            0.1,
        )))
        .collect();
    let requests: Vec<_> = lines.iter().filter_map(|l| parse_line(l).ok()).collect();
    let d = log_digest::digest(&requests, 0);

    assert_eq!(
        d.ranges_dominate(),
        Some(false),
        "ranges not being served went unremarked"
    );
}

#[test]
fn nothing_at_all_is_not_the_same_as_no() {
    // An empty stretch has to say "there is nothing here", never "ranges are not served".
    let d = log_digest::digest(&[], 0);
    assert_eq!(d.ranges_dominate(), None);
    assert_eq!(d.addresses, 0);
    assert!(d.from.is_none());
}

#[test]
fn failures_are_grouped_by_what_failed() {
    let lines: Vec<String> =
        std::iter::repeat_n(line("203.0.113.4", "/videos/gone.mp4", 404, 0, 0.01), 3)
            .chain(std::iter::once(line(
                "203.0.113.4",
                "/videos/broken/master.m3u8",
                500,
                0,
                0.01,
            )))
            .collect();
    let requests: Vec<_> = lines.iter().filter_map(|l| parse_line(l).ok()).collect();
    let d = log_digest::digest(&requests, 0);

    assert_eq!(d.failed(), 4);
    assert_eq!(d.failures.len(), 2);
    // Most first: three of one thing is a pattern, one of another is an accident.
    assert_eq!(d.failures[0].status, 404);
    assert_eq!(d.failures[0].times, 3);
}
