//! T599 — `media_rename` refuses to run `mv -n` through a top-level path a
//! `ladder_build`/`upload_start` is actively writing right now.
//!
//! **The gap, flagged by T596 and left for the coordinator to decide, then found again by an
//! independent QA audit 2026-09-16 (round 17).** `media_delete`/`file_delete` call
//! `refuse_if_busy` before touching a single top-level path on the server (T596);
//! `media_rename` never did, although `rename_entries` runs `mv -n` on exactly the same
//! top-level paths `media_delete` runs `rm -rf` on. A `ladder_build` for slug `t599` writing
//! `video_dir/t599/v22` for as long as it takes to encode, and a `media_rename` for the same
//! medium (changing its slug) arriving mid-build, raced straight through: `mv -n` moving a
//! directory a background task was still writing into, on a real server. Same severity class
//! as T596/T593 — corruption of a live server, not merely a rejected click.
//!
//! Modelled directly on `media_delete_during_active_build.rs` (T596): same fixture shapes,
//! same reasoning for attaching a ladder by hand rather than running a build to completion
//! first (see that module's own doc for the details), copied here rather than imported for
//! the same reason `ladder_build_race.rs`/`deploy_run_race.rs` give: each fixture file stays
//! free to change its own shape without disturbing the others.

use std::path::Path;
use std::time::Duration;

use vrcast_studio_lib::commands::error::ErrorCode;
use vrcast_studio_lib::commands::ladder::{api as ladder, BuildRequest};
use vrcast_studio_lib::commands::library::api as library;
use vrcast_studio_lib::commands::servers::{api as servers, ServerInput};
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::domain::ladder::{Quality, Rung};
use vrcast_studio_lib::domain::server_profile::AuthKind;
use vrcast_studio_lib::media::ffmpeg;
use vrcast_studio_lib::server::{gate, manifest_io};
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;

use super::fixture::{key_path, TestServer, KEY_PASSPHRASE};
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

/// Attach `{slug}/master.m3u8` to `media_id` in the catalogue directly, and make a plausible
/// directory for it on the server — the state a medium that has already been built once, and
/// is now being rebuilt, is really in. See `media_delete_during_active_build.rs`'s identical
/// helper for the full reasoning.
async fn attach_ladder_by_hand(state: &AppState, server_id: &str, media_id: &str, slug: &str) {
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
        .with_file_under(media_id, &format!("{slug}/master.m3u8"), true)
        .expect("the medium was not found to attach the ladder to");
    manifest_io::write(&conn, &profile.video_dir, &next, manifest.generation)
        .await
        .expect("the catalogue would not write");
    conn.exec(&format!(
        "mkdir -p '{VIDEO_DIR}/{slug}' && echo '#EXTM3U' > '{VIDEO_DIR}/{slug}/master.m3u8'"
    ))
    .await
    .expect("could not lay out the pre-existing set on the server");
    conn.close().await;
}

/// A tiny, real film — see `ladder_build_race.rs`'s identical helper for why this shape.
fn make_film(path: &Path) -> Result<(), String> {
    let ffmpeg_bin = ffmpeg::locate("ffmpeg").map_err(|e| e.to_string())?;
    let out = std::process::Command::new(ffmpeg_bin)
        .args([
            "-nostdin",
            "-y",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x240:rate=24:duration=2",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=2",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-b:v",
            "1000k",
            "-g",
            "24",
            "-keyint_min",
            "24",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
        ])
        .arg(path)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    Ok(())
}

fn rungs() -> Vec<Rung> {
    vec![
        Rung {
            index: 0,
            bitrate_bps: 500_000,
            maxrate_bps: 550_000,
            bufsize_bps: 550_000,
            width: 320,
            height: 240,
            level: String::from("3.0"),
            reasons: Vec::new(),
            quality: Quality::MeasuredHere { vmaf_x100: 9200 },
        },
        Rung {
            index: 1,
            bitrate_bps: 250_000,
            maxrate_bps: 275_000,
            bufsize_bps: 275_000,
            width: 320,
            height: 240,
            level: String::from("3.0"),
            reasons: Vec::new(),
            quality: Quality::MeasuredHere { vmaf_x100: 8800 },
        },
    ]
}

fn build_request(server_id: &str, path: &str, slug: &str, confirmed: bool) -> BuildRequest {
    BuildRequest {
        server_id: server_id.to_owned(),
        path: path.to_owned(),
        slug: slug.to_owned(),
        rungs: rungs(),
        audio_track: 0,
        prefer_hardware: false,
        batch: None,
        confirmed,
    }
}

async fn wait_for_final(state: &AppState, task_id: &str, limit: Duration) {
    use vrcast_studio_lib::commands::api as core;
    let deadline = std::time::Instant::now() + limit;
    loop {
        let task = core::task_get(state, task_id).expect("the task vanished from the list");
        if task.state.is_final() {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "task '{task_id}' never reached a final state in the time allowed (state: {:?})",
            task.state
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn media_rename_is_refused_while_a_build_of_the_same_slug_is_running() {
    let (server, state, id) = setup().await;

    let film_dir =
        std::env::temp_dir().join(format!("vrcast-t599-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&film_dir).expect("could not make a working directory");
    let film = film_dir.join("source.mp4");
    make_film(&film).expect("the fixture film would not encode");
    let path = film.to_string_lossy().into_owned();

    let media_id = library::media_create(&state, &id, "T599 fixture", Some("t599"))
        .await
        .expect("the medium was not created");

    // ---- the medium already has an attached set, as a rebuild target really would ----
    attach_ladder_by_hand(&state, &id, &media_id, "t599").await;

    // ---- a rebuild of the SAME slug starts; its background task is very likely still
    // encoding once this `.await` resolves, the same timing `ladder_build_race.rs` relies on
    // ----
    let task_id = ladder::ladder_build(&state, build_request(&id, &path, "t599", false))
        .await
        .expect("the rebuild was refused although nothing else was running");
    tokio::time::sleep(Duration::from_millis(50)).await;

    // ---- media_rename on the SAME medium (changing its slug), while the rebuild is
    // (almost certainly) still running: refused with the new T599 code, not run through to
    // `mv -n` ----
    let err = library::media_rename(&state, &id, &media_id, None, Some("t599-renamed"), true)
        .await
        .expect_err("media_rename went through while a rebuild of the same slug was running");
    assert_eq!(err.code, ErrorCode::MediaBusy);

    // ---- and the directory is really still under its old name on the server — not merely
    // "the command failed", but "nothing was moved" ----
    server
        .exec_inside(&format!("test -d '{VIDEO_DIR}/t599'"))
        .expect("the medium's directory is gone from its old name despite the refusal");
    assert!(
        server
            .exec_inside(&format!("test -d '{VIDEO_DIR}/t599-renamed'"))
            .is_err(),
        "the medium's directory was moved to the new name despite the refusal"
    );

    // ---- the rebuild finishes normally, undisturbed ----
    wait_for_final(&state, &task_id, Duration::from_secs(60)).await;
    server
        .exec_inside(&format!("test -e '{VIDEO_DIR}/t599/master.m3u8'"))
        .expect("the rebuild itself did not finish on the server");

    // ---- and once the rebuild is done, the guard lifts: the same rename now succeeds ----
    library::media_rename(&state, &id, &media_id, None, Some("t599-renamed"), true)
        .await
        .expect("media_rename was still refused after the rebuild finished");
    server
        .exec_inside(&format!("test -d '{VIDEO_DIR}/t599-renamed'"))
        .expect("the medium's directory was not renamed on the server after the guard lifted");

    let _ = std::fs::remove_dir_all(&film_dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn media_rename_of_an_unrelated_medium_is_not_blocked_by_a_running_build() {
    let (_server, state, id) = setup().await;

    let film_dir = std::env::temp_dir().join(format!(
        "vrcast-t599-other-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&film_dir).expect("could not make a working directory");
    let film = film_dir.join("source.mp4");
    make_film(&film).expect("the fixture film would not encode");
    let path = film.to_string_lossy().into_owned();

    let building_id = library::media_create(&state, &id, "T599 building", Some("t599-building"))
        .await
        .expect("the first medium was not created");
    let other_id = library::media_create(&state, &id, "T599 untouched", Some("t599-untouched"))
        .await
        .expect("the second medium was not created");
    attach_ladder_by_hand(&state, &id, &building_id, "t599-building").await;

    let task_id = ladder::ladder_build(&state, build_request(&id, &path, "t599-building", false))
        .await
        .expect("the build was refused although nothing else was running");
    tokio::time::sleep(Duration::from_millis(50)).await;

    // The guard is scoped to the top-level paths actually touched, not a per-server lock: a
    // rename of an unrelated, empty medium must go through while the build runs.
    library::media_rename(
        &state,
        &id,
        &other_id,
        None,
        Some("t599-untouched-renamed"),
        true,
    )
    .await
    .expect("an unrelated medium's rename was blocked by a build of a different slug");

    wait_for_final(&state, &task_id, Duration::from_secs(60)).await;
    let _ = std::fs::remove_dir_all(&film_dir);
}
