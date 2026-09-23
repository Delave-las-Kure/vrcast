//! T606 — `media_rename` refuses to move a top-level path ONTO a name an active
//! `upload_start` is about to enter serving under.
//!
//! **The gap T599 left, found by an independent QA audit 2026-09-23 (round 18).** T599 made
//! `media_rename` check the medium's CURRENT top-level paths against running tasks; the new
//! names were only computed later, inside the `mv` loop, and never asked about. Scenario:
//! medium `t606` owns `t606_9.mp4`; a long upload of a new `t606-fresh_9.mp4` is in transfer
//! (its final file does not exist yet — it is still in the staging directory); a confirmed
//! rename `t606`→`t606-fresh` moves the old file onto `t606-fresh_9.mp4`, and the upload's
//! own `mv -f` into serving overwrites it the moment it finishes. `slug_available` could not
//! catch it: it only knows the catalogue, not what running tasks are about to create.
//!
//! Modelled on `media_rename_during_active_build.rs` (T599); the fixture helpers are copied
//! rather than imported for the same reason that module gives.

use std::sync::Arc;
use std::time::Duration;

use vrcast_studio_lib::commands::error::ErrorCode;
use vrcast_studio_lib::commands::library::api as library;
use vrcast_studio_lib::commands::servers::{api as servers, ServerInput};
use vrcast_studio_lib::commands::upload::{api as upload, UploadRequest};
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::domain::server_profile::AuthKind;
use vrcast_studio_lib::server::{gate, manifest_io};
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;

use super::fixture::{key_path, TestServer, KEY_PASSPHRASE};
use super::library_ops::confirm_fingerprint;

const VIDEO_DIR: &str = "/var/lib/vrcast/videos";

/// Big enough, at [`SLOW_BPS`], for the upload to still be in transfer for the whole of the
/// rename (~24 s); the test cancels it once it has what it needs.
const FILE_SIZE: usize = 12 * 1024 * 1024;
const SLOW_BPS: u64 = 512 * 1024;

fn app_state() -> AppState {
    AppState::with_db(
        Arc::new(Db::open_in_memory().unwrap()),
        Arc::new(InMemorySecretStore::new()),
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

/// A local file with non-uniform contents — see `upload_live.rs`'s identical helper.
fn make_local_file(name: &str, size: usize) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vrcast-t606-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).expect("could not create the temporary directory");
    let path = dir.join(name);
    let mut data = Vec::with_capacity(size);
    let mut x: u32 = 0x1234_5678;
    while data.len() < size {
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        data.extend_from_slice(&x.to_le_bytes());
    }
    data.truncate(size);
    std::fs::write(&path, &data).expect("could not write the file");
    path
}

/// Attach an already-served `file` to `media_id` in the catalogue and lay it out on the
/// server with recognisable contents — the medium's existing `t606_9.mp4`.
async fn attach_file_by_hand(state: &AppState, server_id: &str, media_id: &str, file: &str) {
    let profile = vrcast_studio_lib::store::profiles::get(&state.db, server_id)
        .unwrap()
        .expect("no such profile");
    let conn = gate::open(state.secrets.as_ref(), &profile, gate::Intent::Change)
        .await
        .expect("could not connect")
        .conn;
    let manifest = manifest_io::read(&conn, &profile.video_dir)
        .await
        .expect("the catalogue would not read");
    let next = manifest
        .with_file_under(media_id, file, false)
        .expect("the medium was not found to attach the file to");
    manifest_io::write(&conn, &profile.video_dir, &next, manifest.generation)
        .await
        .expect("the catalogue would not write");
    conn.exec(&format!("echo 'the old film' > '{VIDEO_DIR}/{file}'"))
        .await
        .expect("could not lay out the pre-existing file on the server");
    conn.close().await;
}

/// Start a slow upload of a new file under `remote_name`; it is still in transfer when this
/// returns (the resume token naming its target is written before `upload_start` returns).
async fn start_slow_upload(state: &AppState, server_id: &str, remote_name: &str) -> String {
    let local = make_local_file("fresh.mp4", FILE_SIZE);
    let task = upload::upload_start(
        state,
        UploadRequest {
            server_id: server_id.to_owned(),
            local_path: local.to_string_lossy().into_owned(),
            remote_name: remote_name.to_owned(),
            media_id: None,
            limit_bps: Some(SLOW_BPS),
            confirmed: true,
        },
    )
    .await
    .expect("the upload was refused although nothing else was running");
    let record = state
        .tasks
        .get(&task)
        .unwrap()
        .expect("the upload task vanished");
    assert!(
        record.resume_token.is_some(),
        "the upload's target was not recorded, so no guard could see it"
    );
    task
}

async fn cancel_and_wait(state: &AppState, task_id: &str) {
    let _ = state.tasks.cancel(task_id);
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(Some(task)) = state.tasks.get(task_id) {
            if task.state.is_final() {
                return;
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the cancelled upload never reached a final state"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn media_rename_onto_the_target_of_a_running_upload_is_refused() {
    let (server, state, id) = setup().await;

    let media_id = library::media_create(&state, &id, "T606 fixture", Some("t606"))
        .await
        .expect("the medium was not created");
    attach_file_by_hand(&state, &id, &media_id, "t606_9.mp4").await;

    // ---- a new `t606-fresh_9.mp4` is in transfer; nothing under that final name yet ----
    let task = start_slow_upload(&state, &id, "t606-fresh_9.mp4").await;
    assert!(
        server
            .exec_inside(&format!("test -e '{VIDEO_DIR}/t606-fresh_9.mp4'"))
            .is_err(),
        "the upload's final file already exists — the scenario needs it still in transfer"
    );

    // ---- rename t606 → t606-fresh (confirmed) would move t606_9.mp4 onto the upload's
    // target: refused, with the busy code, before any `mv` ----
    let err = library::media_rename(&state, &id, &media_id, None, Some("t606-fresh"), true)
        .await
        .expect_err("media_rename moved a file onto the target of a running upload");
    assert_eq!(err.code, ErrorCode::MediaBusy);

    // ---- nothing was moved: the old file is where it was, with its contents, and the
    // upload's final name is still free ----
    let old = server
        .exec_inside(&format!("cat '{VIDEO_DIR}/t606_9.mp4'"))
        .expect("the medium's file is gone from its old name despite the refusal");
    assert!(
        old.contains("the old film"),
        "the old file's contents changed: {old:?}"
    );
    assert!(
        server
            .exec_inside(&format!("test -e '{VIDEO_DIR}/t606-fresh_9.mp4'"))
            .is_err(),
        "a file was moved onto the upload's target despite the refusal"
    );

    // ---- and the catalogue was not rewritten either ----
    let listed = library::library_list(&state, &id, true)
        .await
        .expect("the library would not list");
    let medium = listed
        .media
        .iter()
        .find(|m| m.id == media_id)
        .expect("the medium vanished");
    assert_eq!(medium.slug, "t606");

    cancel_and_wait(&state, &task).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn media_rename_that_misses_the_running_upload_goes_through() {
    let (server, state, id) = setup().await;

    let media_id = library::media_create(&state, &id, "T606 fixture", Some("t606"))
        .await
        .expect("the medium was not created");
    attach_file_by_hand(&state, &id, &media_id, "t606_9.mp4").await;

    let task = start_slow_upload(&state, &id, "t606-fresh_9.mp4").await;

    // The guard is scoped to the names actually touched: a rename whose destinations do not
    // meet the upload's target goes through while the upload runs.
    library::media_rename(&state, &id, &media_id, None, Some("t606-other"), true)
        .await
        .expect("a rename that does not touch the upload's target was refused");

    server
        .exec_inside(&format!("test -e '{VIDEO_DIR}/t606-other_9.mp4'"))
        .expect("the medium's file was not renamed on the server");
    assert!(
        server
            .exec_inside(&format!("test -e '{VIDEO_DIR}/t606_9.mp4'"))
            .is_err(),
        "the old name is still there after the rename"
    );

    cancel_and_wait(&state, &task).await;
}
