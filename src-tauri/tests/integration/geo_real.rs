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
