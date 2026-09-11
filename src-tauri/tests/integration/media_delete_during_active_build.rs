//! T596 — `media_delete`/`file_delete` refuse to run `rm -rf` through a top-level path a
//! `ladder_build`/`upload_start` is actively writing right now.
//!
//! **The gap, found by an independent QA audit 2026-09-11 (round 16).** `running_build_for`
//! (T591) and `running_upload_for` guarded only one direction: a second task could not start
//! on top of a running one. Neither was ever called from `library.rs` — so a `ladder_build`
//! for slug `t596` writing `video_dir/t596/v22` for as long as it takes to encode, and a
//! `media_delete` for the same medium arriving mid-build, raced straight through: `rm -rf`
//! against a directory a background task was still writing into, on a real server. Same
//! severity class as T593 — corruption of a live server, not merely a rejected click.
//!
//! **Why the medium's `ladders` entry is attached by hand rather than by running a real build
//! to completion first.** `attach_built_set` (`commands/ladder.rs`) only ties a built set to
//! its medium on a SUCCESSFUL run, and the realistic danger this guard exists for is a
//! REBUILD: a medium that already has `t596/master.m3u8` under `ladders`, being rebuilt while
//! somebody deletes it. Writing that catalogue entry directly with `manifest_io` — the same
//! door `library_ops.rs`'s own `a_second_copy_of_the_application_gets_a_refusal_code_of_its_own`
//! test uses to reach the catalogue for its own purposes — reaches that state in one step
//! rather than by running an encode start to finish only to throw its own timing away.
//!
//! Modelled on `ladder_build_race.rs` (T591): its fixture helpers are copied rather than
//! imported — each fixture file stays free to change its own shape without disturbing the
//! others, the same reasoning `deploy_run_race.rs` gives for the same choice.

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
/// is now being rebuilt, is really in. Through `gate::open`/`manifest_io`, the same door
/// `library_ops.rs` uses to reach the catalogue for its own purposes (see the module doc).
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
async fn media_delete_is_refused_while_a_build_of_the_same_slug_is_running() {
    let (server, state, id) = setup().await;

    let film_dir =
        std::env::temp_dir().join(format!("vrcast-t596-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&film_dir).expect("could not make a working directory");
    let film = film_dir.join("source.mp4");
    make_film(&film).expect("the fixture film would not encode");
    let path = film.to_string_lossy().into_owned();

    let media_id = library::media_create(&state, &id, "T596 fixture", Some("t596"))
        .await
        .expect("the medium was not created");

    // ---- the medium already has an attached set, as a rebuild target really would (see the
    // module doc for why this is attached by hand rather than by running a build first) ----
    attach_ladder_by_hand(&state, &id, &media_id, "t596").await;

    // ---- a rebuild of the SAME slug starts; its background task is very likely still
    // encoding once this `.await` resolves, the same timing `ladder_build_race.rs` relies on
    // ----
    let task_id = ladder::ladder_build(&state, build_request(&id, &path, "t596", false))
        .await
        .expect("the rebuild was refused although nothing else was running");
    tokio::time::sleep(Duration::from_millis(50)).await;

    // ---- media_delete on the SAME medium, while the rebuild is (almost certainly) still
    // running: refused with the new T596 code, not run through to `rm -rf` ----
    let err = library::media_delete(&state, &id, &media_id, true)
        .await
        .expect_err("media_delete went through while a rebuild of the same slug was running");
    assert_eq!(err.code, ErrorCode::MediaBusy);

    // ---- and the files are really still on the server — not merely "the command failed",
    // but "nothing was removed" ----
    server
        .exec_inside(&format!("test -d '{VIDEO_DIR}/t596'"))
        .expect("the medium's directory is gone from the server despite the refusal");

    // ---- the rebuild finishes normally, undisturbed ----
    wait_for_final(&state, &task_id, Duration::from_secs(60)).await;
    server
        .exec_inside(&format!("test -e '{VIDEO_DIR}/t596/master.m3u8'"))
        .expect("the rebuild itself did not finish on the server");

    // ---- and once the rebuild is done, the guard lifts: the same delete now succeeds ----
    library::media_delete(&state, &id, &media_id, true)
        .await
        .expect("media_delete was still refused after the rebuild finished");

    let _ = std::fs::remove_dir_all(&film_dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn media_delete_of_an_unrelated_medium_is_not_blocked_by_a_running_build() {
    let (_server, state, id) = setup().await;

    let film_dir = std::env::temp_dir().join(format!(
        "vrcast-t596-other-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&film_dir).expect("could not make a working directory");
    let film = film_dir.join("source.mp4");
    make_film(&film).expect("the fixture film would not encode");
    let path = film.to_string_lossy().into_owned();

    let building_id = library::media_create(&state, &id, "T596 building", Some("t596-building"))
        .await
        .expect("the first medium was not created");
    let other_id = library::media_create(&state, &id, "T596 untouched", Some("t596-untouched"))
        .await
        .expect("the second medium was not created");
    attach_ladder_by_hand(&state, &id, &building_id, "t596-building").await;

    let task_id = ladder::ladder_build(&state, build_request(&id, &path, "t596-building", false))
        .await
        .expect("the build was refused although nothing else was running");
    tokio::time::sleep(Duration::from_millis(50)).await;

    // The guard is scoped to the top-level paths actually touched, not a per-server lock: a
    // delete of an unrelated, empty medium must go through while the build runs.
    library::media_delete(&state, &id, &other_id, true)
        .await
        .expect("an unrelated medium's deletion was blocked by a build of a different slug");

    wait_for_final(&state, &task_id, Duration::from_secs(60)).await;
    let _ = std::fs::remove_dir_all(&film_dir);
}
