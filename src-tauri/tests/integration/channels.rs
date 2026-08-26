//! T154 — the spending of channels, counted together.
//!
//! What is checked here is the thing R-04 warned about and the layer had no answer to until
//! T153: from Phase 4 onwards two channels are held for as long as a session lasts —
//! following the access log and polling the connection table — and they have to coexist
//! with a band of tasks inside a limit of eight.
//!
//! What is NOT checked here is that many channels queue inside one connection rather than
//! being refused: that stands in `ssh_live.rs`, and repeating it would only make a failure
//! appear in two places at once.

use std::time::Duration;

use super::fixture::{key_path, TestServer, KEY_PASSPHRASE};
use vrcast_studio_lib::ssh::connection::{BRIEF_CHANNELS, STANDING_CHANNELS};
use vrcast_studio_lib::ssh::{fingerprint, Connection, Credentials, ServerAddress};

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

#[tokio::test]
async fn standing_channels_do_not_hold_up_ordinary_work() {
    let server = TestServer::start().expect("the container would not come up");
    let conn = connect(&server).await;

    // Both standing places taken, as they will be during a session: one for following the
    // log, one for polling the connections (R-02).
    let watching = conn
        .reserve_standing_channel()
        .await
        .expect("the place for following the log was not given");
    let polling = conn
        .reserve_standing_channel()
        .await
        .expect("the place for polling the connections was not given");

    assert_eq!(
        conn.standing_channels_in_use(),
        STANDING_CHANNELS,
        "both places set aside are taken, and the count does not show it"
    );
    assert_eq!(
        conn.brief_channels_in_use(),
        0,
        "the standing users ate places meant for ordinary work"
    );

    // A band of tasks alongside: the arithmetic written down in R-04 — one preparation, one
    // transfer and four light ones. All at once, so that they really compete for places.
    let mut band = Vec::new();
    for i in 0..BRIEF_CHANNELS {
        let c = conn.clone();
        band.push(tokio::spawn(async move {
            c.exec(&format!("echo band-{i}")).await
        }));
    }

    // Every one of them must get through. Waiting is allowed; a refusal is not — that is
    // the whole difference between our own limit and the server's.
    for (i, task) in band.into_iter().enumerate() {
        let out = tokio::time::timeout(Duration::from_secs(30), task)
            .await
            .unwrap_or_else(|_| panic!("piece {i} of the band never finished: the places ran out"))
            .expect("the task fell over")
            .expect("the channel did not work");
        assert!(
            out.ok() && out.stdout.contains(&format!("band-{i}")),
            "piece {i} of the band ended unsuccessfully: {out:?}"
        );
    }

    // And the watching is still in place after all of it: the band did not push it out.
    assert_eq!(
        conn.standing_channels_in_use(),
        STANDING_CHANNELS,
        "the band of tasks displaced the watching of viewers"
    );

    // The places come back by themselves when what holds them is dropped. Giving them back
    // by hand would sooner or later be forgotten in one of the ways watching stops.
    drop(watching);
    drop(polling);
    assert_eq!(
        conn.standing_channels_in_use(),
        0,
        "a place set aside was not given back when the watching stopped"
    );
}

#[tokio::test]
async fn ordinary_work_cannot_leave_the_watching_unable_to_start() {
    let server = TestServer::start().expect("the container would not come up");
    let conn = connect(&server).await;

    // The band goes first and deliberately asks for more than there is room for: with one
    // shared pool it would take every place, and the watching of viewers would then never
    // start — the screen would stay empty with nothing to say why. This is the failure T153
    // exists to make impossible.
    let mut band = Vec::new();
    for i in 0..(BRIEF_CHANNELS * 2) {
        let c = conn.clone();
        band.push(tokio::spawn(async move {
            c.exec(&format!("sleep 1; echo crowd-{i}")).await
        }));
    }

    // While the band is competing for places, the watching asks for its own. It must get it
    // without waiting for the queue to clear.
    let reserved = tokio::time::timeout(Duration::from_secs(5), conn.reserve_standing_channel())
        .await
        .expect("the watching waited for a place while the band of tasks ran — the places are not set aside")
        .expect("the place for the watching was not given");

    assert_eq!(conn.standing_channels_in_use(), 1);

    for task in band {
        let out = task
            .await
            .expect("the task fell over")
            .expect("the channel did not work");
        assert!(out.ok(), "a piece of the band ended unsuccessfully");
    }
    drop(reserved);

    // One connection to the server throughout, not one per channel. Counted by the server's
    // own means: counting by ours would only show that our code agrees with itself.
    let established = server
        .exec_inside("ss -tn state established '( sport = :22 )' | tail -n +2 | wc -l")
        .expect("could not count the connections by the server's own means");
    let count: usize = established
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("ss's output would not parse: \"{established}\""));
    assert!(
        (1..=2).contains(&count),
        "the server sees {count} connections instead of one — multiplexing does not work"
    );
}
