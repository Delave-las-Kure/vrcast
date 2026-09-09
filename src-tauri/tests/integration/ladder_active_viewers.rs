//! T571 — `ladder_build` refuses a rebuild while somebody is actively watching, unless
//! `confirmed`, the same mechanism `media_rename`/`media_delete`/`upload_start` already have
//! for the same reason (FR-019a, FR-037): a rebuild rewrites `master.m3u8` on the server, and
//! a rebuild while it is being served can wash a real viewer's quality out from under them —
//! `tasks::ladder_build.rs:245-251` (T468) names a real incident where this happened quietly
//! on production.
//!
//! Modelled directly on
//! `library_ops.rs::renaming_the_short_name_while_watched_is_refused_unless_confirmed`: a
//! real `TestServer`, a real `Viewer::attach` + a real download in flight, `media_rename`'s
//! own refusal code (`ErrorCode::FileInUse`), and the same "confirmed still goes through"
//! shape.
//!
//! **Why the rungs are hand-built with `Quality::MeasuredHere` rather than run through a
//! real measurement grid.** What T571 changes is entirely in `ladder_build`'s own preflight,
//! before a task is ever submitted — nothing about the measurement path is touched. Running
//! the real few-minute measurement grid to reach that preflight would test `seams.rs`'s
//! ground a second time while proving nothing new about T571 itself; `seams.rs` already
//! takes the same shortcut for the same reason.
//!
//! **Why the checks below do not wait for the task to finish as `Completed`.** Unlike
//! `seams.rs`, which builds its own `master_url` pointing straight at the container
//! (`http://{server.host()}:{server.http_port}/...`), `commands::ladder::api::ladder_build`
//! computes its own `master_url` from the profile's `domain` field
//! (`domain::links::for_path`) — `https://stream.example.com/...` here, which resolves
//! nowhere and terminates no TLS. That is exactly right for what T571 touches (the
//! preflight, which runs before any of that) and irrelevant to it (the final
//! `hls_verify::verify` step, unreachable in this fixture regardless of T571). So what these
//! checks wait for is not the task's own final state but the one fact T571 actually claims:
//! whether `master.m3u8` was written to the server at all. `tasks::ladder_build::run` writes
//! it well before the unreachable verify step ever runs, so its presence on the server is
//! proof the confirmation guard let the build through — proof that does not depend on
//! network reachability this fixture cannot provide.

use std::path::Path;
use std::time::{Duration, Instant};

use vrcast_studio_lib::commands::error::ErrorCode;
use vrcast_studio_lib::commands::ladder::{api as ladder, BuildRequest};
use vrcast_studio_lib::commands::library::api as library;
use vrcast_studio_lib::commands::servers::{api as servers, ServerInput};
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::domain::ladder::{Quality, Rung};
use vrcast_studio_lib::domain::server_profile::AuthKind;
use vrcast_studio_lib::media::ffmpeg;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;

use super::fixture::{key_path, TestServer, KEY_PASSPHRASE};
use super::library_ops::confirm_fingerprint;
use super::viewer::Viewer;

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

/// Two small rungs, already `MeasuredHere` — see the module doc for why a real measurement
/// grid is not run here.
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
/// See the module doc for why this, and not the task's own final state, is what these
/// checks wait on: the build's final `hls_verify::verify` step is unreachable in this
/// fixture regardless of T571, but `write_master` runs well before it.
async fn wait_for_master(server: &TestServer, slug: &str, limit: Duration) {
    let deadline = Instant::now() + limit;
    loop {
        if server
            .exec_inside(&format!("test -e '{VIDEO_DIR}/{slug}/master.m3u8'"))
            .is_ok()
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "master.m3u8 for '{slug}' never appeared on the server in the time allowed"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rebuilding_while_watched_is_refused_unless_confirmed() {
    let (server, state, id) = setup().await;

    let film_dir =
        std::env::temp_dir().join(format!("vrcast-t571-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&film_dir).expect("could not make a working directory");
    let film = film_dir.join("source.mp4");
    make_film(&film).expect("the fixture film would not encode");
    let path = film.to_string_lossy().into_owned();

    // The medium the set is meant to land under — created the way a person creates one,
    // before the first build ever runs. Not essential to what T571 checks, but it is what a
    // real screen would have done first, and it costs nothing extra.
    library::media_create(&state, &id, "T571 fixture", Some("t571"))
        .await
        .expect("the medium was not created");

    // ---- the first build: nobody is watching, so it goes through unconfirmed ----
    ladder::ladder_build(&state, build_request(&id, &path, "t571", false))
        .await
        .expect("the first build was refused although nobody was watching yet");
    wait_for_master(&server, "t571", Duration::from_secs(60)).await;
    let master_before = server
        .exec_inside(&format!("cat '{VIDEO_DIR}/t571/master.m3u8'"))
        .expect("the built master.m3u8 is not on the server");

    // ---- a viewer starts pulling something being served, so there is now an open
    // connection on port 80 for `active_use::serving_connections` to see ----
    //
    // Deliberately a plain file placed directly in the serving directory rather than one of
    // the set's own segments: `serving_connections` counts open connections on 80/443 at
    // all, not connections to a particular medium (the same coarse check `media_rename` and
    // `upload_start` already live with, named in the doc comment on
    // `server::active_use::serving_connections` — T571 does not change that), so what is
    // being watched does not have to belong to the set under test at all.
    server
        .exec_inside(&format!(
            "head -c 20000000 /dev/urandom > '{VIDEO_DIR}/t571-bystander.bin'"
        ))
        .expect("the bystander file was not created");
    let viewer = Viewer::attach(&server).expect("the viewer would not attach");
    viewer
        .start_watching("/videos/t571-bystander.bin", Some("100k"))
        .expect("the watching would not start");
    viewer
        .wait_until_watching(Duration::from_secs(10))
        .expect("the viewer never began pulling");

    // ---- the rebuild, unconfirmed: refused SYNCHRONOUSLY (before a task is even
    // submitted), and nothing on the server moved ----
    let err = ladder::ladder_build(&state, build_request(&id, &path, "t571", false))
        .await
        .expect_err("a rebuild went through while somebody was watching");
    assert_eq!(err.code, ErrorCode::FileInUse);

    let master_after_refusal = server
        .exec_inside(&format!("cat '{VIDEO_DIR}/t571/master.m3u8'"))
        .expect("master.m3u8 is gone after a refused rebuild");
    assert_eq!(
        master_before, master_after_refusal,
        "master.m3u8 changed even though the rebuild was refused"
    );

    // ---- confirmed, the same rebuild goes through — even with the viewer still pulling:
    // the warning is the interface's to act on, not a bar the core enforces by itself,
    // exactly as `media_rename` already documents for the same shape of guard. Submission
    // itself succeeding (no FileInUse) is already most of the point; that the build then
    // actually ran is confirmed by master.m3u8 landing on the server again below. ----
    ladder::ladder_build(&state, build_request(&id, &path, "t571", true))
        .await
        .expect("the confirmed rebuild was refused");
    wait_for_master(&server, "t571", Duration::from_secs(60)).await;

    viewer.stop_watching().ok();
    let _ = std::fs::remove_dir_all(&film_dir);
}

#[tokio::test]
async fn a_build_with_no_active_connections_needs_no_confirmation() {
    // The other side of the guard: it must not be an unconditional bar. `media_rename`'s own
    // check makes exactly this distinction, and the point here is the same — a rebuild does
    // not deserve a hoop to jump through when there is genuinely nobody to disturb.
    let (server, state, id) = setup().await;

    let film_dir = std::env::temp_dir().join(format!(
        "vrcast-t571-quiet-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&film_dir).expect("could not make a working directory");
    let film = film_dir.join("source.mp4");
    make_film(&film).expect("the fixture film would not encode");
    let path = film.to_string_lossy().into_owned();

    ladder::ladder_build(&state, build_request(&id, &path, "t571-quiet", false))
        .await
        .expect("a build with nobody watching was refused");
    wait_for_master(&server, "t571-quiet", Duration::from_secs(60)).await;

    let _ = std::fs::remove_dir_all(&film_dir);
}
