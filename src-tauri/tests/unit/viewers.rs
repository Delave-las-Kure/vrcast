//! T163 — the rules the list of viewers is built by.
//!
//! The lines and the tables here were taken off a running container on 2026-08-26, not
//! invented. That matters more than it sounds: two of the things this code has to cope with
//! could not have been guessed at, and were found by looking — the server reports an
//! ordinary IPv4 viewer as `[::ffff:10.10.0.3]`, and `ss` writes half its fields with a
//! colon and half with a space.

use time::{Duration, OffsetDateTime};

use vrcast_studio_lib::domain::access_log::{self, Asked, LineProblem};
use vrcast_studio_lib::domain::connections::{self, ConnectionRow};
use vrcast_studio_lib::domain::viewers::{Problem, Session, VariantFacts, Viewer};

// ---------- the access log ----------

/// A line as Caddy really writes it, shortened only in its headers.
const REAL_LINE: &str = r#"{"level":"info","ts":1787707296.3399053,"logger":"http.log.access.log0","msg":"handled request","request":{"remote_ip":"10.10.0.3","remote_port":"34560","client_ip":"10.10.0.3","proto":"HTTP/1.1","method":"GET","host":"stream.example","uri":"/videos/demo/v2/seg1.ts","headers":{"User-Agent":["curl/8.8.0"]}},"bytes_read":0,"user_id":"","duration":0.000453683,"size":2500000,"status":200,"resp_headers":{"Content-Type":["video/mp2t"]}}"#;

#[test]
fn a_real_line_is_read_whole() {
    let request = access_log::parse_line(REAL_LINE).expect("a real line would not parse");

    assert_eq!(request.client_ip, "10.10.0.3");
    assert_eq!(request.path, "/videos/demo/v2/seg1.ts");
    assert_eq!(request.status, 200);
    assert_eq!(request.bytes, 2_500_000);
    assert!((request.duration_s - 0.000_453_683).abs() < 1e-9);
    // By the server's clock, not this machine's.
    assert_eq!(request.at.unix_timestamp(), 1_787_707_296);
}

#[test]
fn a_line_caught_halfway_through_being_written_is_passed_over() {
    // Following the end of a file catches this constantly: the line is read before it has
    // finished being written. Were it to stop the parsing, watching would break off several
    // times a minute for no reason at all.
    let cut = &REAL_LINE[..REAL_LINE.len() / 2];
    assert_eq!(access_log::parse_line(cut), Err(LineProblem::NotJson));
}

#[test]
fn caddys_own_notes_are_told_apart_from_requests() {
    // Caddy writes its working notes into the same stream. They are not damage, and must be
    // told apart from a line that is a request but is missing something — a whole file of
    // the latter would mean the serving writes something other than what is read here, and
    // that is worth knowing.
    let note = r#"{"level":"info","ts":1787707200.0,"msg":"server running","logger":"http"}"#;
    assert_eq!(
        access_log::parse_line(note),
        Err(LineProblem::NotARequest),
        "a note about the server was taken for a request"
    );

    let no_address = r#"{"level":"info","ts":1787707200.0,"msg":"handled request","request":{"uri":"/videos/a.mp4"},"status":200}"#;
    assert_eq!(
        access_log::parse_line(no_address),
        Err(LineProblem::Incomplete("client_ip"))
    );
}

#[test]
fn a_name_with_spaces_and_cyrillic_is_read_back_as_it_is() {
    // A file uploaded before the application existed obeys none of its naming rules. Its
    // name reaches the log encoded, and left that way it would match nothing in the
    // library — the viewer would be shown watching an unknown something.
    let encoded = "/videos/%D0%9C%D0%BE%D0%B9%20%D1%84%D0%B8%D0%BB%D1%8C%D0%BC.mp4";
    let line = format!(
        r#"{{"level":"info","ts":1787707296.0,"msg":"handled request","request":{{"client_ip":"10.0.0.1","uri":"{encoded}?v=2"}},"status":200,"size":10}}"#
    );
    let request = access_log::parse_line(&line).expect("the line would not parse");

    assert_eq!(request.path, "/videos/Мой фильм.mp4");
    // The query is not part of what was asked for: `?v=2` must not make it another file.
    assert_eq!(
        access_log::what_was_asked_for(&request.path),
        Asked::DirectFile {
            name: String::from("Мой фильм.mp4")
        }
    );
}

#[test]
fn the_three_ways_of_serving_are_told_apart() {
    use access_log::what_was_asked_for as asked;

    assert_eq!(
        asked("/videos/film.mp4"),
        Asked::DirectFile {
            name: String::from("film.mp4")
        }
    );
    assert_eq!(
        asked("/videos/demo/master.m3u8"),
        Asked::SetDescription {
            slug: String::from("demo"),
            shortened: false
        }
    );
    // The shortened description a viewer under a limit is handed (Phase 6). Told apart so
    // that such a viewer does not look like someone asking for something unrecognised.
    assert_eq!(
        asked("/videos/_slow/demo/master.m3u8"),
        Asked::SetDescription {
            slug: String::from("demo"),
            shortened: true
        }
    );
    assert_eq!(
        asked("/videos/demo/v2/stream.m3u8"),
        Asked::RungPlaylist {
            slug: String::from("demo"),
            rung: String::from("v2")
        }
    );
    assert_eq!(
        asked("/videos/demo/v2/seg1.ts"),
        Asked::Segment {
            slug: String::from("demo"),
            rung: String::from("v2")
        }
    );
    assert_eq!(asked("/healthz"), Asked::Other);
}

#[test]
fn asking_what_there_is_is_not_yet_watching() {
    // A description is asked for once, before anything is pulled. Counting that as watching
    // would put someone in the list who opened the link and went away.
    assert!(!access_log::what_was_asked_for("/videos/demo/master.m3u8").is_pulling_video());
    assert!(access_log::what_was_asked_for("/videos/demo/v2/seg1.ts").is_pulling_video());
    assert!(access_log::what_was_asked_for("/videos/film.mp4").is_pulling_video());
}

// ---------- the connection table ----------

/// Exactly what `ss -tin` printed in the container on 2026-08-26, for a viewer deliberately
/// held to 200 kB/s.
const REAL_SS: &str = "Recv-Q Send-Q       Local Address:Port       Peer Address:Port Process\n\
0      3251366 [::ffff:10.10.0.2]:80   [::ffff:10.10.0.3]:59498\n\
\t cubic wscale:7,7 rto:220 backoff:1 rtt:17.327/23.09 ato:40 mss:65483 pmtu:65535 rcvmss:536 advmss:65483 cwnd:10 ssthresh:18 bytes_sent:7294299 bytes_retrans:523864 bytes_acked:6770435 bytes_received:88 segs_out:162 segs_in:51 data_segs_out:155 data_segs_in:1 send 302339701bps lastsnd:550 lastrcv:4130 lastack:320 pacing_rate 362802400bps delivery_rate 19402370368bps delivered:156 busy:4130ms rwnd_limited:4070ms(98.5%) retrans:0/8 dsack_dups:8 rcv_space:65483 rcv_ssthresh:65483 notsent:3251366 minrtt:0.002\n";

#[test]
fn a_real_connection_table_is_read_whole() {
    let rows = connections::parse(REAL_SS);
    assert_eq!(rows.len(), 1, "the record was not found: {rows:?}");
    let row = &rows[0];

    // The address in the form the log writes it. Without this the two sources never meet:
    // no connection would match any request, every viewer would come out watching an
    // unknown something, and nothing would look broken.
    assert_eq!(row.peer_ip, "10.10.0.3");
    assert_eq!(row.peer_port, 59498);

    // Half the fields are written with a colon and half with a space. A parser that knew
    // only the colon would read no speed at all — and silently, the field simply being
    // absent rather than wrong.
    assert_eq!(row.bytes_acked, 6_770_435);
    assert_eq!(row.segs_out, 162);
    assert_eq!(
        row.retrans_total, 8,
        "retrans:0/8 — the total, not the current"
    );
    assert_eq!(row.delivery_rate_bps, Some(19_402_370_368));
    assert_eq!(row.receiver_limited_share, Some(0.985));
}

#[test]
fn the_mapped_form_of_an_address_is_brought_to_the_plain_one() {
    for (raw, expected) in [
        ("[::ffff:10.10.0.3]:59498", ("10.10.0.3", 59498)),
        ("10.10.0.3:59498", ("10.10.0.3", 59498)),
        ("[2001:db8::1]:443", ("2001:db8::1", 443)),
    ] {
        let (ip, port) =
            connections::normalise_address(raw).unwrap_or_else(|| panic!("{raw} would not parse"));
        assert_eq!((ip.as_str(), port), expected);
    }
}

#[test]
fn a_record_without_its_details_still_counts_as_a_viewer() {
    // Losing a viewer over the absence of the details would be worse than showing them
    // without: they are plainly there.
    let rows = connections::parse("ESTAB 0 0 10.0.0.1:80 10.0.0.9:5000\n");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].peer_ip, "10.0.0.9");
    assert_eq!(rows[0].bytes_acked, 0);
}

#[test]
fn a_poll_without_a_readable_time_yields_nothing() {
    // Deliberately nothing rather than falling back on this machine's clock: the fallback
    // would produce speeds that look right and are not.
    assert!(connections::parse_poll("not-a-time\nESTAB 0 0 1.2.3.4:80 5.6.7.8:9\n").is_none());

    let good = format!("1787707296.339\n{REAL_SS}");
    let poll = connections::parse_poll(&good).expect("a good poll would not parse");
    assert_eq!(poll.at.unix_timestamp(), 1_787_707_296);
    assert_eq!(poll.rows.len(), 1);
}

// ---------- bringing the two together ----------

fn at(seconds: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_787_707_000 + seconds).unwrap()
}

fn row(
    ip: &str,
    bytes_acked: u64,
    segs_out: u64,
    retrans: u64,
    limited: Option<f64>,
) -> ConnectionRow {
    ConnectionRow {
        peer_ip: ip.to_owned(),
        peer_port: 50000,
        bytes_acked,
        segs_out,
        retrans_total: retrans,
        delivery_rate_bps: None,
        receiver_limited_share: limited,
    }
}

fn request(ip: &str, path: &str, when: i64) -> vrcast_studio_lib::domain::access_log::Request {
    vrcast_studio_lib::domain::access_log::Request {
        client_ip: ip.to_owned(),
        path: path.to_owned(),
        status: 200,
        bytes: 1000,
        duration_s: 0.1,
        at: at(when),
    }
}

/// A library that answers for the fixture's quality set.
fn library(asked: &Asked) -> VariantFacts {
    match asked.library_key() {
        Some("demo") => VariantFacts {
            media_id: Some(String::from("media-demo")),
            variant: asked.rung().map(str::to_owned),
            required_bps: match asked.rung() {
                Some("v1") => Some(10_000_000),
                Some("v2") => Some(5_000_000),
                _ => None,
            },
        },
        Some(name) => VariantFacts {
            media_id: Some(format!("media-{name}")),
            variant: Some(name.to_owned()),
            required_bps: Some(8_000_000),
        },
        None => VariantFacts::default(),
    }
}

fn only(viewers: &[Viewer]) -> &Viewer {
    assert_eq!(viewers.len(), 1, "expected one viewer, got {viewers:?}");
    &viewers[0]
}

#[test]
fn someone_pulling_is_seen_before_any_request_of_theirs_has_finished() {
    // The case the whole arrangement exists for. A film served as one file is one request
    // lasting the whole showing, and its line appears only at the end. Were the list built
    // on the log alone, the screen would stay empty for the whole two hours.
    let mut session = Session::default();
    session.note_connections(&[row("10.0.0.9", 1_000_000, 100, 0, None)], at(0));

    let viewers = session.active(at(1));
    let viewer = only(&viewers);
    assert_eq!(viewer.ip, "10.0.0.9");
    // And what they are watching is honestly unknown rather than made up.
    assert_eq!(viewer.media_id, None);
    assert_eq!(viewer.variant, None);
}

#[test]
fn a_connection_is_attributed_by_the_addresss_most_recent_request() {
    let mut session = Session::default();
    session.note_request(&request("10.0.0.9", "/videos/demo/v2/seg0.ts", 0), &library);
    session.note_connections(&[row("10.0.0.9", 1_000_000, 100, 0, None)], at(1));

    let viewers = session.active(at(2));
    let viewer = only(&viewers);
    assert_eq!(viewer.media_id.as_deref(), Some("media-demo"));
    assert_eq!(viewer.variant.as_deref(), Some("v2"));
    assert_eq!(viewer.required_bps, Some(5_000_000));
}

#[test]
fn a_refused_request_does_not_say_what_is_being_watched() {
    // Naming a medium on the strength of a refusal would put a viewer in front of a film
    // they were never shown.
    let mut session = Session::default();
    let mut refused = request("10.0.0.9", "/videos/demo/v2/seg0.ts", 0);
    refused.status = 404;
    session.note_request(&refused, &library);

    assert_eq!(only(&session.active(at(1))).media_id, None);
}

#[test]
fn several_connections_from_one_address_are_one_viewer() {
    // A player opens more than one. Showing them as several people watching the same thing
    // would be a lie about how many there are — and the count goes into the medium's card.
    let mut session = Session::default();
    session.note_connections(
        &[
            row("10.0.0.9", 1_000_000, 100, 0, None),
            row("10.0.0.9", 2_000_000, 200, 0, None),
        ],
        at(0),
    );
    assert_eq!(session.active(at(1)).len(), 1);
}

#[test]
fn the_speed_is_the_growth_of_what_was_confirmed_over_the_window() {
    let mut session = Session::default();
    session.note_connections(&[row("10.0.0.9", 0, 0, 0, None)], at(0));
    // Ten megabytes over ten seconds is eight megabits a second.
    session.note_connections(&[row("10.0.0.9", 10_000_000, 1000, 0, None)], at(10));

    assert_eq!(only(&session.active(at(11))).delivery_bps, Some(8_000_000));
}

#[test]
fn no_speed_is_shown_until_there_is_enough_to_work_one_out_from() {
    // A viewer who has just appeared has no speed yet, and saying so is honest. A figure
    // made from a one-second sample would be noise shown as a measurement.
    let mut session = Session::default();
    session.note_connections(&[row("10.0.0.9", 0, 0, 0, None)], at(0));
    session.note_connections(&[row("10.0.0.9", 500_000, 50, 0, None)], at(1));

    assert_eq!(only(&session.active(at(2))).delivery_bps, None);
}

#[test]
fn a_narrow_link_is_marked_and_a_full_buffer_is_not() {
    // The two look alike from the outside and are not alike at all. A player that has
    // filled its buffer stops reading, and the delivered speed drops right off — with
    // nothing wrong. Marking that would light the flag for healthy viewers, and a flag that
    // cries wolf is worse than no flag.
    let narrow = {
        let mut session = Session::default();
        session.note_request(&request("10.0.0.9", "/videos/demo/v1/seg0.ts", 0), &library);
        session.note_connections(&[row("10.0.0.9", 0, 0, 0, Some(0.02))], at(0));
        // One megabyte over ten seconds — 800 kbit/s against the 10 Mbit/s the rung needs.
        session.note_connections(&[row("10.0.0.9", 1_000_000, 100, 0, Some(0.02))], at(10));
        session.active(at(11)).remove(0)
    };
    assert!(
        narrow.problems.contains(&Problem::SlowLink),
        "a viewer getting a twelfth of what they need was not marked: {narrow:?}"
    );

    let buffered = {
        let mut session = Session::default();
        session.note_request(&request("10.0.0.8", "/videos/demo/v1/seg0.ts", 0), &library);
        session.note_connections(&[row("10.0.0.8", 0, 0, 0, Some(0.98))], at(0));
        session.note_connections(&[row("10.0.0.8", 1_000_000, 100, 0, Some(0.98))], at(10));
        session.active(at(11)).remove(0)
    };
    assert!(
        !buffered.problems.contains(&Problem::SlowLink),
        "a viewer whose player had simply stopped reading was accused of a bad link: {buffered:?}"
    );
}

#[test]
fn a_lossy_link_is_marked_and_ordinary_loss_is_not() {
    let mark = |retrans_grown: u64| {
        let mut session = Session::default();
        session.note_connections(&[row("10.0.0.9", 0, 0, 0, None)], at(0));
        session.note_connections(
            &[row("10.0.0.9", 10_000_000, 1000, retrans_grown, None)],
            at(10),
        );
        session.active(at(11)).remove(0).problems
    };

    // Every link loses something; five segments in a thousand is a working link.
    assert!(!mark(5).contains(&Problem::Retransmits));
    // A twentieth of everything sent going twice is not.
    assert!(mark(50).contains(&Problem::Retransmits));
}

#[test]
fn a_viewer_who_stops_leaves_the_active_ones_and_stays_in_the_history() {
    let mut session = Session::new(Duration::seconds(30));
    session.note_request(&request("10.0.0.9", "/videos/demo/v2/seg0.ts", 0), &library);
    session.note_connections(&[row("10.0.0.9", 1_000, 10, 0, None)], at(0));

    // On the very threshold they are still watching: a list that puts people out a moment
    // early flickers, and a flickering list is read as broken.
    assert_eq!(session.active(at(30)).len(), 1);
    assert_eq!(session.active(at(31)).len(), 0);

    assert_eq!(session.retire_gone(at(31)), 1);
    assert!(session.active(at(31)).is_empty());
    assert_eq!(
        session.history().len(),
        1,
        "the departed viewer was forgotten"
    );
    assert_eq!(session.history()[0].media_id.as_deref(), Some("media-demo"));
}

#[test]
fn the_threshold_can_be_changed_without_losing_who_is_watching() {
    let mut session = Session::new(Duration::seconds(30));
    session.note_connections(&[row("10.0.0.9", 1_000, 10, 0, None)], at(0));
    assert_eq!(session.active(at(45)).len(), 0);

    session.set_threshold(Duration::seconds(60));
    assert_eq!(
        session.active(at(45)).len(),
        1,
        "changing the setting threw away the session"
    );
}
