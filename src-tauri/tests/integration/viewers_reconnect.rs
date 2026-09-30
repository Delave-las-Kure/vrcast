//! T664 — the watching of viewers comes back after the connection to the server is cut.
//!
//! ⚠ **QA-24B-05.** After a break in the connection the watching used to stay "running" for
//! good: the reader of the log ended with nothing to follow it, the poll kept failing through
//! the same dead connection, a repeated start answered "already watching", and the screen
//! showed the last list as if it were now. What is checked here, against a real container
//! whose network is really cut (`docker network disconnect`, the same fault T560 and T570
//! use):
//!
//! 1. the watching says it has lost the connection — an update marked `reconnecting`, still
//!    carrying the last list and when it was current, within a bounded time of the cut;
//! 2. once the network is back, it comes back **by itself** — nobody calls start again —
//!    and a viewer who starts watching after that appears in the list, which can only
//!    happen if both sources (the log and the connection table) were opened again.

use std::sync::Arc;
use std::time::{Duration, Instant};

use time::Duration as TimeDuration;
use tokio::sync::mpsc;

use super::fixture::{key_path, TestServer, KEY_PASSPHRASE, SERVER_ALIAS};
use super::hls_fixture::lay_out_direct_file;
use super::viewer::Viewer;
use vrcast_studio_lib::domain::access_log::Asked;
use vrcast_studio_lib::domain::geo::Place;
use vrcast_studio_lib::domain::viewers::{VariantFacts, WatchState};
use vrcast_studio_lib::server::viewers::{self, Reconnect, Retry, ViewerContext, ViewersUpdate};
use vrcast_studio_lib::ssh::{fingerprint, Connection, Credentials, ServerAddress};

struct Library;

impl ViewerContext for Library {
    fn facts(&self, asked: &Asked) -> VariantFacts {
        match asked.library_key() {
            Some("film.mp4") => VariantFacts {
                media_id: Some(String::from("media-film")),
                variant: Some(String::from("film.mp4")),
                required_bps: None,
            },
            _ => VariantFacts::default(),
        }
    }

    fn place(&self, _ip: &str) -> Place {
        Place::default()
    }
}

async fn connect(addr: ServerAddress, fp: String) -> Result<Connection, Retry> {
    Connection::connect(
        addr,
        "root",
        Credentials::Key {
            path: key_path(),
            passphrase: Some(KEY_PASSPHRASE.to_owned()),
        },
        &fp,
    )
    .await
    .map_err(|e| Retry::Transient(e.to_string()))
}

fn docker_network(args: &[&str]) {
    let out = std::process::Command::new("docker").args(args).output();
    assert!(
        matches!(&out, Ok(o) if o.status.success()),
        "docker {args:?} failed — the fixture itself is broken: {out:?}"
    );
}

/// Wait for an update that satisfies `wanted`, or panic with the last one seen.
async fn wait_until(
    updates: &mut mpsc::Receiver<ViewersUpdate>,
    limit: Duration,
    what: &str,
    wanted: impl Fn(&ViewersUpdate) -> bool,
) -> ViewersUpdate {
    let deadline = Instant::now() + limit;
    let mut last = None;
    while Instant::now() < deadline {
        let left = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, updates.recv()).await {
            Ok(Some(update)) => {
                if wanted(&update) {
                    return update;
                }
                last = Some(update);
            }
            Ok(None) => panic!("the watching ended while waiting for: {what}"),
            Err(_) => break,
        }
    }
    panic!(
        "not seen within {limit:?}: {what}. Last update: {:?}",
        last.map(|u| (u.watch, u.attempt, u.as_of, u.active.len()))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_watching_comes_back_by_itself_after_the_network_is_cut() {
    super::fixture::logging_if_requested();
    let server = TestServer::start().expect("the container would not come up");
    let film =
        lay_out_direct_file(&server, "film.mp4", 100_000).expect("the file was not laid out");

    let addr = ServerAddress::new(server.host(), server.port);
    let fp = fingerprint::probe(&addr)
        .await
        .expect("the fingerprint was not obtained");
    let first = connect(addr.clone(), fp.clone())
        .await
        .expect("connecting failed");

    let tries = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let reconnect: Reconnect = {
        let tries = tries.clone();
        Arc::new(move || {
            tries.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Box::pin(connect(addr.clone(), fp.clone()))
        })
    };

    let (tx, mut updates) = mpsc::channel(256);
    let watch = viewers::start_reconnecting(
        first,
        Some(reconnect),
        String::from("server-1"),
        Arc::new(Library),
        TimeDuration::seconds(30),
        tx,
    )
    .await
    .expect("the watching would not start");

    // It is watching, and a list has come in.
    let before = wait_until(
        &mut updates,
        Duration::from_secs(15),
        "a first current list",
        |u| u.watch == WatchState::Watching && u.as_of.is_some(),
    )
    .await;

    // --- the cut ---------------------------------------------------------------------
    docker_network(&[
        "network",
        "disconnect",
        "-f",
        server.network(),
        server.container_id(),
    ]);
    let cut_at = Instant::now();

    // 1. It says so, and does not pretend the old list is current. Bounded well below the
    //    90–120 s the SSH keepalive alone would take: the poll has its own limit.
    let lost = wait_until(
        &mut updates,
        Duration::from_secs(60),
        "an update saying the connection was lost",
        |u| u.watch == WatchState::Reconnecting,
    )
    .await;
    println!("the loss was noticed {:?} after the cut", cut_at.elapsed());
    assert!(
        lost.as_of.is_some() && lost.as_of >= before.as_of,
        "the list shown while reconnecting does not say how old it is: {lost:?}"
    );
    assert!(
        watch.is_alive(),
        "a watch getting its connection back counts as dead, and a repeated start would \
         throw it away"
    );

    // Long enough for at least one try to fail against the cut network.
    tokio::time::sleep(Duration::from_secs(10)).await;

    // --- the network comes back, under the name the viewers ask for it by ----------------
    docker_network(&[
        "network",
        "connect",
        "--alias",
        SERVER_ALIAS,
        server.network(),
        server.container_id(),
    ]);
    let back_at = Instant::now();

    // 2. Back by itself — nobody starts it again.
    let back = wait_until(
        &mut updates,
        Duration::from_secs(120),
        "the watching back and current after the network returned",
        |u| u.watch == WatchState::Watching && u.as_of > lost.as_of,
    )
    .await;
    println!(
        "the watching was back {:?} after the network returned, after {} tries",
        back_at.elapsed(),
        tries.load(std::sync::atomic::Ordering::SeqCst)
    );
    assert!(
        tries.load(std::sync::atomic::Ordering::SeqCst) >= 1,
        "no new connection was opened — the watching cannot have come back through one"
    );
    assert_eq!(back.attempt, 0, "a current list still counts tries");

    // Both sources are really back: a viewer who starts only now is seen, and seen watching
    // the film — which only the log can say.
    let viewer = Viewer::attach(&server).expect("the viewer would not attach");
    viewer.probe(&film).expect("the viewer got nothing");
    let seen = wait_until(
        &mut updates,
        Duration::from_secs(20),
        "the new viewer in the list, watching the film",
        |u| {
            u.watch == WatchState::Watching
                && u.active
                    .iter()
                    .any(|v| v.ip == viewer.ip() && v.media_id.as_deref() == Some("media-film"))
        },
    )
    .await;
    assert_eq!(seen.per_media.get("media-film"), Some(&1));

    drop(watch);
}
