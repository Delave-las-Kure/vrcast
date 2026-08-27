//! T265 — walking down from the root, against the real network.
//!
//! This cannot be checked without a network and there is no pretending otherwise: what is
//! being checked is precisely that we reach the servers holding the zone rather than whatever
//! the machine's resolver has lying about. A fake in the middle would check the fake.
//!
//! `example.com` is used deliberately: IANA reserves it for exactly this and its delegation
//! does not move. **It has no AAAA record** — checked 2026-08-27, it answers only over IPv4 —
//! so nothing here asserts one. The IPv6 half of the rule is checked where it can be checked
//! honestly: in `tests/unit/dns_verdict.rs`, against records handed in rather than looked up.

use std::time::Duration;

use vrcast_studio_lib::net::dns::{look_up, DEFAULT_PATIENCE};

#[tokio::test]
async fn a_real_name_is_answered_from_the_root_downwards() {
    let records = look_up("example.com", DEFAULT_PATIENCE)
        .await
        .expect("a name reserved for this purpose could not be looked up");

    assert!(
        !records.a.is_empty(),
        "no ordinary record came back for a name that has one — the walk from the root did not \
         reach the servers holding the zone"
    );
}

#[tokio::test]
async fn a_name_that_does_not_exist_is_an_answer_rather_than_a_failure() {
    // **The distinction the whole module turns on.** "There is no such record" is something
    // the person can act on — they go and create it. "The lookup failed" is not, and sending
    // them to edit a record because a network was in the way is the worse mistake.
    //
    // The patience is set to nothing: the growing pause is for a record that is on its way,
    // and this one is not.
    let records = look_up(
        "no-such-name-vrcast-check.example.com",
        Duration::from_millis(1),
    )
    .await
    .expect("a name that does not exist came back as a failure");

    assert!(
        records.a.is_empty() && records.aaaa.is_empty(),
        "records came back for a name that does not exist: {records:?}"
    );
}

#[tokio::test]
async fn a_domain_that_does_not_exist_is_an_answer_too() {
    // A top level reserved as permanently unusable. This is the shape of a mistyped domain,
    // and it must come back as "nothing points here" rather than as a failure of the
    // application: the person's next move is the same either way — look at what they typed.
    let records = look_up("vrcast-nothing-here.invalid", Duration::from_millis(1))
        .await
        .expect("a domain that cannot exist came back as a failure of ours");

    assert!(records.a.is_empty() && records.aaaa.is_empty());
}

#[tokio::test]
async fn something_that_is_not_a_name_is_refused_before_the_network() {
    // Costs nothing to catch here and costs a walk from the root to catch later.
    let problem = look_up("not a domain at all", Duration::from_millis(1))
        .await
        .expect_err("a string with spaces in it was looked up");
    assert!(
        matches!(
            problem,
            vrcast_studio_lib::net::dns::Problem::NotAName { .. }
        ),
        "the wrong problem came back: {problem:?}"
    );
}
