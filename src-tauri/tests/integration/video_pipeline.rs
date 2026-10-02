//! T672 — one video goes the whole way in one place, against a real server in a container:
//! plan → «Start» → measuring → encoding the rungs → sending → cutting → checking → link,
//! and carries on after the application is killed, from the stage it was at.
//!
//! **The check at the end reaches the container.** `ladder_build` checks the set where viewers
//! get it, at the profile's domain over `https://`. The container has neither, so
//! `AppState::verify_origin` points the check at the container's own plain HTTP — the one
//! thing set here that the application never sets (the same address `seams.rs` builds by
//! hand). Everything before the check runs exactly as it does for a person.
//!
//! **The application is killed for real**, as a process of its own, as `upload_live.rs`
//! explains: destroying a runtime inside one process lets worker threads record endings a
//! killed application never would.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use vrcast_studio_lib::commands::video::{api as video, PlanSource, VideoView};
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::domain::ladder::{Quality, Rung};
use vrcast_studio_lib::domain::video::{VideoStage, VideoState};
use vrcast_studio_lib::media::ffmpeg;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::{InMemorySecretStore, SecretStore};

use super::fixture::TestServer;
use super::upload_live::{add_profile, attach_secret};

pub(crate) const VIDEO_DIR: &str = "/var/lib/vrcast/videos";

/// Where the check at the end looks for the set: the container's own HTTP.
pub(crate) fn origin_of(server: &TestServer) -> String {
    format!("http://{}:{}", server.host(), server.http_port)
}

pub(crate) fn state_on(db: &Path, secrets: Arc<dyn SecretStore>, origin: &str) -> AppState {
    let mut state = AppState::with_db(
        Arc::new(Db::open(db).expect("the database would not open")),
        secrets,
    )
    .expect("the application state would not assemble");
    state.verify_origin = Some(origin.to_owned());
    state
}

/// A real film. `seconds` long at `size`, with sound, keyframes every second.
pub(crate) fn make_film(path: &Path, size: &str, seconds: u32) {
    make_film_from(path, &format!("testsrc2=size={size}:rate=24"), seconds);
}

/// The same, with the picture from another lavfi source — another film of the same length.
pub(crate) fn make_film_from(path: &Path, picture: &str, seconds: u32) {
    let ff = ffmpeg::locate("ffmpeg").expect("no bundled FFmpeg: run `npm run ffmpeg`");
    let out = std::process::Command::new(ff)
        .args(["-nostdin", "-y", "-v", "error", "-f", "lavfi", "-i"])
        .arg(format!("{picture}:duration={seconds}"))
        .args(["-f", "lavfi", "-i"])
        .arg(format!("sine=frequency=440:duration={seconds}"))
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-b:v",
            "3000k",
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
        .expect("could not run the bundled FFmpeg");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A directory that removes itself.
pub(crate) struct Scratch(pub(crate) PathBuf);

impl Scratch {
    pub(crate) fn new(what: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("vrcast-{what}-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).expect("could not make a working directory");
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(crate) async fn until(
    state: &AppState,
    id: &str,
    what: &str,
    limit: Duration,
    ok: impl Fn(&VideoView) -> bool,
) -> VideoView {
    let deadline = Instant::now() + limit;
    loop {
        let now = video::video_get(state, id).expect("the video went missing");
        if ok(&now) {
            return now;
        }
        assert!(
            Instant::now() < deadline,
            "{what}: never got there; the video is {:?} at {:?}, problem {:?}",
            now.state,
            now.stage,
            now.problem
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Two rungs, already measured — for the restart checks, where measuring would only make
/// the window to kill in harder to hit. The whole way with a real measurement is
/// [`a_video_goes_the_whole_way_from_a_plan_to_a_link`].
fn two_rungs() -> Vec<Rung> {
    let rung = |index, bitrate_bps: u64, width, height, vmaf| Rung {
        index,
        bitrate_bps,
        maxrate_bps: bitrate_bps + bitrate_bps / 10,
        bufsize_bps: bitrate_bps + bitrate_bps / 10,
        width,
        height,
        level: String::from("3.1"),
        reasons: Vec::new(),
        quality: Quality::MeasuredHere { vmaf_x100: vmaf },
    };
    vec![
        rung(0, 2_000_000, 1280, 720, 9300),
        rung(1, 1_000_000, 640, 360, 8900),
    ]
}

pub(crate) fn the_set_is_served(server: &TestServer, slug: &str) {
    let master = server
        .exec_inside(&format!("cat '{VIDEO_DIR}/{slug}/master.m3u8'"))
        .expect("master.m3u8 is not on the server");
    assert!(master.contains("#EXT-X-STREAM-INF"), "{master}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_video_goes_the_whole_way_from_a_plan_to_a_link() {
    super::fixture::logging_if_requested();
    let server = TestServer::start().expect("the container would not come up");
    let scratch = Scratch::new("t672-whole");
    let db = scratch.0.join("vrcast.sqlite3");
    let state = state_on(
        &db,
        Arc::new(InMemorySecretStore::new()),
        &origin_of(&server),
    );
    let server_id = add_profile(&state, &server).await;
    let film = scratch.0.join("Whole Way.mp4");
    make_film(&film, "640x360", 15);

    let mut events = state.subscribe();
    let added = video::video_add(
        &state,
        &server_id,
        &[film.to_string_lossy().into_owned()],
        None,
    )
    .await
    .expect("adding failed");
    assert!(added.refused.is_empty(), "{:?}", added.refused);
    let id = added.added[0].id.clone();

    // ---- the plan, before «Start»: the server was asked this time ----
    let ready = until(&state, &id, "the plan", Duration::from_secs(120), |v| {
        v.state != VideoState::Planning
    })
    .await;
    assert_eq!(ready.state, VideoState::Ready, "{:?}", ready.problem);
    let plan = ready.plan.clone().expect("no plan");
    assert_eq!(plan.from, PlanSource::Formula);
    assert!(plan.needs_measuring);
    assert!(
        matches!(
            plan.server_space,
            vrcast_studio_lib::commands::video::SpaceCheck::Fits { .. }
        ),
        "{:?}",
        plan.server_space
    );
    assert_eq!(plan.name_taken, Some(false));

    // ---- «Start»: measuring, then the build the measurement chains on to ----
    let started = video::video_start(&state, std::slice::from_ref(&id));
    assert!(started[0].error.is_none(), "{:?}", started[0].error);

    let done = until(
        &state,
        &id,
        "the whole way",
        Duration::from_secs(540),
        |v| {
            matches!(
                v.state,
                VideoState::Done | VideoState::Problem | VideoState::Cancelled
            )
        },
    )
    .await;
    assert_eq!(done.state, VideoState::Done, "{:?}", done.problem);
    assert_eq!(done.stage, VideoStage::Done);
    assert!(done.media_id.is_some(), "the set belongs to no medium");
    let link = done.link.expect("a finished video has no link");
    assert!(
        link.origin
            .ends_with(&format!("/videos/{}/master.m3u8", done.slug)),
        "{}",
        link.origin
    );
    the_set_is_served(&server, &done.slug);
    // The plan now says what was measured.
    assert_eq!(done.plan.expect("the plan went").from, PlanSource::Measured);

    // ---- every stage was shown, in order ----
    let mut stages = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let vrcast_studio_lib::commands::AppEvent::VideoUpdate(v) = event {
            if v.id == id && stages.last() != Some(&v.stage) {
                stages.push(v.stage);
            }
        }
    }
    // Inside the build encoding and sending alternate rung by rung (the contract says so);
    // apart from that the stage never goes back.
    let order = |s: &VideoStage| match s {
        VideoStage::Uploading => VideoStage::Encoding,
        other => *other,
    };
    for pair in stages.windows(2) {
        assert!(
            order(&pair[0]) <= order(&pair[1]),
            "the stage went back: {stages:?}"
        );
    }
    for wanted in [
        VideoStage::Measuring,
        VideoStage::Encoding,
        VideoStage::Uploading,
        VideoStage::Cutting,
        VideoStage::Done,
    ] {
        assert!(
            stages.contains(&wanted),
            "{wanted:?} was never shown: {stages:?}"
        );
    }

    // ---- the same file again is a second video, and the taken name is a choice ----
    let again = video::video_add(
        &state,
        &server_id,
        &[film.to_string_lossy().into_owned()],
        None,
    )
    .await
    .unwrap();
    let second = again.added[0].id.clone();
    let ready = until(
        &state,
        &second,
        "the second plan",
        Duration::from_secs(120),
        |v| v.state != VideoState::Planning,
    )
    .await;
    assert_eq!(ready.plan.unwrap().name_taken, Some(true));
    video::video_start(&state, std::slice::from_ref(&second));
    let stopped = until(
        &state,
        &second,
        "the taken name",
        Duration::from_secs(120),
        |v| v.state == VideoState::Problem,
    )
    .await;
    let problem = stopped.problem.unwrap();
    assert_eq!(
        problem.error.code,
        vrcast_studio_lib::commands::error::ErrorCode::SlugTaken
    );
    use vrcast_studio_lib::domain::video::VideoAction;
    assert_eq!(
        problem.actions,
        vec![VideoAction::Replace, VideoAction::Rename]
    );
    // «Replace»: build into the medium that has the name. Its old set is removed first and
    // built again whole (T676); the medium stays the same one.
    video::video_replace(&state, &second, false)
        .await
        .expect("replace was refused");
    let replaced = until(
        &state,
        &second,
        "the replace",
        Duration::from_secs(300),
        |v| matches!(v.state, VideoState::Done | VideoState::Problem),
    )
    .await;
    assert_eq!(replaced.state, VideoState::Done, "{:?}", replaced.problem);
    assert_eq!(replaced.media_id, done.media_id);
}

// ---------- «Replace» under a taken name (T676) ----------

/// What a file on the server is, by its bytes.
fn digest(server: &TestServer, path: &str) -> String {
    server
        .exec_inside(&format!("md5sum '{path}' | cut -d' ' -f1"))
        .unwrap_or_else(|e| panic!("{path} is not on the server: {e}"))
        .trim()
        .to_owned()
}

/// Every file of the set `slug` on the server, by its bytes: the prepared rungs, the cut
/// segments, their playlists and the master.
pub(crate) fn the_set(server: &TestServer, slug: &str) -> Vec<(String, String)> {
    let names = server
        .exec_inside(&format!(
            "cd '{VIDEO_DIR}' && ls -1 {slug}_*.mp4 && find '{slug}' -type f \\( -name '*.ts' -o -name '*.m3u8' \\) | sort"
        ))
        .expect("the set could not be listed");
    names
        .lines()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(|n| (n.to_owned(), digest(server, &format!("{VIDEO_DIR}/{n}"))))
        .collect()
}

/// Add a film, give it the two measured rungs, start it, and wait until it stops.
pub(crate) async fn build_one(state: &AppState, server_id: &str, film: &Path) -> VideoView {
    let added = video::video_add(
        state,
        server_id,
        &[film.to_string_lossy().into_owned()],
        None,
    )
    .await
    .expect("adding failed");
    assert!(added.refused.is_empty(), "{:?}", added.refused);
    let id = added.added[0].id.clone();
    let planned = until(state, &id, "the plan", Duration::from_secs(120), |v| {
        v.state != VideoState::Planning
    })
    .await;
    assert_eq!(planned.state, VideoState::Ready, "{:?}", planned.problem);
    video::video_set_rungs(state, &id, Some(two_rungs())).unwrap();
    let started = video::video_start(state, std::slice::from_ref(&id));
    assert!(started[0].error.is_none(), "{:?}", started[0].error);
    until(state, &id, "the build", Duration::from_secs(400), |v| {
        matches!(
            v.state,
            VideoState::Done | VideoState::Problem | VideoState::Cancelled
        )
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replace_under_a_taken_name_builds_every_rung_anew_for_another_film_of_the_same_length() {
    super::fixture::logging_if_requested();
    let server = TestServer::start().expect("the container would not come up");
    let scratch = Scratch::new("t676-replace");
    let state = state_on(
        &scratch.0.join("vrcast.sqlite3"),
        Arc::new(InMemorySecretStore::new()),
        &origin_of(&server),
    );
    let server_id = add_profile(&state, &server).await;

    // Two films of the same length and the same name, in two folders: what `ladder_build`
    // would take for each other by length alone.
    std::fs::create_dir_all(scratch.0.join("a")).unwrap();
    std::fs::create_dir_all(scratch.0.join("b")).unwrap();
    let first = scratch.0.join("a").join("Same Name.mp4");
    let second = scratch.0.join("b").join("Same Name.mp4");
    make_film(&first, "1280x720", 12);
    make_film_from(&second, "testsrc=size=1280x720:rate=24", 12);

    let done = build_one(&state, &server_id, &first).await;
    assert_eq!(done.state, VideoState::Done, "{:?}", done.problem);
    let slug = done.slug.clone();
    let before = the_set(&server, &slug);
    assert!(
        before.iter().any(|(n, _)| n.ends_with("_2.mp4"))
            && before.iter().any(|(n, _)| n.ends_with("master.m3u8")),
        "{before:?}"
    );

    // The other film under the same name stops on the name, with «Replace» to press.
    let stopped = build_one(&state, &server_id, &second).await;
    assert_eq!(stopped.slug, slug);
    assert_eq!(stopped.state, VideoState::Problem);
    assert_eq!(
        stopped.problem.as_ref().unwrap().error.code,
        vrcast_studio_lib::commands::error::ErrorCode::SlugTaken
    );
    // Nothing of the first set was touched by stopping.
    assert_eq!(the_set(&server, &slug), before);

    video::video_replace(&state, &stopped.id, false)
        .await
        .expect("replace was refused");
    let replaced = until(
        &state,
        &stopped.id,
        "the replace",
        Duration::from_secs(400),
        |v| matches!(v.state, VideoState::Done | VideoState::Problem),
    )
    .await;
    assert_eq!(replaced.state, VideoState::Done, "{:?}", replaced.problem);
    assert_eq!(replaced.media_id, done.media_id, "not the same medium");
    the_set_is_served(&server, &slug);

    // **Every rung is new**: no prepared file, no segment and no playlist of the first film
    // is left under the name — each was made again from the second film.
    let after = the_set(&server, &slug);
    let rungs_after: Vec<&String> = after
        .iter()
        .filter(|(n, _)| n.ends_with(".mp4"))
        .map(|(n, _)| n)
        .collect();
    assert_eq!(rungs_after.len(), 2, "{after:?}");
    for (name, old) in &before {
        if let Some((_, new)) = after.iter().find(|(n, _)| n == name) {
            if name.ends_with(".ts") || name.ends_with(".mp4") {
                assert_ne!(new, old, "{name} is still the first film's");
            }
        }
    }
    for (name, _) in after.iter().filter(|(n, _)| n.ends_with(".mp4")) {
        let had = before.iter().find(|(n, _)| n == name).map(|(_, d)| d);
        assert!(had.is_some(), "{name} was not there before");
    }
    drop(scratch);
}

// ---------- «Build a set» for a medium already in the library (T675) ----------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_set_is_built_into_a_medium_beside_its_single_file_named_like_a_rung_which_stays_byte_for_byte(
) {
    use vrcast_studio_lib::commands::error::ErrorCode;
    use vrcast_studio_lib::commands::library::api as library;

    super::fixture::logging_if_requested();
    let server = TestServer::start().expect("the container would not come up");
    let scratch = Scratch::new("t675-into");
    let state = state_on(
        &scratch.0.join("vrcast.sqlite3"),
        Arc::new(InMemorySecretStore::new()),
        &origin_of(&server),
    );
    let server_id = add_profile(&state, &server).await;

    // A medium as the shell script left them: one file, named the way a rung's would be.
    let medium = library::media_create(&state, &server_id, "Old Film", Some("old-film"))
        .await
        .expect("the medium was not made");
    server
        .exec_inside(&format!(
            "head -c 300000 /dev/urandom > '{VIDEO_DIR}/old-film_9.mp4'"
        ))
        .unwrap();
    library::file_move(&state, &server_id, "old-film_9.mp4", &medium, true)
        .await
        .expect("the file was not filed under the medium");
    let single = digest(&server, &format!("{VIDEO_DIR}/old-film_9.mp4"));

    let film = scratch.0.join("anything.mp4");
    make_film(&film, "1280x720", 12);
    let path = film.to_string_lossy().into_owned();

    // More than one file for a medium: refused whole.
    let err = video::video_add(
        &state,
        &server_id,
        &[path.clone(), path.clone()],
        Some(&medium),
    )
    .await
    .expect_err("two files were taken for one medium");
    assert_eq!(err.code, ErrorCode::InvalidInput);

    // **A rung whose prepared file would be the medium's own file**: 9 Mbit/s is
    // `old-film_9.mp4`. Not a refusal (T677, the owner's decision of 2026-10-02): the rung
    // is made under the next free name, and the medium's file is not touched.
    let added = video::video_add(
        &state,
        &server_id,
        std::slice::from_ref(&path),
        Some(&medium),
    )
    .await
    .expect("the medium did not take the film");
    let into = added.added[0].clone();
    assert_eq!(into.slug, "old-film");
    assert_eq!(into.title, "Old Film");
    assert_eq!(into.media_id.as_deref(), Some(medium.as_str()));
    assert!(into.problem.is_none(), "{:?}", into.problem);
    until(
        &state,
        &into.id,
        "the plan",
        Duration::from_secs(120),
        |v| v.state != VideoState::Planning,
    )
    .await;

    // While it is on the list unfinished, the medium does not take a second film.
    let err = video::video_add(
        &state,
        &server_id,
        std::slice::from_ref(&path),
        Some(&medium),
    )
    .await
    .expect_err("a second film was taken for a medium already on its way");
    assert_eq!(err.code, ErrorCode::MediaSetInWork);

    let mut nine = two_rungs();
    nine[0].bitrate_bps = 9_000_000;
    nine[0].maxrate_bps = 9_900_000;
    nine[0].bufsize_bps = 9_900_000;
    video::video_set_rungs(&state, &into.id, Some(nine.clone())).unwrap();
    video::video_start(&state, std::slice::from_ref(&into.id));
    let done = until(&state, &into.id, "the set", Duration::from_secs(400), |v| {
        matches!(v.state, VideoState::Done | VideoState::Problem)
    })
    .await;
    assert_eq!(done.state, VideoState::Done, "{:?}", done.problem);
    assert_eq!(done.media_id.as_deref(), Some(medium.as_str()));
    assert!(done
        .link
        .unwrap()
        .origin
        .ends_with("/videos/old-film/master.m3u8"));

    // The single file is still there, byte for byte, and still the medium's.
    assert_eq!(
        digest(&server, &format!("{VIDEO_DIR}/old-film_9.mp4")),
        single
    );
    // The rung went under the next free name, and the set says so for carrying on.
    let renamed = format!("{VIDEO_DIR}/old-film_9v.mp4");
    let made = identity(&server, &renamed).expect("the rung was not made as old-film_9v.mp4");
    let record = server
        .exec_inside(&format!("cat '{VIDEO_DIR}/old-film/.prepared'"))
        .expect("the set has no record of its prepared files");
    assert!(record.contains("v9=old-film_9v.mp4"), "{record}");
    assert!(record.contains("v1=old-film_1.mp4"), "{record}");

    // **The set is served by its master.m3u8**, which names the rungs' own playlists — the
    // check a viewer's player would make, every rung, every first segment.
    the_set_is_served(&server, "old-film");
    let master = format!("{}/videos/old-film/master.m3u8", origin_of(&server));
    let verdict = vrcast_studio_lib::server::hls_verify::verify(&master, 2)
        .await
        .expect("the set could not be asked for");
    assert!(verdict.ok(), "{:?}", verdict.broken());
    assert_eq!(verdict.variants_in_master, 2);

    // The library: the medium has its file and its set; the renamed rung is not taken for
    // the medium's file. Both rungs are the set's own (T678), not «not recognised».
    let view = library::library_list(&state, &server_id, true)
        .await
        .unwrap();
    let m = view.media.iter().find(|m| m.id == medium).unwrap();
    assert_eq!(
        m.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
        vec!["old-film_9.mp4"],
        "{m:?}"
    );
    assert!(
        m.ladders.iter().any(|l| l.path == "old-film/master.m3u8"),
        "{m:?}"
    );
    assert_eq!(
        m.set_files
            .iter()
            .map(|f| f.path.as_str())
            .collect::<Vec<_>>(),
        vec!["old-film_1.mp4", "old-film_9v.mp4"],
        "{m:?}"
    );
    let loose = |name: &str| view.unrecognized.iter().any(|f| f.path == name);
    assert!(
        !loose("old-film_9v.mp4") && !loose("old-film_1.mp4"),
        "a rung of the set is shown as not recognised: {:?}",
        view.unrecognized
    );
    eprintln!(
        "T677 unrecognised after the build: {:?}",
        view.unrecognized
            .iter()
            .map(|f| f.path.as_str())
            .collect::<Vec<_>>()
    );

    // Now it has a set: another one is refused.
    let err = video::video_add(
        &state,
        &server_id,
        std::slice::from_ref(&path),
        Some(&medium),
    )
    .await
    .expect_err("a second set was taken for a medium that has one");
    assert_eq!(err.code, ErrorCode::MediaHasSet);

    // **Carrying on finds the rung under the name it was given**, even once the medium's
    // file is gone and the first name is free again: built again, the 9 Mbit/s rung is found
    // done as old-film_9v.mp4 — not made a second time as old-film_9.mp4.
    library::file_delete(&state, &server_id, "old-film_9.mp4", true)
        .await
        .expect("the single file was not deleted");
    let task = vrcast_studio_lib::commands::ladder::api::ladder_build(
        &state,
        vrcast_studio_lib::commands::ladder::BuildRequest {
            server_id: server_id.clone(),
            path: path.clone(),
            slug: String::from("old-film"),
            rungs: nine,
            audio_track: 0,
            prefer_hardware: true,
            batch: None,
            confirmed: true,
        },
    )
    .await
    .expect("the rebuild was refused");
    let deadline = Instant::now() + Duration::from_secs(300);
    let ended = loop {
        let t = state.tasks.get(&task).unwrap().unwrap();
        if t.state.is_final() {
            break t;
        }
        assert!(Instant::now() < deadline, "the rebuild never ended");
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    assert_eq!(
        ended.state,
        vrcast_studio_lib::tasks::state::TaskState::Completed,
        "{:?}",
        ended.error
    );
    assert_eq!(
        identity(&server, &renamed).as_deref(),
        Some(made.as_str()),
        "the renamed rung was made again"
    );
    assert!(
        identity(&server, &format!("{VIDEO_DIR}/old-film_9.mp4")).is_none(),
        "the rung was made again under its first name"
    );

    // **Deleting the medium** takes its set with it — the directory, the record inside it,
    // and the set's prepared rung files (T678) — and nothing that is not the medium's: another
    // medium's single file, and a loose file named like a rung the set does not serve.
    let other = library::media_create(&state, &server_id, "Other", Some("other"))
        .await
        .expect("the other medium was not made");
    server
        .exec_inside(&format!(
            "head -c 1000 /dev/urandom > '{VIDEO_DIR}/other_9.mp4' && \
             head -c 1000 /dev/urandom > '{VIDEO_DIR}/old-film_7.mp4'"
        ))
        .unwrap();
    library::file_move(&state, &server_id, "other_9.mp4", &other, true)
        .await
        .expect("the other medium's file was not filed");
    let others = digest(&server, &format!("{VIDEO_DIR}/other_9.mp4"));
    let loose_one = digest(&server, &format!("{VIDEO_DIR}/old-film_7.mp4"));

    // The confirmation names them.
    let asked = library::media_delete(&state, &server_id, &medium, false)
        .await
        .expect_err("deleted without confirmation");
    let named = asked
        .details
        .iter()
        .find(|d| d.key.as_str() == "CONFIRM_DELETE_SET_FILES")
        .unwrap_or_else(|| panic!("the set's rung files are not named: {asked:?}"));
    assert_eq!(
        named.params.get("names").and_then(|v| v.as_str()),
        Some("old-film_1.mp4, old-film_9v.mp4"),
        "{asked:?}"
    );

    library::media_delete(&state, &server_id, &medium, true)
        .await
        .expect("the medium was not deleted");
    assert!(
        server
            .exec_inside(&format!("test ! -e '{VIDEO_DIR}/old-film'"))
            .is_ok(),
        "the set's directory outlived its medium"
    );
    let left = server
        .exec_inside(&format!("cd '{VIDEO_DIR}' && ls -1"))
        .unwrap_or_default();
    eprintln!("T678 left on the server after deleting the medium: {left:?}");
    for gone in ["old-film_1.mp4", "old-film_9v.mp4", "old-film_9.mp4"] {
        assert!(
            !left.lines().any(|l| l.trim() == gone),
            "{gone} outlived its medium: {left:?}"
        );
    }
    assert_eq!(digest(&server, &format!("{VIDEO_DIR}/other_9.mp4")), others);
    assert_eq!(
        digest(&server, &format!("{VIDEO_DIR}/old-film_7.mp4")),
        loose_one
    );
    let view = library::library_list(&state, &server_id, true)
        .await
        .unwrap();
    assert!(
        view.unrecognized.iter().any(|f| f.path == "old-film_7.mp4"),
        "{:?}",
        view.unrecognized
    );
    drop(scratch);
}

// ---------- a medium's single file of the same length is not a rung (T681, QA-25 №2) ----------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_single_file_of_the_same_length_named_like_a_rung_stays_and_the_rung_is_made_anew() {
    use vrcast_studio_lib::commands::library::api as library;

    super::fixture::logging_if_requested();
    let server = TestServer::start().expect("the container would not come up");
    let scratch = Scratch::new("t681-same-length");
    let state = state_on(
        &scratch.0.join("vrcast.sqlite3"),
        Arc::new(InMemorySecretStore::new()),
        &origin_of(&server),
    );
    let server_id = add_profile(&state, &server).await;

    // Another film of exactly the same length, 720p like the top rung, lying on the server as
    // the medium's single file under the name the 2 Mbit/s rung would take: the one
    // `variant_already_there` took for the rung by its length alone.
    let medium = library::media_create(&state, &server_id, "Same", Some("same"))
        .await
        .expect("the medium was not made");
    let theirs = scratch.0.join("theirs.mp4");
    make_film_from(&theirs, "testsrc=size=1280x720:rate=24", 12);
    server
        .put_file(&theirs, &format!("{VIDEO_DIR}/same_2.mp4"))
        .expect("the single file was not put on the server");
    library::file_move(&state, &server_id, "same_2.mp4", &medium, true)
        .await
        .expect("the file was not filed under the medium");
    let single = digest(&server, &format!("{VIDEO_DIR}/same_2.mp4"));

    let film = scratch.0.join("ours.mp4");
    make_film(&film, "1280x720", 12);
    let added = video::video_add(
        &state,
        &server_id,
        &[film.to_string_lossy().into_owned()],
        Some(&medium),
    )
    .await
    .expect("the medium did not take the film");
    let id = added.added[0].id.clone();
    until(&state, &id, "the plan", Duration::from_secs(120), |v| {
        v.state != VideoState::Planning
    })
    .await;
    video::video_set_rungs(&state, &id, Some(two_rungs())).unwrap();
    video::video_start(&state, std::slice::from_ref(&id));
    let done = until(&state, &id, "the set", Duration::from_secs(400), |v| {
        matches!(v.state, VideoState::Done | VideoState::Problem)
    })
    .await;
    assert_eq!(done.state, VideoState::Done, "{:?}", done.problem);

    // The single file is untouched and still the medium's own.
    assert_eq!(digest(&server, &format!("{VIDEO_DIR}/same_2.mp4")), single);
    // The rung was made from this film, under the next free name.
    let made = identity(&server, &format!("{VIDEO_DIR}/same_2v.mp4"));
    assert!(made.is_some(), "the rung was not made as same_2v.mp4");
    let record = server
        .exec_inside(&format!("cat '{VIDEO_DIR}/same/.prepared'"))
        .expect("the set has no record of its prepared files");
    assert!(record.contains("v2=same_2v.mp4"), "{record}");
    assert!(record.contains("made v2 same_2v.mp4"), "{record}");
    the_set_is_served(&server, "same");
    let view = library::library_list(&state, &server_id, true)
        .await
        .unwrap();
    let m = view.media.iter().find(|m| m.id == medium).unwrap();
    assert_eq!(
        m.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
        vec!["same_2.mp4"]
    );
    assert!(m.set_files.iter().any(|f| f.path == "same_2v.mp4"), "{m:?}");
    drop(scratch);
}

// ---------- killed, and carried on ----------

mod env_names {
    pub const DB: &str = "VRCAST_T672_DB";
    pub const FILM: &str = "VRCAST_T672_FILM";
    pub const ORIGIN: &str = "VRCAST_T672_ORIGIN";
    pub const PAUSE: &str = "VRCAST_T672_PAUSE";
    pub const MARK: &str = "VRCAST_T672_MARK";
}

const HELPER: &str = "video_pipeline::the_first_run_of_a_video_that_gets_killed";

/// The first run — the one that gets killed. Not a check on its own: started only by the
/// two below, as a process of its own. With no conditions in the environment it does
/// nothing.
#[test]
#[ignore = "half of the restart checks: started as a process of its own"]
fn the_first_run_of_a_video_that_gets_killed() {
    let (Ok(db), Ok(film), Ok(origin), Ok(mark)) = (
        std::env::var(env_names::DB),
        std::env::var(env_names::FILM),
        std::env::var(env_names::ORIGIN),
        std::env::var(env_names::MARK),
    ) else {
        return;
    };
    let pause = std::env::var(env_names::PAUSE).is_ok();
    let rt = tokio::runtime::Runtime::new().expect("the runtime would not be created");
    rt.block_on(async move {
        let state = state_on(
            Path::new(&db),
            Arc::new(InMemorySecretStore::new()),
            &origin,
        );
        let server_id = attach_secret(&state);
        let added = video::video_add(&state, &server_id, &[film], None)
            .await
            .unwrap();
        let id = added.added[0].id.clone();
        until(&state, &id, "the plan", Duration::from_secs(120), |v| {
            v.state == VideoState::Ready
        })
        .await;
        video::video_set_rungs(&state, &id, Some(two_rungs())).unwrap();
        let started = video::video_start(&state, std::slice::from_ref(&id));
        assert!(started[0].error.is_none(), "{:?}", started[0].error);

        if pause {
            // Paused by a person once the second rung is in hand: the first is on the
            // server by then.
            until(
                &state,
                &id,
                "the second rung",
                Duration::from_secs(300),
                |v| v.progress.as_ref().and_then(|p| p.rung).unwrap_or(0) >= 2,
            )
            .await;
            video::video_pause(&state, &id).unwrap();
        }
        std::fs::write(&mark, &id).unwrap();
        loop {
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    });
}

struct Killable(std::process::Child);

impl Drop for Killable {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// What both restart checks share: a server, a profile, a film, and a first run that has got
/// as far as the first rung on the server — then is killed. Returns the video's id.
struct Killed {
    server: TestServer,
    scratch: Scratch,
    db: PathBuf,
    secrets: Arc<dyn SecretStore>,
    id: String,
    /// What the first rung's file on the server was when the application died.
    first_rung: String,
}

fn first_rung_file(slug: &str) -> String {
    format!("{VIDEO_DIR}/{slug}_2.mp4")
}

fn identity(server: &TestServer, path: &str) -> Option<String> {
    server
        .exec_inside(&format!("stat -c '%i %y %s' '{path}'"))
        .ok()
        .map(|s| s.trim().to_owned())
}

async fn run_and_kill(pause: bool) -> Killed {
    super::fixture::logging_if_requested();
    let server = TestServer::start().expect("the container would not come up");
    let scratch = Scratch::new("t672-restart");
    let db = scratch.0.join("vrcast.sqlite3");
    let secrets: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    {
        let state = state_on(&db, secrets.clone(), &origin_of(&server));
        add_profile(&state, &server).await;
    }
    // Long enough that the second rung is still being made when the first is on the server.
    let film = scratch.0.join("carry on.mp4");
    make_film(&film, "1280x720", 90);
    let slug = "carry-on";
    let mark = scratch.0.join("started");

    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([HELPER, "--exact", "--ignored", "--test-threads=1"])
        .env(env_names::DB, &db)
        .env(env_names::FILM, &film)
        .env(env_names::ORIGIN, origin_of(&server))
        .env(env_names::MARK, &mark)
        .envs(pause.then_some((env_names::PAUSE, "1")))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("the first run did not start");
    let mut running = Killable(child);

    // The first rung whole on the server (and, when pausing, the pause pressed).
    let deadline = Instant::now() + Duration::from_secs(400);
    let first_rung = loop {
        if let (Some(there), true) = (identity(&server, &first_rung_file(slug)), mark.exists()) {
            break there;
        }
        if let Ok(Some(status)) = running.0.try_wait() {
            panic!("the first run ended by itself ({status})");
        }
        assert!(
            Instant::now() < deadline,
            "the first rung never reached the server"
        );
        std::thread::sleep(Duration::from_millis(200));
    };
    // The application dies — without warning, without a record of how it ended.
    drop(running);
    let id = std::fs::read_to_string(&mark).unwrap();
    Killed {
        server,
        scratch,
        db,
        secrets,
        id,
        first_rung,
    }
}

fn start_again(killed: &Killed) -> AppState {
    let state = state_on(
        &killed.db,
        killed.secrets.clone(),
        &origin_of(&killed.server),
    );
    attach_secret(&state);
    state
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_video_killed_mid_build_carries_on_by_itself_from_its_stage() {
    let killed = run_and_kill(false).await;
    let state = start_again(&killed);

    let before = video::video_get(&state, &killed.id).unwrap();
    assert_eq!(
        before.state,
        VideoState::Working,
        "it was not going when killed"
    );
    assert!(before.stage >= VideoStage::Encoding, "{:?}", before.stage);
    assert_ne!(
        before.stage,
        VideoStage::Done,
        "it finished before it could be killed"
    );

    // What the application does at start-up — nobody presses anything.
    assert_eq!(video::restore_videos(&state).unwrap(), 1);
    let done = until(
        &state,
        &killed.id,
        "carrying on",
        Duration::from_secs(480),
        |v| {
            matches!(
                v.state,
                VideoState::Done | VideoState::Problem | VideoState::Cancelled
            )
        },
    )
    .await;
    assert_eq!(done.state, VideoState::Done, "{:?}", done.problem);
    assert!(done.link.is_some());
    the_set_is_served(&killed.server, &done.slug);
    // The rung already on the server was found done, not made and sent again.
    assert_eq!(
        identity(&killed.server, &first_rung_file(&done.slug)).as_deref(),
        Some(killed.first_rung.as_str()),
        "the first rung was sent again after the restart"
    );
    drop(killed.scratch);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pause_a_person_pressed_is_still_a_pause_after_a_restart() {
    let killed = run_and_kill(true).await;
    let state = start_again(&killed);

    assert_eq!(
        video::restore_videos(&state).unwrap(),
        0,
        "a paused video was carried on"
    );
    let paused = video::video_get(&state, &killed.id).unwrap();
    assert_eq!(paused.state, VideoState::Paused);
    assert!(paused.paused_by_person);
    assert!(paused.stage >= VideoStage::Encoding);

    // Nothing moves while it is paused.
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(
        video::video_get(&state, &killed.id).unwrap().state,
        VideoState::Paused
    );
    assert!(
        state
            .tasks
            .list()
            .unwrap()
            .iter()
            .all(|t| t.state.is_final() || !state.tasks.is_alive(&t.id)),
        "work was started for a paused video"
    );

    // «Continue» carries on from where it was.
    video::video_resume(&state, &killed.id).expect("continue was refused");
    let done = until(
        &state,
        &killed.id,
        "continuing",
        Duration::from_secs(480),
        |v| {
            matches!(
                v.state,
                VideoState::Done | VideoState::Problem | VideoState::Cancelled
            )
        },
    )
    .await;
    assert_eq!(done.state, VideoState::Done, "{:?}", done.problem);
    the_set_is_served(&killed.server, &done.slug);
    assert_eq!(
        identity(&killed.server, &first_rung_file(&done.slug)).as_deref(),
        Some(killed.first_rung.as_str()),
        "the first rung was sent again after continuing"
    );
    drop(killed.scratch);
}
// ---------- found here: «Start» on several videos at once ----------

/// Two measured rungs at 360p — enough to get past making the medium, which is what this is
/// about, without a measurement.
fn two_small_rungs() -> Vec<Rung> {
    let rung = |index, bitrate_bps: u64, width, height| Rung {
        index,
        bitrate_bps,
        maxrate_bps: bitrate_bps + bitrate_bps / 10,
        bufsize_bps: bitrate_bps + bitrate_bps / 10,
        width,
        height,
        level: String::from("3.1"),
        reasons: Vec::new(),
        quality: Quality::MeasuredHere { vmaf_x100: 9000 },
    };
    vec![rung(0, 1_000_000, 640, 360), rung(1, 500_000, 426, 240)]
}

/// Found by the acceptance run (2026-10-02): «Start» pressed on several videos at once — the
/// screen's own way of starting a selection — made every medium at the same moment. Each
/// `media_create` read the catalogue at the same generation, the first write won, and the
/// rest stopped on `MANIFEST_CONFLICT` with «Retry», although nothing was wrong. Each video
/// must get its medium and go on.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn several_videos_started_at_once_each_get_their_medium() {
    super::fixture::logging_if_requested();
    let server = TestServer::start().expect("the container would not come up");
    let scratch = Scratch::new("t672-start-many");
    let state = state_on(
        &scratch.0.join("vrcast.sqlite3"),
        Arc::new(InMemorySecretStore::new()),
        &origin_of(&server),
    );
    let server_id = add_profile(&state, &server).await;
    let names = ["Many One", "Many Two", "Many Three"];
    let mut paths = Vec::new();
    for n in names {
        let p = scratch.0.join(format!("{n}.mp4"));
        make_film(&p, "1280x720", 8);
        paths.push(p.to_string_lossy().into_owned());
    }
    let added = video::video_add(&state, &server_id, &paths, None)
        .await
        .unwrap();
    assert!(added.refused.is_empty(), "{:?}", added.refused);
    let ids: Vec<String> = added.added.iter().map(|v| v.id.clone()).collect();
    for id in &ids {
        until(&state, id, "the plan", Duration::from_secs(120), |v| {
            v.state == VideoState::Ready
        })
        .await;
        video::video_set_rungs(&state, id, Some(two_small_rungs())).unwrap();
    }
    let started = video::video_start(&state, &ids);
    assert!(started.iter().all(|s| s.error.is_none()), "{started:?}");
    for id in &ids {
        let v = until(&state, id, "the medium", Duration::from_secs(120), |v| {
            v.media_id.is_some() || v.state == VideoState::Problem
        })
        .await;
        assert!(
            v.problem.is_none(),
            "{} stopped on {:?}",
            v.title,
            v.problem.map(|p| p.error)
        );
    }
    for id in &ids {
        let v = until(&state, id, "done", Duration::from_secs(300), |v| {
            matches!(
                v.state,
                VideoState::Done | VideoState::Problem | VideoState::Cancelled
            )
        })
        .await;
        assert_eq!(v.state, VideoState::Done, "{} {:?}", v.title, v.problem);
    }
    // And every set is attached to its own medium in the catalogue.
    let view = vrcast_studio_lib::commands::library::api::library_list(&state, &server_id, true)
        .await
        .unwrap();
    for id in &ids {
        let v = video::video_get(&state, id).unwrap();
        let m = view
            .media
            .iter()
            .find(|m| Some(&m.id) == v.media_id.as_ref())
            .unwrap_or_else(|| panic!("{} has no medium in the library", v.title));
        assert!(
            m.ladders
                .iter()
                .any(|l| l.path == format!("{}/master.m3u8", v.slug)),
            "{}'s set is not attached to its medium: {m:?}",
            v.title
        );
    }
}
