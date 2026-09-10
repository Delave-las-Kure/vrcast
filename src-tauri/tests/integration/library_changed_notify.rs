//! T578 — `upload_start` and `ladder_build` say `library:changed` after writing the
//! manifest, the same way the five mutating commands in `library.rs` already do
//! (`media_create`, `media_rename`, `media_delete`, `file_move`, `file_delete`, all through
//! their shared `invalidate(state, server_id)`).
//!
//! **Why against a real server rather than a mocked-out `finish`/`ladder_build` closure.**
//! Both paths only reach the write this task cares about — `file_it_under` writing
//! `library.json` after the checksum comparison, and `attach_built_set` after the encoded
//! set lands — by going through `manifest_io::write` over a real connection. A test that
//! stubbed that write would prove the plumbing compiles, not that the event actually fires
//! after the thing the task description names: "after writing the manifest".
//!
//! **Why `state.subscribe()` before acting, exactly as `server_state_event.rs` (unit) and
//! `stand_scenarios.rs` (integration, for `ViewersUpdate`) already do it.** `AppEvent` is a
//! broadcast with no memory of its last value — a subscription made after the event fired
//! would see nothing, and that would look identical to the bug this task closes.
//!
//! What is checked, matching the task's own "Тесты" list:
//! 1. A successful `upload_start` that files the upload under a medium sends
//!    `AppEvent::LibraryChanged { server_id }` for the right server, end to end.
//! 2. The other side of (1): an upload with no medium chosen writes nothing to the
//!    manifest and must not claim it did.
//! 3. The pair `ladder_build`'s closure runs together on a successful build —
//!    `attach_built_set` then this task's own `invalidate_library_parts` — exercised
//!    directly against a real server (see the doc comment on the test itself for why not
//!    through the full command), plus a source check that the two really do sit inside the
//!    same `if outcome.is_ok()` in `ladder.rs`.
//! 4. In every case the on-disk `library_cache` is actually forgotten — not merely the
//!    event sent — because a screen reading through `library_list(refresh=false)` would
//!    otherwise hand back the stale cached view despite the event having arrived.

use std::time::Duration;

use vrcast_studio_lib::commands::library::api as library;
use vrcast_studio_lib::commands::servers::{api as servers, ServerInput};
use vrcast_studio_lib::commands::upload::{api as upload, UploadRequest};
use vrcast_studio_lib::commands::{AppEvent, AppState};
use vrcast_studio_lib::domain::server_profile::AuthKind;
use vrcast_studio_lib::ssh::{fingerprint, Connection, Credentials, ServerAddress};
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::library_cache;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;
use vrcast_studio_lib::tasks::state::TaskState;

use super::fixture::{key_path, TestServer, KEY_PASSPHRASE};
use super::hls_fixture;
use super::library_ops::confirm_fingerprint;

const VIDEO_DIR: &str = "/var/lib/vrcast/videos";

fn app_state() -> AppState {
    AppState::with_db(
        std::sync::Arc::new(Db::open_in_memory().unwrap()),
        std::sync::Arc::new(InMemorySecretStore::new()),
    )
    .expect("the application state would not assemble")
}

async fn setup() -> (TestServer, AppState, String) {
    let server = TestServer::start().expect("the container would not come up");
    let state = app_state();
    let input = ServerInput {
        name: String::from("Container"),
        host: server.host().to_owned(),
        port: server.port,
        user: String::from("root"),
        auth_kind: AuthKind::Key,
        key_path: Some(key_path().to_string_lossy().into_owned()),
        domain: String::from("stream.example.com"),
        video_dir: Some(String::from(VIDEO_DIR)),
        cdn_base: None,
        ipv6_mode: None,
    };
    let id =
        servers::server_add(&state, input, KEY_PASSPHRASE).expect("the profile was not created");
    confirm_fingerprint(&state, &id, &server).await;
    (server, state, id)
}

/// A connection independent of the application's own gate — the same shape
/// `ladder_attach.rs` and `manifest_conflict.rs` use for the same reason: setting
/// conditions up (or, here, standing in for what `ladder_build`'s own closure does) must
/// not lean on the code under test.
async fn connect(server: &TestServer) -> Connection {
    let addr = ServerAddress::new(server.host(), server.port);
    let fp = fingerprint::probe(&addr)
        .await
        .expect("the fingerprint was not obtained");
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
    .expect("connecting failed")
}

/// Wait for the next `AppEvent::LibraryChanged` on this receiver, ignoring any other kind of
/// event that might interleave (there should not be any here, but nothing about this test
/// depends on that).
async fn wait_for_library_changed(
    rx: &mut tokio::sync::broadcast::Receiver<AppEvent>,
    limit: Duration,
) -> String {
    let deadline = tokio::time::Instant::now() + limit;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let event = tokio::time::timeout(remaining, rx.recv())
            .await
            .expect("no AppEvent arrived at all within the time allowed")
            .expect("the event channel closed instead of delivering an event");
        if let AppEvent::LibraryChanged { server_id } = event {
            return server_id;
        }
    }
}

fn make_local_file(name: &str, size: usize) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vrcast-t578-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).expect("could not create the temporary directory");
    let path = dir.join(name);
    std::fs::write(&path, vec![7u8; size]).expect("could not write the file");
    path
}

async fn wait_done(state: &AppState, task_id: &str, limit: Duration) -> TaskState {
    let deadline = std::time::Instant::now() + limit;
    loop {
        if let Ok(Some(task)) = state.tasks.get(task_id) {
            if task.state.is_final() {
                return task.state;
            }
        }
        if std::time::Instant::now() >= deadline {
            let task = state.tasks.get(task_id).ok().flatten();
            panic!("the task did not finish in the time allowed: {task:?}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// 1. A successful `upload_start` that files under a medium notifies `LibraryChanged`, end
/// to end, and forgets the cache for real.
#[tokio::test]
async fn a_successful_upload_notifies_library_changed_and_clears_the_cache() {
    let (_server, state, id) = setup().await;
    let local = make_local_file("film_t578.mp4", 4096);

    let media_id = library::media_create(&state, &id, "T578 upload fixture", Some("t578up"))
        .await
        .expect("the medium was not created");

    // Populate the cache first, the way an open library screen would have already done —
    // otherwise "the cache is empty after the event" is trivially true because it was
    // already empty, and this checks nothing.
    let _ = library::library_list(&state, &id, true)
        .await
        .expect("the library would not read");
    assert!(
        library_cache::load(&state.db, &id)
            .expect("the cache would not read")
            .is_some(),
        "the cache was not populated ahead of the check — the check below would prove nothing"
    );

    let mut rx = state.subscribe();

    let task = upload::upload_start(
        &state,
        UploadRequest {
            server_id: id.clone(),
            local_path: local.to_string_lossy().into_owned(),
            remote_name: String::from("film_t578.mp4"),
            media_id: Some(media_id),
            limit_bps: None,
            confirmed: true,
        },
    )
    .await
    .expect("the upload would not submit");

    assert_eq!(
        wait_done(&state, &task, Duration::from_secs(60)).await,
        TaskState::Completed,
        "the upload did not finish successfully: {:?}",
        state.tasks.get(&task).ok().flatten()
    );

    let notified_server = wait_for_library_changed(&mut rx, Duration::from_secs(10)).await;
    assert_eq!(notified_server, id, "the event named the wrong server");

    assert!(
        library_cache::load(&state.db, &id)
            .expect("the cache would not read")
            .is_none(),
        "the library cache is still populated after the upload — a screen reading without \
         `refresh` would hand back the stale view despite the event having arrived"
    );
}

/// The other side: an upload with no `media_id` never reaches `file_it_under`, writes
/// nothing to the manifest, and must not claim that it did.
#[tokio::test]
async fn an_upload_with_no_medium_chosen_does_not_notify_library_changed() {
    let (_server, state, id) = setup().await;
    let local = make_local_file("film_t578_unfiled.mp4", 4096);

    let mut rx = state.subscribe();

    let task = upload::upload_start(
        &state,
        UploadRequest {
            server_id: id.clone(),
            local_path: local.to_string_lossy().into_owned(),
            remote_name: String::from("film_t578_unfiled.mp4"),
            media_id: None,
            limit_bps: None,
            confirmed: true,
        },
    )
    .await
    .expect("the upload would not submit");

    assert_eq!(
        wait_done(&state, &task, Duration::from_secs(60)).await,
        TaskState::Completed
    );

    let empty = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await;
    assert!(
        empty.is_err(),
        "an event arrived although the upload was never filed under a medium and wrote \
         nothing to the manifest"
    );
}

/// **Why `ladder_build` is not driven end to end here, the way the upload check above drives
/// `upload_start` end to end.** Tried first, and it does not reach the line under test.
/// `commands::ladder::api::ladder_build`'s closure only runs `attach_built_set` — and, right
/// beside it, this task's own `invalidate_library_parts` call — inside `if outcome.is_ok()`,
/// and `outcome` is the result of `tasks::ladder_build::run`, whose very last step is
/// `hls_verify::verify(job.master_url, …)`. `master_url` is built from the profile's
/// `domain` field with a hardcoded `https://` scheme (`domain::links::for_path`) — here
/// `https://stream.example.com/…`, which resolves nowhere and terminates no TLS in this
/// container fixture. So `verify` always returns `Err`, `outcome` is always `Err`, and the
/// whole guarded block — attach AND invalidate together — never runs at all, no matter how
/// long the build is waited on. `ladder_active_viewers.rs`'s own module doc names this exact
/// wall for the same reason (its own checks wait on `master.m3u8` landing on the server
/// rather than on the task's final state).
///
/// This is not a gap this task introduces: it is why `ladder_attach.rs` tests
/// `attach_built_set` directly rather than through the full command — the same pattern this
/// test follows for the two calls `ladder_build`'s closure runs together on success:
/// connects independently (as `ladder_attach.rs` and `manifest_conflict.rs` already do, so
/// setting conditions up does not lean on the code under test), calls `attach_built_set` for
/// real against a laid-out set on a real server, then calls
/// `AppState::invalidate_library` — the public route to the same crate-private
/// `invalidate_library_parts` call `ladder.rs`'s closure makes right beside `attach_built_set`
/// — and checks the event and the cache exactly as the upload check above does.
///
/// `ladder_build_calls_attach_and_invalidate_together_on_success` right below is the other
/// half: it reads `ladder.rs`'s own source to confirm the two calls this test exercises
/// separately really do sit inside the same `if outcome.is_ok()` in the real closure, the
/// same way `engine.rs::the_ladder_build_says_things_as_they_happen_rather_than_at_the_end`
/// already reads `ladder_build.rs`'s source to confirm a different wiring fact this same
/// kind of fixture wall keeps it from exercising end to end.
#[tokio::test]
async fn the_pair_ladder_build_runs_on_success_notifies_library_changed_and_clears_the_cache() {
    let (server, state, id) = setup().await;

    let media_id = library::media_create(&state, &id, "T578 ladder fixture", Some("t578ladder"))
        .await
        .expect("the medium was not created");

    hls_fixture::lay_out_ladder(&server, "t578ladder").expect("the quality set was not laid out");

    // Populate the cache ahead of the write, same reasoning as the upload check above.
    let _ = library::library_list(&state, &id, true)
        .await
        .expect("the library would not read");
    assert!(
        library_cache::load(&state.db, &id)
            .expect("the cache would not read")
            .is_some(),
        "the cache was not populated ahead of the check — the check below would prove nothing"
    );

    let mut rx = state.subscribe();

    // Exactly the pair `ladder_build`'s closure runs together inside `if outcome.is_ok()`:
    // attach, then invalidate — over the same connection a real build would still be
    // holding.
    let conn = connect(&server).await;
    let attached =
        vrcast_studio_lib::commands::ladder::attach_built_set(&conn, VIDEO_DIR, "t578ladder")
            .await
            .expect("the set was not attached to any medium");
    assert_eq!(
        attached, media_id,
        "the set was attached to the wrong medium"
    );
    // The same public route `AppState::invalidate_library` gives `library.rs`'s own
    // `invalidate` wrapper — equivalent to the crate-private `invalidate_library_parts`
    // `ladder.rs`'s closure actually calls (see `AppState::invalidate_library`'s own doc
    // comment), reached here because a test outside the crate cannot call a `pub(crate)`
    // free function directly.
    state.invalidate_library(&id);
    conn.close().await;

    let notified_server = wait_for_library_changed(&mut rx, Duration::from_secs(10)).await;
    assert_eq!(notified_server, id, "the event named the wrong server");

    assert!(
        library_cache::load(&state.db, &id)
            .expect("the cache would not read")
            .is_none(),
        "the library cache is still populated after the write — a screen reading without \
         `refresh` would hand back the stale view despite the event having arrived"
    );
}

/// The other half of the pair above: that the two calls it exercises separately really do
/// sit inside one and the same `if outcome.is_ok()` in `ladder.rs`'s real closure, so that
/// the previous check is not merely plausible but actually describes the wiring.
///
/// A source check, in the same spirit as
/// `engine.rs::the_ladder_build_says_things_as_they_happen_rather_than_at_the_end` — reached
/// for there, and here, for the same reason: the fixture wall documented above keeps this
/// project's own `TestServer` from ever driving `outcome.is_ok()` to `true`, so nothing
/// short of a real reachable domain (`stand_scenarios.rs`'s territory, and ignored by
/// default for exactly that reason) can exercise the guard itself end to end. Reading the
/// source is not a substitute for the behavioural check above — it is what closes the one
/// gap that check cannot: that `attach_built_set` and `invalidate_library_parts` are truly
/// the same "on success" as each other, not two things this test merely assumed line up.
#[test]
fn ladder_build_calls_attach_and_invalidate_together_on_success() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/ladder.rs");
    let text = std::fs::read_to_string(&path).expect("could not read ladder.rs");

    let guard = text
        .find("if outcome.is_ok() {")
        .expect("ladder_build's closure no longer guards attaching on outcome.is_ok()");
    // The next closing brace at the same nesting level as the `if` would take real brace
    // counting to find exactly; a generous slice past the guard is enough to prove both
    // calls are textually inside it without needing a parser, the same trade the source
    // checks in `engine.rs` already make.
    let inside = &text[guard..(guard + 1200).min(text.len())];

    assert!(
        inside.contains("attach_built_set("),
        "attach_built_set is no longer called inside the outcome.is_ok() guard"
    );
    assert!(
        inside.contains("invalidate_library_parts("),
        "T578: invalidate_library_parts is no longer called inside the same \
         outcome.is_ok() guard as attach_built_set — the event would then fire (or not) \
         out of step with whether the build actually succeeded"
    );
}
