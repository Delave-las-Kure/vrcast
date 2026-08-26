//! T149, T151, T152 — a check of the oscilloscope itself.
//!
//! Everything the checks of Phases 4 and 6 will say about the code rests on this fixture
//! being what it claims: that the container really serves, that the access log really gets
//! written, that the numbers in the description of the quality set are the segments' own
//! numbers, and that two viewers really arrive from two addresses. An unchecked measuring
//! instrument is not a measuring instrument — a check standing on it would be measuring
//! its faults and calling them the code's.
//!
//! Nothing about the application is checked here. That is deliberate: this is about the
//! fixture, and mixing the two would mean a failure that does not say which of them broke.

use std::time::Duration;

use super::fixture::TestServer;
use super::hls_fixture::{lay_out_direct_file, lay_out_ladder, RUNGS, SEGMENT_SECONDS, VIDEO_DIR};
use super::viewer::Viewer;

#[test]
fn the_container_serves_a_direct_file_and_a_quality_set() {
    let server = TestServer::start().expect("the container would not come up");
    let film =
        lay_out_direct_file(&server, "film.mp4", 3_000_000).expect("the file was not laid out");
    let master = lay_out_ladder(&server, "demo").expect("the quality set was not laid out");
    let viewer = Viewer::attach(&server).expect("the viewer would not attach");

    let direct = viewer
        .probe(&film)
        .expect("the direct file was not asked for");
    assert_eq!(
        direct.status, 200,
        "the direct file is not served: {direct:?}"
    );
    assert_eq!(
        direct.bytes, 3_000_000,
        "the direct file came back the wrong length: {direct:?}"
    );

    let description = viewer
        .probe(&master)
        .expect("the description was not asked for");
    assert_eq!(
        description.status, 200,
        "the description is not served: {description:?}"
    );
    // The MIME type matters and is not decoration: without it Caddy hands a playlist out as
    // audio, and a player refuses it. It is set on the live server, so it is set here.
    assert_eq!(
        description.content_type, "application/vnd.apple.mpegurl",
        "the description is served under the wrong type: {description:?}"
    );

    // Every rung, not the first. That is what FR-047 will demand of the application, and
    // the fixture has to be able to answer for all of them before the check can.
    for rung in &RUNGS {
        let playlist = viewer
            .probe(&format!("/videos/demo/{}/stream.m3u8", rung.name))
            .unwrap_or_else(|e| panic!("rung {} was not asked for: {e}", rung.name));
        assert_eq!(
            playlist.status, 200,
            "rung {} is not served: {playlist:?}",
            rung.name
        );
        let segment = viewer
            .probe(&format!("/videos/demo/{}/seg0.ts", rung.name))
            .unwrap_or_else(|e| panic!("a segment of rung {} was not asked for: {e}", rung.name));
        assert_eq!(
            segment.bytes, rung.segments[0],
            "a segment of rung {} came back the wrong length: {segment:?}",
            rung.name
        );
    }
}

#[test]
fn what_the_description_declares_is_what_the_segments_are() {
    let server = TestServer::start().expect("the container would not come up");
    lay_out_ladder(&server, "demo").expect("the quality set was not laid out");

    // Measured on the server, out of the files themselves — not out of the table the
    // description was written from. Comparing the fixture against its own constants would
    // only show that it agrees with itself, and FR-046 is about the other thing entirely:
    // the declared figures being the variants' figures.
    for rung in &RUNGS {
        let listing = server
            .exec_inside(&format!(
                "for f in {VIDEO_DIR}/demo/{}/seg*.ts; do stat -c %s \"$f\"; done",
                rung.name
            ))
            .expect("the segments would not be measured");
        let sizes: Vec<u64> = listing
            .split_whitespace()
            .map(|s| {
                s.parse()
                    .unwrap_or_else(|_| panic!("a size would not parse: \"{s}\""))
            })
            .collect();
        assert_eq!(
            sizes.len(),
            rung.segments.len(),
            "rung {} has the wrong number of segments on disk",
            rung.name
        );

        let measured_peak = sizes.iter().copied().max().unwrap() * 8 / SEGMENT_SECONDS;
        let measured_average =
            sizes.iter().sum::<u64>() * 8 / (SEGMENT_SECONDS * sizes.len() as u64);
        assert_eq!(
            measured_peak,
            rung.peak_bps(),
            "the peak declared for rung {} is not the peak of its segments",
            rung.name
        );
        assert_eq!(
            measured_average,
            rung.average_bps(),
            "the average declared for rung {} is not the average of its segments",
            rung.name
        );
        // And the two must not coincide: were they equal, a check that mixed them up would
        // pass, and the mixing up would only be found on a real ladder.
        assert_ne!(
            rung.peak_bps(),
            rung.average_bps(),
            "the peak and the average of rung {} are equal — the fixture cannot tell them apart",
            rung.name
        );
    }
}

#[test]
fn every_request_lands_in_the_access_log() {
    let server = TestServer::start().expect("the container would not come up");
    let film =
        lay_out_direct_file(&server, "film.mp4", 200_000).expect("the file was not laid out");
    let viewer = Viewer::attach(&server).expect("the viewer would not attach");

    viewer.probe(&film).expect("the file was not asked for");

    let log = server
        .wait_in_access_log("/videos/film.mp4", Duration::from_secs(10))
        .expect("the request left no line in the access log");

    // The address is in it, and it is the viewer's own. This is the field the whole list of
    // viewers is keyed by: were the log to hold the gateway's address instead, every viewer
    // would merge into one and the merging would look like correct behaviour.
    assert!(
        log.contains(&format!("\"client_ip\":\"{}\"", viewer.ip())),
        "the log does not hold the viewer's address {}. The log:\n{log}",
        viewer.ip()
    );
    // And the fields the viewers are assembled from (R-02).
    for field in ["\"status\":200", "\"duration\":", "\"size\":200000"] {
        assert!(
            log.contains(field),
            "the log has no {field} — the parsing is written against this shape. The log:\n{log}"
        );
    }
}

#[test]
fn two_viewers_arrive_from_two_addresses() {
    let server = TestServer::start().expect("the container would not come up");
    let film =
        lay_out_direct_file(&server, "film.mp4", 100_000).expect("the file was not laid out");

    let one = Viewer::attach(&server).expect("the first viewer would not attach");
    let two = Viewer::attach(&server).expect("the second viewer would not attach");

    // The point of the whole arrangement. Were the requests sent from this machine, both
    // would arrive from the network gateway's address, the list of viewers would show one
    // person instead of two, and every check of milestone C would agree with that.
    assert_ne!(
        one.ip(),
        two.ip(),
        "both viewers arrive from one address — there is nobody to tell apart"
    );

    one.probe(&film).expect("the first viewer got nothing");
    two.probe(&film).expect("the second viewer got nothing");

    let log = server
        .wait_in_access_log(
            &format!("\"client_ip\":\"{}\"", two.ip()),
            Duration::from_secs(10),
        )
        .expect("the second viewer left no line in the access log");
    assert!(
        log.contains(&format!("\"client_ip\":\"{}\"", one.ip())),
        "the first viewer left no line in the access log. The log:\n{log}"
    );
}

#[test]
fn a_slow_viewer_really_pulls_slowly() {
    let server = TestServer::start().expect("the container would not come up");
    // Big enough that a narrow link cannot finish it while the check is looking: were the
    // pulling to end early, "the viewer is not in the list" would mean "they had already
    // left" rather than anything about the code.
    let film =
        lay_out_direct_file(&server, "heavy.mp4", 20_000_000).expect("the file was not laid out");
    let viewer = Viewer::attach(&server).expect("the viewer would not attach");

    viewer
        .start_watching(&film, Some("100k"))
        .expect("the watching would not start");
    viewer
        .wait_until_watching(Duration::from_secs(10))
        .expect("the viewer never began pulling");

    // Twenty megabytes at a hundred kilobytes a second is over three minutes: after a few
    // seconds it must still be going. This is what makes a viewer whose link is too narrow
    // for what they are getting — there is nothing to check the SlowLink mark against
    // without one (FR-053).
    std::thread::sleep(Duration::from_secs(3));
    assert!(
        viewer.is_watching(),
        "the slow viewer finished pulling already — the speed ceiling is not being applied"
    );

    viewer.stop_watching().expect("the watching would not stop");
    // Stopping is what the check of leaving the active list rests on (FR-055), so it too
    // has to be known to work rather than assumed.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while viewer.is_watching() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        !viewer.is_watching(),
        "the viewer went on pulling after being told to stop"
    );
}
