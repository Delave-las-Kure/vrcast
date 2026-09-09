//! T162 — the tables of places, against the real ones.
//!
//! Ignored by default: it downloads about seventy megabytes from DB-IP, which has no place
//! in an ordinary run or in continuous integration. To run it:
//!
//! ```text
//! cargo test --features integration --test integration -- --ignored --nocapture the_real_tables
//! ```
//!
//! **Why it exists at all.** Everything else about placing an address is checked on rules
//! and on an empty table; nothing checks that the fields this code reaches for are the
//! fields DB-IP actually writes. Getting a path wrong there fails in the quietest way
//! there is — every viewer comes back "not determined", exactly as if the table were
//! missing, and nothing in the application looks broken.

use std::time::Instant;

use vrcast_studio_lib::store::geo::{self, Places};

#[tokio::test]
#[ignore = "downloads about seventy megabytes from DB-IP"]
async fn the_real_tables_answer_for_real_addresses() {
    let dir = std::env::temp_dir().join("vrcast-geo-real");
    let now = time::OffsetDateTime::now_utc();

    if geo::needs_fetching(&dir, &geo::month_name(now.year(), now.month() as u8)) {
        let started = Instant::now();
        let month = geo::fetch(&dir, now.year(), now.month() as u8)
            .await
            .expect("the tables would not download");
        println!("took the tables for {month} in {:?}", started.elapsed());
    }

    let places = Places::open(&dir);
    assert!(
        !places.is_empty(),
        "the tables downloaded but would not open"
    );

    // A well-known address that every table in the world has an answer for. What is checked
    // is that *something* comes back, not what: the free tables differ month to month, and
    // asserting a particular city would make this fail for a reason that is nobody's fault.
    let known = places.look_up("8.8.8.8");
    println!("8.8.8.8 -> {known:?}");
    assert!(
        known.country.is_some(),
        "no country came back for a public address — the path into the table is wrong, \
         and every viewer would silently read as \"not determined\""
    );
    assert!(
        known.asn_org.is_some(),
        "no provider came back — the provider table is read down the wrong path"
    );

    // IPv6, which is a separate tree in the same file and a separate chance to be wrong.
    let six = places.look_up("2a00:1450:4001:800::200e");
    println!("2a00:1450:… -> {six:?}");
    assert!(six.country.is_some(), "IPv6 is not answered for");

    // And the rule that matters most: an address nobody can speak for is not answered for,
    // even though the tables do hold rows covering it.
    for ip in ["127.0.0.1", "10.0.0.9", "192.168.1.1", "::1"] {
        assert_eq!(
            places.look_up(ip),
            Default::default(),
            "{ip} was placed out of the table's reserved rows"
        );
    }
}

/// T569 — two callers racing `geo::fetch` against the same directory must not corrupt the
/// tables, and the one that had to wait for the lock must not fetch a second time once the
/// first has already brought the wanted month in.
///
/// This does not call `commands::geo::api::geo_update` / `refresh_in_background` directly:
/// both read `geo::dir()`, which is hard-wired to the real profile directory rather than
/// taking one as a parameter, so there is no way to point either at a scratch directory
/// without downloading into the developer's own profile. What is reproduced here instead is
/// their exact shape against a directory this test controls — `state.geo_fetch.lock()`, then
/// the same `month_on_disk == wanted` recheck, then `geo::fetch` — which is the whole of
/// what T569 added and the only part a real network call can actually prove.
#[tokio::test]
#[ignore = "downloads about seventy megabytes from DB-IP, twice, on purpose"]
async fn two_real_fetches_racing_the_same_lock_do_not_corrupt_the_tables() {
    use std::sync::Arc;

    use vrcast_studio_lib::commands::AppState;
    use vrcast_studio_lib::store::db::Db;
    use vrcast_studio_lib::store::secrets::InMemorySecretStore;

    let dir = std::env::temp_dir().join("vrcast-geo-real-race");
    let _ = std::fs::remove_dir_all(&dir);

    let state = AppState::with_db(
        Arc::new(Db::open_in_memory().unwrap()),
        Arc::new(InMemorySecretStore::new()),
    )
    .expect("the application state would not assemble");

    let now = time::OffsetDateTime::now_utc();
    let (year, month) = (now.year(), now.month() as u8);
    let wanted = geo::month_name(year, month);

    async fn guarded_fetch(
        state: &AppState,
        dir: &std::path::Path,
        year: i32,
        month: u8,
        wanted: &str,
    ) -> Option<String> {
        let _guard = state.geo_fetch.lock().await;
        if geo::month_on_disk(dir).as_deref() == Some(wanted) {
            return None;
        }
        geo::fetch(dir, year, month).await.ok()
    }

    let started = Instant::now();
    let (a, b) = tokio::join!(
        guarded_fetch(&state, &dir, year, month, &wanted),
        guarded_fetch(&state, &dir, year, month, &wanted),
    );
    println!(
        "two racing fetches finished in {:?}: {a:?} / {b:?}",
        started.elapsed()
    );

    // Exactly one of the two actually downloaded — the other found the tables already
    // current once it got the lock and skipped the network entirely. Both returning
    // `None` would mean neither fetched anything, which the fresh scratch directory rules
    // out; both returning `Some` would mean the mutex did not serialize them at all.
    let downloads = [&a, &b].into_iter().filter(|r| r.is_some()).count();
    assert_eq!(
        downloads, 1,
        "expected exactly one of the two racing calls to have actually fetched — \
         got a={a:?} b={b:?}"
    );

    // The proof that matters: the tables on disk are not corrupted by having been written
    // twice at once, and the STAMP file names the month that is actually on disk.
    let places = Places::open(&dir);
    assert!(
        !places.is_empty(),
        "the tables were left corrupted or missing after the race"
    );
    assert_eq!(
        geo::month_on_disk(&dir).as_deref(),
        Some(wanted.as_str()),
        "the STAMP file does not name the month whose tables are actually on disk"
    );

    let known = places.look_up("8.8.8.8");
    assert!(
        known.country.is_some(),
        "a known address could not be answered for after the race"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
