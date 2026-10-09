//! T311 — the recorded session, read back.
//!
//! Two viewers in one fixture and both of them matter, but **the healthy one matters more**.
//! A reading that shouts at people whose picture is perfectly fine is as useless as one that
//! stays quiet on the person whose picture is stopping, and rather more annoying: after the
//! second false alarm it is not consulted again, and then the one true alarm is missed too.
//!
//! The fixture is `tests/fixtures/logs/session.log`. Its **figures** are the recorded ones —
//! 0.53×, 15.9 Mbit/s over the wall clock against 18.6 inside the downloads, the gaps, the set
//! description read three times. Its **addresses and its moment in time are invented**: the
//! real ones are viewers' addresses, and FR-057 does not let those into a repository.

use vrcast_studio_lib::domain::access_log::parse_line;
use vrcast_studio_lib::domain::stalls::{self, Cause, FileShape, Load, NotAViewer, Watcher};
use vrcast_studio_lib::domain::wording::DetailCode;

/// The addresses the fixture was built with.
const STARVING: &str = "203.0.113.24";
const FAST: &str = "198.51.100.7";
const CACHE: &str = "192.0.2.55";
const OURS: &str = "192.0.2.1";

fn recorded() -> Vec<vrcast_studio_lib::domain::access_log::Request> {
    include_str!("../fixtures/logs/session.log")
        .lines()
        .filter_map(|l| parse_line(l).ok())
        .collect()
}

fn sifted() -> stalls::Sifted {
    stalls::sift(&recorded(), &[String::from(OURS)])
}

#[test]
fn the_recorded_viewer_gets_the_recorded_figures() {
    let s = sifted();
    let it = s
        .watchers
        .iter()
        .find(|w| w.client_ip == STARVING)
        .expect("the viewer the whole case is about was not in the report");

    // 0.53× — one second of film for every two seconds lived.
    let ratio = it.content_ratio.expect("no ratio was worked out");
    assert!(
        (ratio - 0.53).abs() < 0.005,
        "content against real time came to {ratio}, and the recorded case was 0.53"
    );
    assert!(it.starving());

    // **The wall clock, not the downloads.** These are the two numbers the whole method rests
    // on being told apart, and getting them the wrong way round would mean advising a viewer
    // whose link is fine to go and buy a better one.
    let wall = it.mbit_s.expect("no speed over the wall clock");
    let inside = it
        .in_download_mbit_s
        .expect("no speed inside the downloads");
    assert!(
        (wall - 15.9).abs() < 0.1,
        "their link came to {wall}, and the recorded figure is 15.9"
    );
    assert!(
        (inside - 18.6).abs() < 0.1,
        "inside the downloads came to {inside}, and the recorded figure is 18.6"
    );
    assert!(
        wall < inside,
        "the two figures came back the wrong way round"
    );

    // The player jumping its playhead: 10, 12, 13 and 15 were never asked for.
    assert_eq!(it.skipped, vec![10, 12, 13, 15]);
    // The set description read three times: a healthy player reads it once a session.
    assert_eq!(it.restarts, 2);
    assert_eq!(it.reinits, 1);
}

#[test]
fn the_viewer_with_a_full_buffer_is_not_called_starving() {
    // The one that matters most. Their requests come in bursts with long gaps between them —
    // which is exactly what a stall looks like if timing is read on its own.
    let s = sifted();
    let it = s
        .watchers
        .iter()
        .find(|w| w.client_ip == FAST)
        .expect("the healthy viewer was dropped from the report");

    assert!(
        !it.starving(),
        "a viewer ahead of the clock was called starving: ratio {:?}",
        it.content_ratio
    );
    assert!(it.skipped.is_empty(), "gaps invented where there are none");
    assert_eq!(it.restarts, 0);

    let verdict = stalls::explain(it, None, None);
    assert_eq!(
        verdict.cause,
        Cause::NothingWrong,
        "the server was blamed for a viewer who is keeping up"
    );
}

#[test]
fn a_cache_and_our_own_checks_are_set_aside() {
    let s = sifted();
    assert!(
        s.watchers.iter().all(|w| w.client_ip != OURS),
        "our own checks were counted as a viewer, and they are the busiest address in the log"
    );
    assert!(
        s.watchers.iter().all(|w| w.client_ip != CACHE),
        "a cache taking two segments was counted as a viewer"
    );

    let ours = s
        .set_aside
        .iter()
        .find(|a| a.client_ip == OURS)
        .expect("the server's own address is not accounted for at all");
    assert_eq!(ours.why, NotAViewer::OurOwnCheck);

    let cache = s
        .set_aside
        .iter()
        .find(|a| a.client_ip == CACHE)
        .expect("the cache is not accounted for at all");
    assert_eq!(cache.why, NotAViewer::TooLittle { segments: 2 });
}

#[test]
fn the_server_asleep_points_at_the_viewers_link() {
    let s = sifted();
    let mut it = s
        .watchers
        .iter()
        .find(|w| w.client_ip == STARVING)
        .unwrap()
        .clone();
    // What their rung needs, as the film's own set on the server says (T705). Without it
    // there is no number for the link to fall short of — see the test below.
    it.need_mbit = Some(1000.0);

    // Low processor, little read from the disk, a small amount going out — and a viewer
    // hanging. This is the server saying it is not the one at fault.
    let asleep = Load {
        cpu_busy: 0.04,
        disk_read_mb_s: 1.0,
        out_mbit_s: 18.0,
        capacity_mbit_s: 940.0,
        cache_small: false,
    };
    let verdict = stalls::explain(&it, Some(&asleep), None);
    assert_eq!(verdict.cause, Cause::ViewerLink);
    // With the figures behind it, because this conclusion is sometimes wrong.
    assert_eq!(
        verdict.say.params.get("mbit_s").and_then(|v| v.as_f64()),
        Some(15.9)
    );
    assert_eq!(
        verdict.say.params.get("need_mbit").and_then(|v| v.as_f64()),
        Some(1000.0),
        "the link is blamed without the number it falls short of"
    );
}

#[test]
fn without_what_the_film_needs_the_link_is_not_blamed() {
    // T705 (QA-26 №12): the catch-all used to tell any viewer behind real time that their
    // link was too thin — a viewer with a speed of 0 over two requests among them — with no
    // figure for what the link would have had to carry.
    let s = sifted();
    let it = s.watchers.iter().find(|w| w.client_ip == STARVING).unwrap();
    assert_eq!(it.need_mbit, None);
    let asleep = Load {
        cpu_busy: 0.04,
        disk_read_mb_s: 1.0,
        out_mbit_s: 18.0,
        capacity_mbit_s: 940.0,
        cache_small: false,
    };
    let verdict = stalls::explain(it, Some(&asleep), None);
    assert_eq!(verdict.cause, Cause::Unclear);
    assert_eq!(verdict.say.key.as_str(), "STALLS_UNCLEAR");
}

#[test]
fn a_file_measured_for_another_film_is_not_applied() {
    // T705: the measured file is one film; a viewer of another is not judged by it.
    let s = sifted();
    let it = s.watchers.iter().find(|w| w.client_ip == STARVING).unwrap();
    let elsewhere = FileShape {
        average_mbit: 12.0,
        peak_10s_mbit: 150.0,
        slug: Some(String::from("another-film")),
    };
    let unsaid = FileShape {
        slug: None,
        ..elsewhere.clone()
    };
    for shape in [&elsewhere, &unsaid] {
        let verdict = stalls::explain(it, None, Some(shape));
        assert_ne!(verdict.cause, Cause::TheFileItself, "{shape:?}");
        assert_ne!(verdict.cause, Cause::ThePlayer, "{shape:?}");
    }
}

#[test]
fn a_busy_server_takes_the_blame_itself() {
    let s = sifted();
    let it = s.watchers.iter().find(|w| w.client_ip == STARVING).unwrap();

    let flat_out = Load {
        cpu_busy: 0.7,
        disk_read_mb_s: 40.0,
        out_mbit_s: 900.0,
        capacity_mbit_s: 940.0,
        cache_small: false,
    };
    let verdict = stalls::explain(it, Some(&flat_out), None);
    assert_eq!(
        verdict.cause,
        Cause::ServerLink,
        "the viewer was blamed for the server's own link being full"
    );
}

#[test]
fn a_wide_link_and_a_peaky_file_point_at_the_file() {
    let s = sifted();
    let it = s.watchers.iter().find(|w| w.client_ip == STARVING).unwrap();

    // Their 15.9 Mbit/s carries the average with room to spare and does not come near the
    // ten-second peak. Uncapped VBR, and the cure is a re-encode, not a better line.
    let peaky = FileShape {
        average_mbit: 12.0,
        peak_10s_mbit: 150.0,
        // The film they are watching (T705): a measurement is applied to its own film only.
        slug: it.watching.clone(),
    };
    let asleep = Load {
        cpu_busy: 0.04,
        disk_read_mb_s: 1.0,
        out_mbit_s: 18.0,
        capacity_mbit_s: 940.0,
        cache_small: false,
    };
    let verdict = stalls::explain(it, Some(&asleep), Some(&peaky));
    assert_eq!(verdict.cause, Cause::TheFileItself);

    // And a file whose peaks their link carries easily does **not** get the blame.
    let flat = FileShape {
        average_mbit: 30.0,
        peak_10s_mbit: 34.0,
        slug: it.watching.clone(),
    };
    assert_eq!(
        stalls::explain(it, Some(&asleep), Some(&flat)).cause,
        Cause::ViewerLink
    );
}

#[test]
fn segment_numbers_are_read_off_the_name() {
    assert_eq!(
        stalls::segment_number("/videos/film/v30/seg_00012.m4s"),
        Some(12)
    );
    assert_eq!(
        stalls::segment_number("/videos/film/v30/seg_00007.ts"),
        Some(7)
    );
    // A name with no number in it is not a segment number of zero.
    assert_eq!(stalls::segment_number("/videos/film/v30/init.mp4"), None);
}

#[test]
fn a_line_caught_mid_write_does_not_take_the_report_with_it() {
    // The fixture ends on half a line, because the tail of a file being written always might.
    let all = include_str!("../fixtures/logs/session.log").lines().count();
    let read = recorded().len();
    assert_eq!(
        all - read,
        1,
        "something other than the half-written last line failed to parse"
    );
    assert!(!sifted().watchers.is_empty());
}

#[test]
fn a_viewer_whose_link_carries_it_is_not_told_their_link_is_the_problem() {
    // ⚠ **The numbers here were measured on the stand, 2026-09-04** (T482). A viewer at a
    // ratio of 0.39 — plainly behind — was told the fault was their link, while the speed
    // *inside* their downloads was 30.35 Mbit/s against a film needing 4. A hundred and sixty
    // times what it takes. The link was fine and they were not asking in between; the answer
    // sent them to argue with their provider.
    //
    // There was no cause for this at all, and `ViewerLink` — the last branch, reached when
    // nothing else fits — took it.
    let watcher = Watcher {
        client_ip: String::from("203.0.113.7"),
        watching: Some(String::from("film")),
        segments: 5,
        bytes: 9_500_000,
        first: time::OffsetDateTime::UNIX_EPOCH,
        last: time::OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(60),
        elapsed_s: 60.0,
        content_ratio: Some(0.39),
        mbit_s: Some(0.19),
        in_download_mbit_s: Some(30.35),
        skipped: Vec::new(),
        restarts: 0,
        reinits: 0,
        failures: 0,
        rung: Some(String::from("v4")),
        need_mbit: None,
        live: None,
    };
    let film = FileShape {
        average_mbit: 4.0,
        peak_10s_mbit: 6.0,
        slug: Some(String::from("film")),
    };
    let verdict = stalls::explain(&watcher, None, Some(&film));
    assert_eq!(
        verdict.cause,
        Cause::ThePlayer,
        "a viewer whose link carries the film seven times over was blamed for their link"
    );
    // And with both numbers, because the whole argument is the difference between them.
    assert_eq!(
        verdict
            .say
            .params
            .get("in_download_mbit_s")
            .and_then(|v| v.as_f64()),
        Some(30.35)
    );
    assert_eq!(
        verdict
            .say
            .params
            .get("average_mbit")
            .and_then(|v| v.as_f64()),
        Some(4.0)
    );
}

#[test]
fn a_viewer_whose_link_really_is_thin_is_still_told_so() {
    // The other side of the same rule, so it does not turn into "never blame the link". Here
    // the downloads themselves crawl: under what the film needs even while they are running.
    let watcher = Watcher {
        client_ip: String::from("203.0.113.8"),
        watching: Some(String::from("film")),
        segments: 5,
        bytes: 900_000,
        first: time::OffsetDateTime::UNIX_EPOCH,
        last: time::OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(60),
        elapsed_s: 60.0,
        content_ratio: Some(0.30),
        mbit_s: Some(0.12),
        in_download_mbit_s: Some(1.2),
        skipped: Vec::new(),
        restarts: 0,
        reinits: 0,
        failures: 0,
        rung: Some(String::from("v4")),
        need_mbit: None,
        live: None,
    };
    let film = FileShape {
        average_mbit: 4.0,
        peak_10s_mbit: 6.0,
        slug: Some(String::from("film")),
    };
    let verdict = stalls::explain(&watcher, None, Some(&film));
    assert_eq!(verdict.cause, Cause::ViewerLink);
}

#[test]
fn without_a_film_to_compare_against_nothing_is_claimed_about_the_player() {
    // Saying "not the link" needs a number for what the link would have to carry — and so,
    // since T705, does saying "the link". Without one, neither is claimed.
    let watcher = Watcher {
        client_ip: String::from("203.0.113.9"),
        watching: Some(String::from("film")),
        segments: 5,
        bytes: 9_500_000,
        first: time::OffsetDateTime::UNIX_EPOCH,
        last: time::OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(60),
        elapsed_s: 60.0,
        content_ratio: Some(0.39),
        mbit_s: Some(0.19),
        in_download_mbit_s: Some(30.35),
        skipped: Vec::new(),
        restarts: 0,
        reinits: 0,
        failures: 0,
        rung: Some(String::from("v4")),
        need_mbit: None,
        live: None,
    };
    let verdict = stalls::explain(&watcher, None, None);
    assert_eq!(verdict.cause, Cause::Unclear);

    // With the rung's own need from the server, the same viewer is placed: their downloads
    // carry 30.35 against 4 — the player.
    let known = Watcher {
        need_mbit: Some(4.0),
        ..watcher
    };
    assert_eq!(stalls::explain(&known, None, None).cause, Cause::ThePlayer);
}

// ---------- T711: the viewer's own connection tells the link from the player ----------

/// Two readings of `ss -tin`, five seconds apart, in the shape the container printed them
/// (2026-08-26): one viewer, one connection, its figures as given.
fn polls(
    ip: &str,
    (acked1, busy1, held1, segs1, resent1): (u64, u64, u64, u64, u64),
    (acked2, busy2, held2, segs2, resent2): (u64, u64, u64, u64, u64),
) -> (
    vrcast_studio_lib::domain::connections::Poll,
    vrcast_studio_lib::domain::connections::Poll,
) {
    let one = |at: &str, acked: u64, busy: u64, held: u64, segs: u64, resent: u64| {
        let rwnd = if held > 0 {
            format!(" rwnd_limited:{held}ms(50.0%)")
        } else {
            String::new()
        };
        vrcast_studio_lib::domain::connections::parse_poll(&format!(
            "{at}\nRecv-Q Send-Q Local Address:Port Peer Address:Port Process\n\
             0      0      [::ffff:172.18.0.2]:80 [::ffff:{ip}]:43512\n\
             \t cubic wscale:7,7 rto:204 rtt:0.5/0.25 mss:1448 cwnd:10 bytes_acked:{acked} \
             segs_out:{segs} delivery_rate 9400000bps busy:{busy}ms{rwnd} retrans:0/{resent}\n"
        ))
        .expect("the reading would not parse")
    };
    (
        one("1787707000.0", acked1, busy1, held1, segs1, resent1),
        one("1787707005.0", acked2, busy2, held2, segs2, resent2),
    )
}

fn starving_with(live: Option<stalls::LiveLink>, need: Option<f64>) -> Watcher {
    Watcher {
        client_ip: String::from("203.0.113.40"),
        watching: Some(String::from("film")),
        segments: 5,
        bytes: 9_500_000,
        first: time::OffsetDateTime::UNIX_EPOCH,
        last: time::OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(60),
        elapsed_s: 60.0,
        content_ratio: Some(0.4),
        mbit_s: Some(1.2),
        // What the log said for the viewer held to 0.4 Mbit/s on 2026-10-09: the server
        // finishing pieces into its buffers, not the viewer receiving them.
        in_download_mbit_s: Some(6.16),
        skipped: Vec::new(),
        restarts: 0,
        reinits: 0,
        failures: 0,
        rung: Some(String::from("v3")),
        need_mbit: need,
        live,
    }
}

#[test]
fn the_live_link_is_worked_out_from_what_the_viewer_confirmed_over_the_stretch() {
    // 2.5 MB confirmed in 5 s, busy the whole time, nothing held, 4 of 2000 sent again.
    let (before, after) = polls(
        "203.0.113.40",
        (1_000_000, 10_000, 0, 1000, 0),
        (3_500_000, 15_000, 0, 3000, 4),
    );
    let links = stalls::live_links(&before, &after);
    let it = links["203.0.113.40"];
    assert!((it.span_s - 5.0).abs() < 1e-6);
    assert!((it.mbit_s - 4.0).abs() < 1e-6, "{it:?}");
    assert_eq!(it.busy_share, Some(1.0));
    assert!((it.busy_mbit_s.unwrap() - 4.0).abs() < 1e-6);
    assert_eq!(it.held_share, Some(0.0));
    assert!((it.resent_share.unwrap() - 0.002).abs() < 1e-9);
}

#[test]
fn a_connection_seen_in_one_reading_only_says_nothing() {
    let (before, mut after) = polls(
        "203.0.113.40",
        (1_000_000, 10_000, 0, 1000, 0),
        (3_500_000, 15_000, 0, 3000, 0),
    );
    // A new connection in the second reading: its whole life is not five seconds' delivery.
    for row in &mut after.rows {
        row.peer_port += 1;
    }
    assert!(stalls::live_links(&before, &after).is_empty());
}

#[test]
fn a_viewer_whose_side_is_full_is_the_player_whatever_the_log_says() {
    // Busy all five seconds, their side full for 4.5 of them: what arrived was not taken.
    let (before, after) = polls(
        "203.0.113.40",
        (1_000_000, 10_000, 8_000, 1000, 0),
        (1_250_000, 15_000, 12_500, 1200, 0),
    );
    let live = stalls::live_links(&before, &after)["203.0.113.40"];
    let verdict = stalls::explain(&starving_with(Some(live), Some(2.0)), None, None);
    assert_eq!(verdict.cause, Cause::ThePlayer);
    assert_eq!(verdict.say.key, DetailCode::StallsPlayerLive);
    assert_eq!(
        verdict.say.params.get("held_pct").and_then(|v| v.as_u64()),
        Some(90)
    );
    // Without the rung's need too: being full is an answer on its own.
    let verdict = stalls::explain(&starving_with(Some(live), None), None, None);
    assert_eq!(verdict.say.key, DetailCode::StallsPlayerLive);
}

#[test]
fn a_working_connection_carrying_less_than_the_rung_needs_is_the_link() {
    // Busy all the time, never held, 0.25 MB in 5 s = 0.4 Mbit/s against a rung needing 2 —
    // the case the log read as 6.16 Mbit/s "inside the downloads".
    let (before, after) = polls(
        "203.0.113.40",
        (1_000_000, 10_000, 0, 1000, 0),
        (1_250_000, 15_000, 0, 1200, 30),
    );
    let live = stalls::live_links(&before, &after)["203.0.113.40"];
    let verdict = stalls::explain(&starving_with(Some(live), Some(2.0)), None, None);
    assert_eq!(verdict.cause, Cause::ViewerLink, "{:?}", verdict.say);
    assert_eq!(verdict.say.key, DetailCode::StallsViewerLinkLive);
    assert_eq!(
        verdict.say.params.get("live_mbit").and_then(|v| v.as_f64()),
        Some(0.4)
    );
    assert_eq!(
        verdict.say.params.get("need_mbit").and_then(|v| v.as_f64()),
        Some(2.0)
    );
    assert_eq!(
        verdict
            .say
            .params
            .get("resent_pct")
            .and_then(|v| v.as_f64()),
        Some(15.0)
    );
}

#[test]
fn a_working_connection_carrying_what_the_rung_needs_is_not_the_link() {
    let (before, after) = polls(
        "203.0.113.40",
        (1_000_000, 10_000, 0, 1000, 0),
        (6_000_000, 15_000, 0, 5000, 0),
    );
    let live = stalls::live_links(&before, &after)["203.0.113.40"];
    let verdict = stalls::explain(&starving_with(Some(live), Some(2.0)), None, None);
    assert_eq!(verdict.cause, Cause::ThePlayer);
    assert_eq!(verdict.say.key, DetailCode::StallsLinkFineLive);
}

#[test]
fn a_connection_mostly_idle_leaves_the_log_to_say_what_it_can() {
    // One second busy of five: they were hardly asking just now — a full buffer, perhaps,
    // while the log's minutes say behind. The live reading decides nothing.
    let (before, after) = polls(
        "203.0.113.40",
        (1_000_000, 10_000, 0, 1000, 0),
        (1_100_000, 11_000, 0, 1100, 0),
    );
    let live = stalls::live_links(&before, &after)["203.0.113.40"];
    let with = stalls::explain(&starving_with(Some(live), Some(2.0)), None, None);
    let without = stalls::explain(&starving_with(None, Some(2.0)), None, None);
    assert_eq!(with, without);
    assert_eq!(with.say.key, DetailCode::StallsThePlayer);
}

#[test]
fn a_viewer_keeping_up_is_left_alone_whatever_the_connection_says() {
    let (before, after) = polls(
        "203.0.113.40",
        (1_000_000, 10_000, 0, 1000, 0),
        (1_250_000, 15_000, 0, 1200, 0),
    );
    let live = stalls::live_links(&before, &after)["203.0.113.40"];
    let mut fine = starving_with(Some(live), Some(2.0));
    fine.content_ratio = Some(1.3);
    assert_eq!(
        stalls::explain(&fine, None, None).cause,
        Cause::NothingWrong
    );
}

#[test]
fn a_side_full_whenever_anything_was_sent_is_the_player_even_if_little_was() {
    // Measured on the stand 2026-10-09: a slow reader's connection busy a fifth of the
    // stretch, their side full for nine tenths of that. A full side stops the sending, which
    // is why such a connection is rarely busy at all.
    let (before, after) = polls(
        "203.0.113.40",
        (1_000_000, 10_000, 0, 1000, 0),
        (1_900_000, 11_100, 990, 1600, 0),
    );
    let live = stalls::live_links(&before, &after)["203.0.113.40"];
    assert!(live.busy_share.unwrap() < stalls::LIVE_BUSY);
    let verdict = stalls::explain(&starving_with(Some(live), Some(2.0)), None, None);
    assert_eq!(verdict.say.key, DetailCode::StallsPlayerLive);

    // Next to nothing sent at all: nothing to go on, the log says what it can.
    let (before, after) = polls(
        "203.0.113.40",
        (1_000_000, 10_000, 0, 1000, 0),
        (1_010_000, 10_100, 100, 1010, 0),
    );
    let live = stalls::live_links(&before, &after)["203.0.113.40"];
    let verdict = stalls::explain(&starving_with(Some(live), Some(2.0)), None, None);
    assert_eq!(verdict.say.key, DetailCode::StallsThePlayer);
}
