//! T166 — the key check of the phase: both ways of serving, against a real server.
//!
//! Two people watching at once — one a directly served file, one a quality set — and both
//! must appear, with what they are watching named rightly, within ten seconds (SC-005).
//!
//! **This check is written to fail if only the second of them appears.** That is not a
//! detail: a quality set leaves a line in the log for every segment, so a list built on the
//! log alone shows that viewer perfectly well. The one watching a single file leaves no
//! line until the showing ends, and it is for their sake that the connection table is
//! polled at all (R-02). A check that looked at one viewer would pass with half the
//! mechanism missing.

//! **Why every check here spells out a runtime with threads.** The viewer helpers drive
//! Docker, and they block the thread while they do. On the single-threaded runtime a test
//! gets by default, that blocking stops everything else on it — including the reading of
//! the connection to the server. The watching then starts late, misses what it was meant to
//! see, and the check fails for a reason that has nothing to do with the code.

use std::sync::Arc;
use std::time::{Duration, Instant};

use time::Duration as TimeDuration;
use tokio::sync::mpsc;

use super::fixture::{key_path, TestServer, KEY_PASSPHRASE};
use super::hls_fixture::{lay_out_direct_file, lay_out_ladder, RUNGS};
use super::viewer::Viewer;
use vrcast_studio_lib::domain::access_log::Asked;
use vrcast_studio_lib::domain::geo::Place;
use vrcast_studio_lib::domain::viewers::{VariantFacts, Viewer as KnownViewer};
use vrcast_studio_lib::server::viewers::{self, ViewerContext, ViewersUpdate};
use vrcast_studio_lib::ssh::{fingerprint, Connection, Credentials, ServerAddress};

/// What the library would say. Standing in for it here, so that the check is about the
/// watching rather than about the catalogue.
struct Library;

impl ViewerContext for Library {
    fn facts(&self, asked: &Asked) -> VariantFacts {
        match asked.library_key() {
            Some("demo") => VariantFacts {
                media_id: Some(String::from("media-demo")),
                variant: asked.rung().map(str::to_owned),
                required_bps: asked
                    .rung()
                    .and_then(|name| RUNGS.iter().find(|r| r.name == name))
                    .map(|r| r.peak_bps()),
            },
            Some("film.mp4") => VariantFacts {
                media_id: Some(String::from("media-film")),
                variant: Some(String::from("film.mp4")),
                required_bps: Some(8_000_000),
            },
            _ => VariantFacts::default(),
        }
    }

    fn place(&self, _ip: &str) -> Place {
        // The container's addresses are private ones; no table in the world can say where
        // they are, and the honest answer is that it is not determined. The lookup itself
        // is checked in the unit tests, on addresses a table may answer for.
        Place::default()
    }
}

async fn connect(server: &TestServer) -> Connection {
    let a = ServerAddress::new(server.host(), server.port);
    let fp = fingerprint::probe(&a)
        .await
        .expect("the fingerprint was not obtained");
    Connection::connect(
        a,
        "root",
        Credentials::Key {
            path: key_path(),
            passphrase: Some(KEY_PASSPHRASE.to_owned()),
        },
        &fp,
    )
    .await
    .expect("connecting failed")
}

/// Wait for an update in which every one of `wanted` addresses is present.
///
/// The waiting is on the stream rather than on a sleep: the check must be about how long
/// the application really takes, and a sleep long enough to be safe would hide exactly the
/// delay SC-005 puts a limit on.
async fn wait_for(
    updates: &mut mpsc::Receiver<ViewersUpdate>,
    wanted: &[&str],
    limit: Duration,
) -> ViewersUpdate {
    let deadline = Instant::now() + limit;
    let mut last: Option<ViewersUpdate> = None;
    while Instant::now() < deadline {
        let left = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, updates.recv()).await {
            Ok(Some(update)) => {
                if wanted
                    .iter()
                    .all(|ip| update.active.iter().any(|v| v.ip == *ip))
                {
                    return update;
                }
                last = Some(update);
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }
    panic!(
        "not everyone was in the list within {limit:?}. Wanted: {wanted:?}. Last seen: {:?}",
        last.map(|u| u.active.iter().map(|v| v.ip.clone()).collect::<Vec<_>>())
    );
}

fn find<'a>(update: &'a ViewersUpdate, ip: &str) -> &'a KnownViewer {
    update
        .active
        .iter()
        .find(|v| v.ip == ip)
        .unwrap_or_else(|| panic!("{ip} is not in the list: {:?}", update.active))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_following_of_the_log_is_running_before_the_watching_is_declared_started() {
    // The narrowest of the checks here, and it exists because of a fault it caught. Asking
    // the server to run something comes back before the command has started; `tail -n 0`
    // starts at the end of the file, so a request served inside that gap is missed. For a
    // directly served film that request is the only one there will be until the showing
    // ends — the viewer appeared watching an unknown something and stayed that way.
    let server = TestServer::start().expect("the container would not come up");
    let film =
        lay_out_direct_file(&server, "film.mp4", 100_000).expect("the file was not laid out");
    let conn = connect(&server).await;

    let (tx, mut updates) = mpsc::channel(64);
    let watch = viewers::start(
        conn,
        String::from("server-1"),
        Arc::new(Library),
        TimeDuration::seconds(30),
        tx,
    )
    .await
    .expect("the watching would not start");

    // The very next thing after the watching says it has started. If the following were
    // still coming up, this request would fall into the gap.
    let viewer = Viewer::attach(&server).expect("the viewer would not attach");
    viewer.probe(&film).expect("the viewer got nothing");

    let update = wait_for(&mut updates, &[viewer.ip()], Duration::from_secs(15)).await;
    assert_eq!(
        find(&update, viewer.ip()).media_id.as_deref(),
        Some("media-film"),
        "the request made immediately after the watching started was missed. \
         What the server recorded:\n{}",
        server.access_log().unwrap_or_default()
    );

    drop(watch);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn both_ways_of_serving_put_their_viewer_in_the_list() {
    let server = TestServer::start().expect("the container would not come up");
    // Large enough that the pulling does not finish while the check is looking: a viewer
    // who has already left says nothing about whether they would have been seen.
    let film =
        lay_out_direct_file(&server, "film.mp4", 40_000_000).expect("the file was not laid out");
    lay_out_ladder(&server, "demo").expect("the quality set was not laid out");

    let conn = connect(&server).await;
    let (tx, mut updates) = mpsc::channel(64);

    // The watching starts first. Following the log begins at the end of the file, so
    // anything asked for before this moment is over and done with as far as it knows —
    // which is right, and means the requests have to come after.
    // A deliberately short threshold, and the reason is the whole point of this check. The
    // viewer of the single file makes one request that finishes — the player asking what is
    // there — and that alone would put them in the list for as long as the threshold lasts.
    // A check that looked before it expired would pass with the connection table never
    // polled at all, which is precisely the half of the mechanism it exists to guard.
    // Past the threshold, only the polling can keep them there.
    let threshold = TimeDuration::seconds(5);
    let watch = viewers::start(
        conn,
        String::from("server-1"),
        Arc::new(Library),
        threshold,
        tx,
    )
    .await
    .expect("the watching would not start");

    let direct = Viewer::attach(&server).expect("the first viewer would not attach");
    let set = Viewer::attach(&server).expect("the second viewer would not attach");

    // A player asks what is there before it settles down to pull. For the one watching a
    // single file that first request is the only thing that will finish for the whole
    // showing, and it is what says which film it is.
    direct.probe(&film).expect("the first viewer got nothing");
    direct
        .start_watching(&film, Some("300k"))
        .expect("the pulling would not start");
    set.start_watching_a_set("demo", "v2", 3)
        .expect("the watching of the set would not start");

    // Both are found within the ten seconds SC-005 allows...
    wait_for(
        &mut updates,
        &[direct.ip(), set.ip()],
        Duration::from_secs(10),
    )
    .await;

    // ...and both are still there once the request that announced them has gone stale. From
    // here on the viewer of the single file is held in the list by the connection table and
    // by nothing else.
    tokio::time::sleep(Duration::from_secs(threshold.whole_seconds() as u64 + 3)).await;
    // Everything that piled up while we waited is thrown away first. Without this the next
    // read would be an update from before the request went stale — and the check would pass
    // with the connection table never consulted at all, which is exactly what it is for.
    // Found by breaking the polling on purpose and watching this pass anyway.
    while updates.try_recv().is_ok() {}
    let update = wait_for(
        &mut updates,
        &[direct.ip(), set.ip()],
        Duration::from_secs(10),
    )
    .await;

    // The one watching a single file. Seen only because the connection table is polled —
    // their one request will not be recorded until the showing ends.
    let one = find(&update, direct.ip());
    assert_eq!(
        one.media_id.as_deref(),
        Some("media-film"),
        "the viewer of the single file is watching the wrong thing: {one:?}\n\
         What the server actually recorded:\n{}",
        server.access_log().unwrap_or_default()
    );

    // The one watching a quality set, down to which rung.
    let other = find(&update, set.ip());
    assert_eq!(
        other.media_id.as_deref(),
        Some("media-demo"),
        "the viewer of the set is watching the wrong thing: {other:?}"
    );
    assert_eq!(
        other.variant.as_deref(),
        Some("v2"),
        "the rung being received was not worked out: {other:?}"
    );

    // And the count that goes into the medium's card (FR-056).
    assert_eq!(update.per_media.get("media-demo"), Some(&1));
    assert_eq!(update.per_media.get("media-film"), Some(&1));

    drop(watch);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_delivered_speed_is_measured_against_a_real_server() {
    // What this adds to the unit tests is the one thing they cannot have: real `ss` output,
    // twice, with real time between. The figure comes from the growth of what the far end
    // has confirmed — the field `ss` calls delivery_rate reported 19 Gbit/s for a viewer
    // held to 200 kB/s, and taking that would have marked the slowest viewer as the
    // fastest.
    //
    // How narrow a link is judged is checked in the unit tests instead: this kernel offers
    // no traffic shaping at all (no netem, no tbf — measured on 2026-08-26), so a genuinely
    // narrow link cannot be made here, and pretending one with a rate-limited puller would
    // be checking a player with a full buffer, which is the case that must NOT be marked.
    let server = TestServer::start().expect("the container would not come up");
    let film =
        lay_out_direct_file(&server, "film.mp4", 40_000_000).expect("the file was not laid out");

    let conn = connect(&server).await;
    let (tx, mut updates) = mpsc::channel(64);
    let watch = viewers::start(
        conn,
        String::from("server-1"),
        Arc::new(Library),
        TimeDuration::seconds(30),
        tx,
    )
    .await
    .expect("the watching would not start");

    let viewer = Viewer::attach(&server).expect("the viewer would not attach");
    viewer.probe(&film).expect("the viewer got nothing");
    viewer
        .start_watching(&film, Some("500k"))
        .expect("the pulling would not start");

    // Long enough for the window to have something in it: no speed is shown before there
    // is enough to work one out from, and that restraint is deliberate.
    let deadline = Instant::now() + Duration::from_secs(25);
    let mut measured = None;
    while Instant::now() < deadline {
        let left = deadline.saturating_duration_since(Instant::now());
        let Ok(Some(update)) = tokio::time::timeout(left, updates.recv()).await else {
            break;
        };
        if let Some(speed) = update
            .active
            .iter()
            .find(|v| v.ip == viewer.ip())
            .and_then(|v| v.delivery_bps)
        {
            measured = Some(speed);
            break;
        }
    }

    let speed =
        measured.expect("no speed was ever worked out for a viewer who was plainly pulling");
    // Half a megabyte a second is four megabits. The bounds are wide on purpose — what is
    // being checked is that the figure is a measurement of this viewer and not a number
    // from somewhere else. Nineteen gigabits would land far outside them.
    assert!(
        (1_000_000..20_000_000).contains(&speed),
        "the delivered speed came out as {speed} bit/s for a viewer held to about 4 Mbit/s"
    );

    drop(watch);
}
