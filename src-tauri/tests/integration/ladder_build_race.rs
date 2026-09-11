//! T591 — `ladder_build` refuses a second concurrent build of the same slug on the same
//! server, the same pattern `running_upload_for` already gives `upload_start`
//! (`commands/upload.rs:150-156, 293-311`) and for the same reason: two builds racing each
//! other would read and write one `master.m3u8` and one set of `v{N}` directories, and
//! whichever finished last would silently win — no error, no warning, just a set that does
//! not match what either caller asked for. Found by an independent QA audit (round 14):
//! `upload_start` had this guard and `ladder_build` did not.
//!
//! Modelled on `ladder_active_viewers.rs`: its small helpers (`app_state`, `setup`,
//! `make_film`, `rungs`, `build_request`, `wait_for_master`) are copied rather than imported
//! — that file keeps everything private, by design, so each fixture file stays free to change
//! its own shape without disturbing the others.
//!
//! **Why the race window is real without any extra synchronisation trick.** The first call's
//! own `.await` only resolves once `ladder_build` has finished its own synchronous preflight
//! and handed the background task off to `submit_in_batch` — the background task itself (the
//! actual encode) is almost certainly still running when that `.await` returns, because
//! encoding two software rungs of even a two-second clip takes real wall-clock time. So a
//! second call made immediately after very reliably lands while the first is still
//! `!is_final()`. A short sleep is used as a fallback only if that turns out too flaky in
//! practice — see the sleep below.

use std::path::Path;
use std::time::Duration;

use vrcast_studio_lib::commands::error::ErrorCode;
use vrcast_studio_lib::commands::ladder::{api as ladder, BuildRequest};
use vrcast_studio_lib::commands::library::api as library;
use vrcast_studio_lib::commands::servers::{api as servers, ServerInput};
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::domain::ladder::{Quality, Rung};
use vrcast_studio_lib::domain::server_profile::AuthKind;
use vrcast_studio_lib::domain::wording::DetailCode;
use vrcast_studio_lib::media::ffmpeg;
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

/// A tiny, real film — small and short enough that even a software encode of two rungs
/// finishes in a few seconds, which is all this file needs from it.
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

/// Two small rungs, already `MeasuredHere` — see the module doc on `ladder_active_viewers.rs`
/// for why a real measurement grid is not run here; the same reasoning applies unchanged.
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
        // Software: what is being checked here holds on a machine with no graphics card,
        // which is most machines this will ever run on — the same reasoning `seams.rs`
        // gives for the same choice.
        prefer_hardware: false,
        batch: None,
        confirmed,
    }
}

/// Wait until `master.m3u8` exists in the given slug's directory on the server.
///
/// See `ladder_active_viewers.rs`'s module doc for why this, and not the task's own final
/// state, is what these checks wait on.
async fn wait_for_master(server: &TestServer, slug: &str, limit: Duration) {
    let deadline = std::time::Instant::now() + limit;
    loop {
        if server
            .exec_inside(&format!("test -e '{VIDEO_DIR}/{slug}/master.m3u8'"))
            .is_ok()
        {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "master.m3u8 for '{slug}' never appeared on the server in the time allowed"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Wait until `running_build_for` would no longer count this task — i.e. `is_final()`.
///
/// `wait_for_master` alone is not enough for the "guard lifts" check below: `write_master`
/// runs partway through `tasks::ladder_build::run`, well before the task itself reaches a
/// final state (there is still `attach_built_set`, the library-cache invalidation, and the
/// task engine's own bookkeeping to get through after that). A third `ladder_build` call
/// made right after `wait_for_master` returns can still race the first task's own trip to
/// `Completed` — so what "the guard lifts" actually needs waiting on is the task record
/// itself, not a side effect that lands earlier in the same run.
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
async fn a_second_build_of_the_same_slug_is_refused_while_the_first_runs() {
    let (server, state, id) = setup().await;

    let film_dir =
        std::env::temp_dir().join(format!("vrcast-t591-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&film_dir).expect("could not make a working directory");
    let film = film_dir.join("source.mp4");
    make_film(&film).expect("the fixture film would not encode");
    let path = film.to_string_lossy().into_owned();

    library::media_create(&state, &id, "T591 fixture", Some("t591"))
        .await
        .expect("the medium was not created");

    // ---- the first build starts; its background task is very likely still encoding once
    // this `.await` resolves, because submitting only waits for the synchronous preflight,
    // not for the encode itself ----
    let task_id_1 = ladder::ladder_build(&state, build_request(&id, &path, "t591", false))
        .await
        .expect("the first build was refused although nothing else was running");

    // A small cushion for the rare case the first task's encode is fast enough to already
    // have finished by the time the second call below runs — see the module doc.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // ---- the second build of the SAME slug, while the first is (almost certainly) still
    // running: refused synchronously, with the T591 detail code naming the slug ----
    let err = ladder::ladder_build(&state, build_request(&id, &path, "t591", false))
        .await
        .expect_err("a second concurrent build of the same slug went through");
    assert_eq!(err.code, ErrorCode::NameExists);
    assert!(
        err.says(DetailCode::BuildAlreadyRunning),
        "the refusal did not carry the T591 detail code: {err:?}"
    );

    // ---- the first build reaches the server despite the second call's refusal ----
    wait_for_master(&server, "t591", Duration::from_secs(60)).await;

    // ---- and once it has actually finished (is_final(), not merely `master.m3u8` having
    // landed — see `wait_for_final`'s own doc), the guard lifts: a further build of the same
    // slug is not refused forever ----
    wait_for_final(&state, &task_id_1, Duration::from_secs(60)).await;
    ladder::ladder_build(&state, build_request(&id, &path, "t591", false))
        .await
        .expect("a build of the same slug was refused even after the earlier one finished");
    wait_for_master(&server, "t591", Duration::from_secs(60)).await;

    let _ = std::fs::remove_dir_all(&film_dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_build_of_a_different_slug_is_not_blocked_by_a_running_one() {
    let (server, state, id) = setup().await;

    let film_dir = std::env::temp_dir().join(format!(
        "vrcast-t591-other-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&film_dir).expect("could not make a working directory");
    let film = film_dir.join("source.mp4");
    make_film(&film).expect("the fixture film would not encode");
    let path = film.to_string_lossy().into_owned();

    library::media_create(&state, &id, "T591 fixture", Some("t591"))
        .await
        .expect("the medium was not created");
    library::media_create(&state, &id, "T591 other fixture", Some("t591-other"))
        .await
        .expect("the second medium was not created");

    ladder::ladder_build(&state, build_request(&id, &path, "t591", false))
        .await
        .expect("the first build was refused although nothing else was running");

    // The guard is scoped to the slug, not a per-server lock: a build of a different slug
    // must go through while the first is (almost certainly) still running.
    ladder::ladder_build(&state, build_request(&id, &path, "t591-other", false))
        .await
        .expect("a build of a different slug was blocked by an unrelated running build");

    wait_for_master(&server, "t591", Duration::from_secs(60)).await;
    wait_for_master(&server, "t591-other", Duration::from_secs(60)).await;

    let _ = std::fs::remove_dir_all(&film_dir);
}
