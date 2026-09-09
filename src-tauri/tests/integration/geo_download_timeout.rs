//! T575 — `store::geo::download()` no longer hangs forever on a connection that never
//! answers.
//!
//! `tests/integration/geo_real.rs` proves the tables come down correctly from the real
//! DB-IP service, but it cannot prove anything about a hang: DB-IP does not hang on
//! request, and that test is `#[ignore]`d besides. What is proved here instead, with a real
//! TCP socket and no real network dependency, is the failure this constant exists to bound:
//! a server that accepts the connection — so `reqwest` gets past DNS and TCP connect
//! cleanly, no fast "connection refused" — and then sends nothing back at all, forever.
//!
//! Without `store::geo::DOWNLOAD_TIMEOUT` actually wired into the client (rather than merely
//! declared and unused), `download()` against such a server would hang indefinitely: this
//! test's own outer `tokio::time::timeout` exists only as a backstop so a regression here
//! fails the test suite instead of hanging it, and is deliberately NOT the thing being
//! measured — see the comment at the assertion below for why elapsed time, not just `Err`,
//! is what actually proves the client-side timeout fired.

use std::time::{Duration, Instant};

use tokio::net::TcpListener;
use vrcast_studio_lib::store::geo::{download, DOWNLOAD_TIMEOUT};

/// How much slack either side of `DOWNLOAD_TIMEOUT` is allowed before the elapsed time no
/// longer counts as "the client timeout fired", rather than something else entirely (an
/// instant failure, or the outer backstop below catching a hang that never ends). Generous
/// enough to absorb scheduler jitter on a loaded machine, tight enough that an elapsed time
/// near-instant or near-double would still fail it.
const SLACK: Duration = Duration::from_secs(5);

#[tokio::test]
async fn a_connection_that_accepts_and_then_says_nothing_is_given_up_on_by_the_client_itself() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("could not bind a scratch listener");
    let port = listener
        .local_addr()
        .expect("the listener has no local address")
        .port();

    // Accept the one connection `download()` makes and then hold it open, sending nothing
    // back, ever. Accepting matters: a closed port gives `reqwest` an immediate connection
    // refused, which would pass this test for entirely the wrong reason (an error that has
    // nothing to do with the read timeout this test exists to prove).
    tokio::spawn(async move {
        if let Ok((socket, _)) = listener.accept().await {
            // Kept alive for as long as the socket lives, so the connection stays open
            // rather than being dropped (which would look like a reset to the client, not
            // a hang).
            std::future::pending::<()>().await;
            drop(socket);
        }
    });

    let url = format!("http://127.0.0.1:{port}/");
    let started = Instant::now();

    // The outer bound is a backstop, not the assertion: if the client-side timeout were
    // missing entirely, `download()` would hang forever and only THIS timeout would end the
    // test — which would still leave `result` an `Err`, and a test that only checked
    // "returned Err" would pass regardless of whether the client timeout exists at all. The
    // elapsed-time assertion below is what actually distinguishes the two.
    let outer_bound = DOWNLOAD_TIMEOUT + Duration::from_secs(30);
    let result = tokio::time::timeout(outer_bound, download(&url)).await;
    let elapsed = started.elapsed();

    let result = result.expect(
        "download() did not return even within DOWNLOAD_TIMEOUT + 30s — the outer backstop \
         had to end the test itself, meaning the client-side timeout is not bounding the \
         request at all",
    );
    assert!(
        result.is_err(),
        "download() against a connection that never answers came back Ok — nothing was \
         ever sent, so a success here means the body read did not actually happen"
    );

    // The proof that matters: the failure arrived at roughly DOWNLOAD_TIMEOUT, not
    // instantly (which would mean some OTHER error — not the read timeout — ended the
    // request early) and not near double or unbounded (which would mean the client timeout
    // is not what ended it, and the outer backstop above did the work instead).
    assert!(
        elapsed >= DOWNLOAD_TIMEOUT.saturating_sub(SLACK) && elapsed < DOWNLOAD_TIMEOUT + SLACK,
        "download() returned Err after {elapsed:?}, not within {SLACK:?} of the configured \
         DOWNLOAD_TIMEOUT of {DOWNLOAD_TIMEOUT:?} — the client-side timeout is not what \
         ended the request"
    );
}
