//! T538 — `server:state` was declared (`commands::events::names::SERVER_STATE`) and never
//! sent: `AppEvent` had no variant for it. This proves the wiring the other way round —
//! that saying a server's state now actually puts an `AppEvent::ServerState` on the channel
//! the interface listens to, shaped the way the contract will describe it.
//!
//! What is checked here is the plumbing (`AppState::notify_server_state` → `state.events` →
//! a subscriber), the same thing `LibraryChanged`'s own `notify_library_changed` would be
//! checked by if a test for it existed — none does, so this is written against the pattern
//! `stand_scenarios.rs` uses for `ViewersUpdate` instead: subscribe first, then act, then
//! read the channel.
//!
//! What this does NOT reach: `deploy::api::server_detect` actually calling
//! `notify_server_state` on a real "at connection" read needs a real server and lives in the
//! integration suite (`tests/integration/detect_live.rs`) behind Docker, the same boundary
//! every other server-reaching check in this project respects.

use std::sync::Arc;

use vrcast_studio_lib::commands::{AppEvent, AppState};
use vrcast_studio_lib::domain::server_state::{unreachable, Kind};
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;

fn state() -> AppState {
    AppState::with_db(
        Arc::new(Db::open_in_memory().unwrap()),
        Arc::new(InMemorySecretStore::new()),
    )
    .expect("the application state would not assemble")
}

#[tokio::test]
async fn saying_a_server_s_state_puts_it_on_the_channel() {
    let s = state();
    // Subscribed before anything happens, the same way `stand_scenarios.rs` subscribes
    // before making a request: `ViewersUpdate` only ever arrives as an event, and so does
    // this one — there is no command to ask for the last one sent.
    let mut rx = s.subscribe();

    let sent = unreachable();
    s.notify_server_state("srv-1", sent.clone());

    let event = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
        .await
        .expect("the channel produced nothing at all within a second")
        .expect("the channel closed instead of delivering the event");

    match event {
        AppEvent::ServerState { server_id, state } => {
            assert_eq!(server_id, "srv-1");
            assert_eq!(
                state, sent,
                "the state that arrived is not the one that was sent"
            );
            assert_eq!(state.kind, Kind::Unreachable);
        }
        other => panic!("expected AppEvent::ServerState, got {other:?}"),
    }
}

#[tokio::test]
async fn nothing_arrives_when_nobody_says_a_server_s_state() {
    // The other half: a subscriber that outlives no call to `notify_server_state` gets
    // nothing at all. Without this, a test that only checked the happy path could not tell
    // "the event fires" from "the channel always has something on it".
    let s = state();
    let mut rx = s.subscribe();

    let empty = tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await;
    assert!(
        empty.is_err(),
        "an event arrived on the channel although nothing was ever sent"
    );
}
