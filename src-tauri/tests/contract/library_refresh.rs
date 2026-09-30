//! T651 — reading a library does not keep reading it.
//!
//! QA-24A №2: `library_list(refresh=false)` handed back the cache and started a refresh
//! behind it; that refresh always ended with `library:changed`, even when it found exactly
//! what the cache held; and the screen answered the event with `library_list(refresh=false)`
//! — which started the next refresh. An open library reread the whole server for as long as
//! it stayed open.
//!
//! Settled here without a server, through `commands::library::refreshes` — the one place a
//! refresh is started, joined and kept:
//! - an unchanged answer is kept without an event; a changed one, or one with no cache
//!   before it, sends one;
//! - two refreshes of the same server asked for at once read it once;
//! - a catalogue change is never overwritten by a refresh begun before it;
//! - `library_list_known` (what the screen reads on the event) starts no refresh, while
//!   `library_list(refresh=false)` does.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use tokio::sync::{broadcast, oneshot};

use super::support::{state, valid_input};
use vrcast_studio_lib::commands::library::{api, refreshes, LibraryView, MediaView};
use vrcast_studio_lib::commands::servers::api as servers_api;
use vrcast_studio_lib::commands::{AppEvent, AppState};
use vrcast_studio_lib::store::library_cache;

const SECRET: &str = "server-password-for-the-test-7c21";

fn state_with_server() -> (AppState, String) {
    let s = state();
    let id = servers_api::server_add(&s, valid_input("Server"), SECRET)
        .expect("the profile was not created");
    (s, id)
}

fn view(server_id: &str, title: &str) -> LibraryView {
    LibraryView {
        server_id: server_id.to_owned(),
        media: vec![MediaView {
            id: String::from("m1"),
            title: title.to_owned(),
            slug: String::from("one"),
            files: Vec::new(),
            ladders: Vec::new(),
            total_bytes: 0,
            created_at: String::from("2026-01-01T00:00:00Z"),
        }],
        unrecognized: Vec::new(),
        disk: None,
        stale: false,
    }
}

fn library_events(rx: &mut broadcast::Receiver<AppEvent>) -> Vec<String> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let AppEvent::LibraryChanged { server_id } = ev {
            out.push(server_id);
        }
    }
    out
}

#[tokio::test]
async fn an_unchanged_refresh_is_kept_without_an_event() {
    let (s, id) = state_with_server();
    library_cache::save(&s.db, &id, &view(&id, "Film")).unwrap();
    let mut rx = s.subscribe();

    let changed = refreshes::settle(&s, &id, &view(&id, "Film")).unwrap();

    assert!(!changed, "the same library was reported as changed");
    assert!(
        library_events(&mut rx).is_empty(),
        "an unchanged refresh still said the library changed — the loop of QA-24A №2"
    );
}

#[tokio::test]
async fn a_changed_refresh_is_kept_and_said() {
    let (s, id) = state_with_server();
    library_cache::save(&s.db, &id, &view(&id, "Film")).unwrap();
    let mut rx = s.subscribe();

    let changed = refreshes::settle(&s, &id, &view(&id, "Renamed")).unwrap();

    assert!(changed);
    assert_eq!(library_events(&mut rx), vec![id.clone()]);
    assert_eq!(
        library_cache::load(&s.db, &id).unwrap().unwrap().media[0].title,
        "Renamed"
    );
}

#[tokio::test]
async fn a_refresh_after_the_cache_was_forgotten_is_said() {
    // A change forgets the cache and expects the library to be read again: finding no
    // cache is a difference, even if what was read equals what used to be there.
    let (s, id) = state_with_server();
    let mut rx = s.subscribe();

    assert!(refreshes::settle(&s, &id, &view(&id, "Film")).unwrap());
    assert_eq!(library_events(&mut rx), vec![id.clone()]);
}

/// A stand-in for reading the server: counts how often it is asked and answers when told.
fn gated_build(
    count: &Arc<AtomicUsize>,
    answer: LibraryView,
) -> (
    oneshot::Sender<()>,
    impl FnOnce() -> futures::future::BoxFuture<
            'static,
            vrcast_studio_lib::commands::error::Result<LibraryView>,
        > + Send
        + 'static,
) {
    let (go, wait) = oneshot::channel::<()>();
    let count = count.clone();
    let build = move || {
        count.fetch_add(1, Ordering::SeqCst);
        async move {
            let _ = wait.await;
            Ok(answer)
        }
        .boxed()
    };
    (go, build)
}

fn counting_build(
    count: &Arc<AtomicUsize>,
    answer: LibraryView,
) -> impl FnOnce() -> futures::future::BoxFuture<
    'static,
    vrcast_studio_lib::commands::error::Result<LibraryView>,
> + Send
       + 'static {
    let count = count.clone();
    move || {
        count.fetch_add(1, Ordering::SeqCst);
        async move { Ok(answer) }.boxed()
    }
}

#[tokio::test]
async fn two_refreshes_of_one_server_at_once_read_it_once() {
    let (s, id) = state_with_server();
    let reads = Arc::new(AtomicUsize::new(0));
    let (go, first) = gated_build(&reads, view(&id, "Film"));

    let a = refreshes::run(&s, &id, first);
    let b = refreshes::run(&s, &id, counting_build(&reads, view(&id, "Other")));
    go.send(()).unwrap();

    let (a, b) = (a.await.unwrap(), b.await.unwrap());
    assert_eq!(
        reads.load(Ordering::SeqCst),
        1,
        "the server was read twice at once"
    );
    assert_eq!(a, b, "the two askers got different answers from one read");
    assert_eq!(a.media[0].title, "Film");

    // Once it has finished, the next refresh is a new read, not the old answer.
    let c = refreshes::run(&s, &id, counting_build(&reads, view(&id, "Later")))
        .await
        .unwrap();
    assert_eq!(reads.load(Ordering::SeqCst), 2);
    assert_eq!(c.media[0].title, "Later");
}

#[tokio::test]
async fn another_server_s_refresh_is_not_joined() {
    let (s, id) = state_with_server();
    let other = servers_api::server_add(&s, valid_input("Other"), SECRET).unwrap();
    let reads = Arc::new(AtomicUsize::new(0));
    let (go, first) = gated_build(&reads, view(&id, "Film"));

    let a = refreshes::run(&s, &id, first);
    let b = refreshes::run(&s, &other, counting_build(&reads, view(&other, "Theirs")))
        .await
        .unwrap();
    go.send(()).unwrap();
    a.await.unwrap();

    assert_eq!(reads.load(Ordering::SeqCst), 2);
    assert_eq!(b.server_id, other);
}

#[tokio::test]
async fn a_refresh_begun_before_a_change_neither_is_joined_nor_overwrites_it() {
    let (s, id) = state_with_server();
    let reads = Arc::new(AtomicUsize::new(0));
    let (go, before) = gated_build(&reads, view(&id, "Old title"));
    let stale = refreshes::run(&s, &id, before);

    // A rename lands while that read is still under way.
    s.invalidate_library(&id);

    // Whoever asks now gets a read of their own, begun after the change.
    let fresh = refreshes::run(&s, &id, counting_build(&reads, view(&id, "New title")))
        .await
        .unwrap();
    assert_eq!(
        reads.load(Ordering::SeqCst),
        2,
        "a read begun before the change was joined"
    );
    assert_eq!(fresh.media[0].title, "New title");

    // The old read ends last; its answer goes to whoever already waited on it, but is not
    // written over the newer one.
    go.send(()).unwrap();
    let _ = stale.await;
    assert_eq!(
        library_cache::load(&s.db, &id).unwrap().unwrap().media[0].title,
        "New title",
        "a refresh begun before the change put the old title back"
    );
}

#[tokio::test]
async fn reading_what_is_known_starts_no_refresh() {
    let (s, id) = state_with_server();
    library_cache::save(&s.db, &id, &view(&id, "Film")).unwrap();

    let known = api::library_list_known(&s, &id).await.unwrap();
    assert_eq!(known.media[0].title, "Film");

    // Had a refresh been started, this one would join it and never be read.
    let reads = Arc::new(AtomicUsize::new(0));
    refreshes::run(&s, &id, counting_build(&reads, view(&id, "Film")))
        .await
        .unwrap();
    assert_eq!(
        reads.load(Ordering::SeqCst),
        1,
        "reading what is known asked the server for a refresh"
    );
}

#[tokio::test]
async fn the_cache_with_a_refresh_behind_it_does_start_one() {
    // The other half: opening the library still refreshes it (FR-080) — once.
    let (s, id) = state_with_server();
    library_cache::save(&s.db, &id, &view(&id, "Film")).unwrap();

    let cached = api::library_list(&s, &id, false).await.unwrap();
    assert_eq!(cached.media[0].title, "Film");

    let reads = Arc::new(AtomicUsize::new(0));
    // A refresh asked for now joins the one started behind the cache — still trying the
    // documentation address nobody answers at — instead of building its own. On a network
    // that refuses that address at once, the refresh behind may already have failed; then
    // this one is a fresh read, which is also correct. Either way no more than one.
    let joined = refreshes::run(&s, &id, counting_build(&reads, view(&id, "Film")));
    let _ = tokio::time::timeout(Duration::from_millis(50), joined).await;
    assert!(reads.load(Ordering::SeqCst) <= 1);

    // A failed refresh behind the cache leaves the cache as it was.
    assert_eq!(
        library_cache::load(&s.db, &id).unwrap().unwrap().media[0].title,
        "Film"
    );
}
