//! T569 — `AppState.geo_fetch` actually keeps two `geo::fetch` calls from overlapping.
//!
//! This checks the wiring, not the network: no real request goes anywhere here (that would
//! be `tests/integration/geo_real.rs`'s job, and it is `#[ignore]`d for the usual reasons —
//! slow, hits a real third party, unwelcome in an ordinary run). What matters is provable
//! without a byte of traffic: the field exists, `AppState::with_db` gives every instance
//! one, clones of that `AppState` share the same lock rather than each getting their own,
//! and a held lock actually blocks a second attempt to take it.

use std::sync::Arc;

use vrcast_studio_lib::commands::AppState;
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
async fn a_held_lock_blocks_a_second_attempt_and_releases_on_drop() {
    let s = state();

    let guard = s.geo_fetch.lock().await;
    assert!(
        s.geo_fetch.try_lock().is_err(),
        "a second attempt at the lock succeeded while the first still held it — \
         two geo::fetch calls could run at once and corrupt the same temp files"
    );

    drop(guard);
    assert!(
        s.geo_fetch.try_lock().is_ok(),
        "the lock stayed held after its guard was dropped"
    );
}

#[tokio::test]
async fn a_clone_of_app_state_shares_the_same_lock() {
    // `AppState` is `#[derive(Clone)]` and handed out freely (T171's `viewers`, T162's
    // `places` already rely on this). If `geo_fetch` were not `Arc`'d, a clone taken before
    // this test's lock would get its own independent `Mutex<()>` — wired but useless, the
    // exact shape of bug this test exists to catch before it ships.
    let s = state();
    let clone = s.clone();

    let guard = s.geo_fetch.lock().await;
    assert!(
        clone.geo_fetch.try_lock().is_err(),
        "a clone of AppState could take the lock while the original held it — \
         geo_fetch is not actually shared between clones"
    );
    drop(guard);
    assert!(clone.geo_fetch.try_lock().is_ok());
}

#[tokio::test]
async fn a_waiting_lock_attempt_proceeds_once_the_holder_drops_it() {
    // The shape the real callers actually use: one task holds the lock for a while, a
    // second is already waiting on `.lock().await` before the first releases it, and only
    // then does the second proceed — proving the mutex actually serializes rather than
    // merely existing unused.
    let s = state();
    let s2 = s.clone();

    let order = Arc::new(tokio::sync::Mutex::new(Vec::<&'static str>::new()));
    let order_a = Arc::clone(&order);
    let order_b = Arc::clone(&order);

    let first = tokio::spawn(async move {
        let _guard = s.geo_fetch.lock().await;
        order_a.lock().await.push("first-acquired");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        order_a.lock().await.push("first-released");
    });

    // Give `first` a moment to be the one to grab the lock before `second` tries.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    let second = tokio::spawn(async move {
        let _guard = s2.geo_fetch.lock().await;
        order_b.lock().await.push("second-acquired");
    });

    first.await.expect("the first task panicked");
    second.await.expect("the second task panicked");

    let seen = order.lock().await.clone();
    assert_eq!(
        seen,
        vec!["first-acquired", "first-released", "second-acquired"],
        "the second task's lock was granted before the first released it"
    );
}
