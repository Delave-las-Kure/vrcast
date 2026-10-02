//! T672 — the commands of a video in work, through the real core with a real film and a
//! server that does not answer.
//!
//! What can be checked without a server is checked here: each file taken or refused on its
//! own, the plan that comes back, what each state allows, a problem with its actions instead
//! of a stuck card, the event that carries the whole video, and what a restart does with each
//! state. The whole way to «done» against a real server is `tests/integration/video_pipeline.rs`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use vrcast_studio_lib::commands::error::{DetailCode, ErrorCode};
use vrcast_studio_lib::commands::servers::api as servers;
use vrcast_studio_lib::commands::video::{api as video, PlanSource, SpaceCheck, VideoView};
use vrcast_studio_lib::commands::{AppEvent, AppState};
use vrcast_studio_lib::domain::ladder::{Quality, Rung};
use vrcast_studio_lib::domain::server_profile::AuthKind;
use vrcast_studio_lib::domain::video::{VideoAction, VideoStage, VideoState};
use vrcast_studio_lib::media::ffmpeg;
use vrcast_studio_lib::store::videos::{self as rows, VideoRow};

use super::support::{state, valid_input};

/// A directory of films that removes itself.
struct Films(PathBuf);

impl Films {
    fn new() -> Self {
        let dir =
            std::env::temp_dir().join(format!("vrcast-t672-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    /// A short real film, with sound unless told otherwise. `None` when there is no bundled
    /// FFmpeg to make it with.
    fn film(&self, name: &str, with_audio: bool) -> Option<String> {
        let ff = ffmpeg::locate("ffmpeg").ok()?;
        let out = self.0.join(name);
        let mut cmd = std::process::Command::new(ff);
        cmd.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=1280x720:rate=24:duration=3",
        ]);
        if with_audio {
            cmd.args(["-f", "lavfi", "-i", "sine=frequency=440:duration=3"]);
        }
        cmd.args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-b:v",
            "4000k",
            "-pix_fmt",
            "yuv420p",
        ]);
        if with_audio {
            cmd.args(["-c:a", "aac", "-shortest"]);
        }
        let made = cmd.arg(&out).output().ok()?;
        assert!(
            made.status.success(),
            "{}",
            String::from_utf8_lossy(&made.stderr)
        );
        Some(out.to_string_lossy().into_owned())
    }

    fn path(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for Films {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A server that refuses at once: nothing listens on port 1 of this machine.
fn server(state: &AppState) -> String {
    let mut input = valid_input("Nowhere");
    input.host = String::from("127.0.0.1");
    input.port = 1;
    input.auth_kind = AuthKind::Password;
    servers::server_add(state, input, "not-a-real-password").expect("the profile was not made")
}

async fn until(
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
            "{what}: the video never got there; it is {:?} at {:?}: {:?}",
            now.state,
            now.stage,
            now.problem
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn skipped() -> bool {
    if ffmpeg::locate("ffmpeg").is_err() {
        eprintln!("SKIPPED: no bundled FFmpeg. Run `npm run ffmpeg` for this to check anything.");
        return true;
    }
    false
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_file_is_taken_or_refused_on_its_own() {
    if skipped() {
        return;
    }
    let films = Films::new();
    let good = films.film("Good One.mp4", true).unwrap();
    let silent = films.film("silent.mp4", false).unwrap();
    let text = films.path("notes.txt");
    std::fs::write(&text, "not a film").unwrap();
    let missing = films.path("gone.mp4");

    let state = state();
    let server = server(&state);
    let added = video::video_add(
        &state,
        &server,
        &[
            text.clone(),
            good.clone(),
            missing.clone(),
            silent.clone(),
            good.clone(),
        ],
        None,
    )
    .await
    .expect("adding went wrong as a whole");

    // The good one is added though the one before it is not a film (as T665).
    assert_eq!(added.added.len(), 1);
    let one = &added.added[0];
    assert_eq!(one.source_path, good);
    assert_eq!(one.title, "Good One");
    assert_eq!(one.slug, "good-one");
    assert_eq!(one.state, VideoState::Planning);
    assert_eq!(one.stage, VideoStage::Planned);

    // Each refusal names its file and why.
    let refused: Vec<(&str, ErrorCode)> = added
        .refused
        .iter()
        .map(|r| (r.path.as_str(), r.error.code))
        .collect();
    assert!(
        refused.contains(&(text.as_str(), ErrorCode::InvalidInput)),
        "{refused:?}"
    );
    assert!(
        refused.contains(&(missing.as_str(), ErrorCode::InvalidInput)),
        "{refused:?}"
    );
    assert!(
        refused.contains(&(silent.as_str(), ErrorCode::NoAudioTracks)),
        "{refused:?}"
    );
    // The same file twice in one call is one video.
    let twice = added
        .refused
        .iter()
        .find(|r| r.path == good)
        .expect("the second copy of the same file was taken");
    assert_eq!(twice.error.details[0].key, DetailCode::VideoAlreadyListed);

    // And it is not added again while it is on its way.
    let again = video::video_add(&state, &server, std::slice::from_ref(&good), None)
        .await
        .unwrap();
    assert!(again.added.is_empty());
    assert_eq!(
        again.refused[0].error.details[0].key,
        DetailCode::VideoAlreadyListed
    );
}

#[tokio::test]
async fn a_video_for_a_server_that_does_not_exist_is_refused_whole() {
    let state = state();
    let err = video::video_add(&state, "srv_nobody", &[String::from("x.mp4")], None)
        .await
        .expect_err("videos were added for nowhere");
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_plan_comes_back_with_its_rungs_sizes_time_and_room() {
    if skipped() {
        return;
    }
    let films = Films::new();
    let film = films.film("plan.mp4", true).unwrap();
    let state = state();
    let server = server(&state);
    let id = video::video_add(&state, &server, &[film], None)
        .await
        .unwrap()
        .added[0]
        .id
        .clone();

    let ready = until(&state, &id, "the plan", Duration::from_secs(120), |v| {
        v.state != VideoState::Planning
    })
    .await;
    assert_eq!(ready.state, VideoState::Ready, "{:?}", ready.problem);
    let plan = ready.plan.expect("a ready video has no plan");
    assert!(!plan.rungs.is_empty());
    // Nothing measured yet: the formula's preview, and «Start» begins with measuring.
    assert_eq!(plan.from, PlanSource::Formula);
    assert!(plan.needs_measuring);
    assert!(plan.measure_s > 0);
    assert!(plan.server_bytes > 0 && plan.local_bytes > 0);
    assert!(plan.server_bytes >= plan.local_bytes);
    assert!(plan.encode_s.is_some());
    assert!(!plan.encoder.is_empty());
    // The server did not answer: not a refusal, an unknown.
    assert_eq!(plan.server_space, SpaceCheck::Unknown);
    assert_eq!(plan.name_taken, None);
    // This machine's own disk was asked.
    assert!(
        matches!(plan.local_space, SpaceCheck::Fits { .. }),
        "{:?}",
        plan.local_space
    );
    // The source and its sound are there to choose from.
    let source = ready.source.expect("no source");
    assert_eq!(source.audio_tracks.len(), 1);
    assert_eq!(ready.audio_track, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn what_a_state_does_not_allow_is_refused_and_changes_nothing() {
    if skipped() {
        return;
    }
    let films = Films::new();
    let film = films.film("states.mp4", true).unwrap();
    let state = state();
    let server = server(&state);
    let id = video::video_add(&state, &server, &[film], None)
        .await
        .unwrap()
        .added[0]
        .id
        .clone();
    until(&state, &id, "the plan", Duration::from_secs(120), |v| {
        v.state == VideoState::Ready
    })
    .await;

    for err in [
        video::video_pause(&state, &id).unwrap_err(),
        video::video_resume(&state, &id).unwrap_err(),
        video::video_retry(&state, &id, false).unwrap_err(),
    ] {
        assert_eq!(err.code, ErrorCode::VideoNotNow);
    }
    assert_eq!(
        video::video_get(&state, &id).unwrap().state,
        VideoState::Ready
    );

    // The audio track is checked against the film.
    let err = video::video_set_audio(&state, &id, 3).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert_eq!(err.details[0].key, DetailCode::PlanNoSuchTrack);
    assert_eq!(
        video::video_set_audio(&state, &id, 0).unwrap().audio_track,
        0
    );

    // Rungs nobody measured are not built (FR-141), here as on the ladder screen.
    let unmeasured = vec![Rung {
        index: 0,
        bitrate_bps: 2_000_000,
        maxrate_bps: 2_200_000,
        bufsize_bps: 2_200_000,
        width: 1280,
        height: 720,
        level: String::from("3.1"),
        reasons: Vec::new(),
        quality: Quality::NotMeasured,
    }];
    let err = video::video_set_rungs(&state, &id, Some(unmeasured.clone())).unwrap_err();
    assert_eq!(err.code, ErrorCode::LadderNotMeasured);
    // Measured ones are taken, and the plan says so; `None` goes back to the plan's own.
    let measured: Vec<Rung> = unmeasured
        .into_iter()
        .map(|r| Rung {
            quality: Quality::MeasuredHere { vmaf_x100: 9400 },
            ..r
        })
        .collect();
    let edited = video::video_set_rungs(&state, &id, Some(measured)).unwrap();
    let plan = edited.plan.unwrap();
    assert_eq!(plan.from, PlanSource::Edited);
    assert!(!plan.needs_measuring);
    assert_eq!(plan.rungs.len(), 1);
    let back = video::video_set_rungs(&state, &id, None).unwrap();
    assert_eq!(back.plan.unwrap().from, PlanSource::Formula);

    // A name is checked as a medium's would be.
    let err = video::video_set_name(&state, &id, "  ", None).unwrap_err();
    assert_eq!(err.details[0].key, DetailCode::MediaTitleEmpty);
    let renamed = video::video_set_name(&state, &id, "Серия 1", None).unwrap();
    assert_eq!(renamed.slug, "seriya-1");

    // Off the list; nothing else is asked of anybody.
    video::video_remove(&state, &id).unwrap();
    assert!(video::video_list(&state).unwrap().is_empty());
    assert_eq!(
        video::video_remove(&state, &id).unwrap_err().code,
        ErrorCode::VideoNotFound
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_start_that_cannot_reach_the_server_stops_on_a_problem_with_a_retry() {
    if skipped() {
        return;
    }
    let films = Films::new();
    let film = films.film("start.mp4", true).unwrap();
    let state = state();
    let server = server(&state);
    let mut events = state.subscribe();
    let id = video::video_add(&state, &server, &[film], None)
        .await
        .unwrap()
        .added[0]
        .id
        .clone();

    // «Start» before the plan is ready: it goes the moment the plan is.
    let started = video::video_start(&state, std::slice::from_ref(&id));
    assert_eq!(started.len(), 1);
    assert!(started[0].error.is_none(), "{:?}", started[0].error);
    assert!(video::video_get(&state, &id).unwrap().start_requested);

    let stopped = until(&state, &id, "the problem", Duration::from_secs(120), |v| {
        v.state == VideoState::Problem
    })
    .await;
    let problem = stopped.problem.expect("a problem with nothing to say");
    assert!(
        problem.actions.contains(&VideoAction::Retry),
        "{:?}",
        problem.actions
    );
    // Nothing was made, so it stopped where it began.
    assert_eq!(stopped.stage, VideoStage::Planned);
    assert!(stopped.media_id.is_none());

    // Every change went out as the whole video, by id.
    let mut seen_problem = false;
    while let Ok(event) = events.try_recv() {
        if let AppEvent::VideoUpdate(view) = event {
            assert_eq!(view.id, id);
            seen_problem |= view.state == VideoState::Problem;
        }
    }
    assert!(seen_problem, "the problem was never sent to the screen");

    // A retry tries again, and stops on the same problem again — not stuck, not silent.
    let retried = video::video_retry(&state, &id, false).unwrap();
    assert!(matches!(
        retried.state,
        VideoState::Working | VideoState::Problem | VideoState::Planning
    ));
    until(
        &state,
        &id,
        "the second problem",
        Duration::from_secs(120),
        |v| v.state == VideoState::Problem,
    )
    .await;

    // A problem can be stopped and then removed.
    let cancelled = video::video_cancel(&state, &id).unwrap();
    assert_eq!(cancelled.state, VideoState::Cancelled);
    video::video_remove(&state, &id).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn after_a_restart_each_video_does_what_its_state_says() {
    let state = state();
    let server = server(&state);
    let put = |id: &str, stage: VideoStage, state_: VideoState, paused: bool| {
        let mut row = VideoRow::new(id, &server, "C:/nowhere/film.mp4", id, id);
        row.stage = stage;
        row.state = state_;
        row.paused_by_person = paused;
        row.media_id = Some(format!("m_{id}"));
        rows::save(&state.db, &row).unwrap();
    };
    put("paused", VideoStage::Uploading, VideoState::Paused, true);
    put("ready", VideoStage::Planned, VideoState::Ready, false);
    put(
        "stopping",
        VideoStage::Cutting,
        VideoState::Cancelling,
        false,
    );
    put("done", VideoStage::Done, VideoState::Done, false);
    put("going", VideoStage::Encoding, VideoState::Working, false);

    let carried = video::restore_videos(&state).unwrap();
    assert_eq!(
        carried, 1,
        "only the one that was going carries on by itself"
    );

    let get = |id: &str| video::video_get(&state, id).unwrap();
    // A pause a person pressed stays a pause.
    let paused = get("paused");
    assert_eq!(paused.state, VideoState::Paused);
    assert!(paused.paused_by_person);
    assert_eq!(paused.stage, VideoStage::Uploading);
    assert_eq!(get("ready").state, VideoState::Ready);
    assert_eq!(get("done").state, VideoState::Done);
    // Stop had been pressed: it is stopped now, and «Retry» is there for it.
    assert_eq!(get("stopping").state, VideoState::Cancelled);

    // The one that was going was taken up again from its stage. Its source is gone, so it
    // stops on a problem — at its stage, not back at the beginning.
    let going = until(
        &state,
        "going",
        "carrying on",
        Duration::from_secs(60),
        |v| v.state == VideoState::Problem,
    )
    .await;
    assert_eq!(going.stage, VideoStage::Encoding);
    assert!(going.problem.unwrap().actions.contains(&VideoAction::Retry));
}

#[test]
fn a_video_waiting_for_start_is_not_paused_or_resumed_by_a_restart() {
    // The rule in one line, apart from the commands: what does not carry on is left as it is.
    use vrcast_studio_lib::domain::video::{after_restart, AfterRestart};
    assert_eq!(after_restart(VideoState::Ready), AfterRestart::Leave);
    assert_eq!(after_restart(VideoState::Paused), AfterRestart::Leave);
}

// ---------- «Replace» (T676) ----------

/// A video stopped on a taken name, as `next_task` leaves it: no medium of its own yet.
fn stopped_on_a_taken_name(state: &AppState, server: &str, id: &str) {
    let mut row = VideoRow::new(id, server, "C:/nowhere/film.mp4", "Film", "film");
    row.stage = VideoStage::Planned;
    row.state = VideoState::Problem;
    row.problem_json = Some(
        serde_json::json!({
            "error": vrcast_studio_lib::commands::error::AppError::new(ErrorCode::SlugTaken),
            "actions": ["replace", "rename"],
        })
        .to_string(),
    );
    rows::save(&state.db, &row).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replace_is_only_for_a_taken_name_and_asks_the_server_before_anything_changes() {
    let state = state();
    let server = server(&state);
    stopped_on_a_taken_name(&state, &server, "taken");

    // The server does not answer: nothing was removed, so nothing changes — the video is
    // still stopped on its name, with the same two choices, and «Replace» can be pressed again.
    let err = video::video_replace(&state, "taken", false)
        .await
        .expect_err("replace went ahead without asking the server");
    assert_ne!(err.code, ErrorCode::VideoNotNow, "{err:?}");
    let after = video::video_get(&state, "taken").unwrap();
    assert_eq!(after.state, VideoState::Problem);
    assert!(after.media_id.is_none());
    assert_eq!(
        after.problem.unwrap().actions,
        vec![VideoAction::Replace, VideoAction::Rename]
    );
    assert!(state.tasks.list().unwrap().is_empty(), "work was started");

    // Not for a video that is not stopped on its name, nor one that has its medium.
    let mut ready = VideoRow::new("ready", &server, "C:/nowhere/a.mp4", "A", "a");
    ready.state = VideoState::Ready;
    rows::save(&state.db, &ready).unwrap();
    let mut owned = VideoRow::new("owned", &server, "C:/nowhere/b.mp4", "B", "b");
    owned.state = VideoState::Problem;
    owned.media_id = Some(String::from("m_b"));
    rows::save(&state.db, &owned).unwrap();
    for id in ["ready", "owned"] {
        for confirmed in [false, true] {
            let err = video::video_replace(&state, id, confirmed)
                .await
                .expect_err("replace was taken in a state that does not allow it");
            assert_eq!(err.code, ErrorCode::VideoNotNow, "{id}");
        }
    }
    assert_eq!(
        video::video_replace(&state, "nobody", false)
            .await
            .unwrap_err()
            .code,
        ErrorCode::VideoNotFound
    );
}

// ---------- «Build a set» for a medium already in the library (T675) ----------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_set_for_a_medium_takes_one_file_and_asks_the_server_before_adding_anything() {
    let state = state();
    let server = server(&state);

    // One file for one medium: two, or none, is refused as a whole, before the server.
    for paths in [
        vec![String::from("a.mp4"), String::from("b.mp4")],
        Vec::new(),
    ] {
        let err = video::video_add(&state, &server, &paths, Some("m_1"))
            .await
            .expect_err("a medium took a number of files other than one");
        assert_eq!(err.code, ErrorCode::InvalidInput, "{paths:?}");
    }

    // Another video already on its way to this medium: refused as in work, also before the
    // server is asked.
    let mut going = VideoRow::new("going", &server, "C:/nowhere/x.mp4", "X", "x");
    going.state = VideoState::Working;
    going.media_id = Some(String::from("m_1"));
    rows::save(&state.db, &going).unwrap();
    let err = video::video_add(&state, &server, &[String::from("y.mp4")], Some("m_1"))
        .await
        .expect_err("a second set was started for a medium already on its way");
    assert_eq!(err.code, ErrorCode::MediaSetInWork);

    // A finished one does not hold the medium; the server is asked then, and a server that
    // does not answer refuses the call with its own code. Nothing was added either way.
    let mut finished = rows::get(&state.db, "going").unwrap().unwrap();
    finished.state = VideoState::Done;
    rows::save(&state.db, &finished).unwrap();
    let err = video::video_add(&state, &server, &[String::from("y.mp4")], Some("m_1"))
        .await
        .expect_err("a medium nobody could look at was taken");
    assert!(
        !matches!(
            err.code,
            ErrorCode::InvalidInput | ErrorCode::MediaSetInWork | ErrorCode::MediaHasSet
        ),
        "{err:?}"
    );
    assert_eq!(video::video_list(&state).unwrap().len(), 1);
}

#[test]
fn a_medium_s_set_refusals_say_what_to_do() {
    // The two codes are ours, and a problem about a medium's own file under a rung's name
    // offers another rung or another name, not a retry that would meet the same file.
    use vrcast_studio_lib::commands::error::AppError;
    use vrcast_studio_lib::domain::video::actions_for;
    assert_eq!(
        ErrorCode::parse("MEDIA_HAS_SET"),
        Some(ErrorCode::MediaHasSet)
    );
    assert_eq!(
        ErrorCode::parse("MEDIA_SET_IN_WORK"),
        Some(ErrorCode::MediaSetInWork)
    );
    let claimed = AppError::new(ErrorCode::InvalidInput).detail(DetailCode::RungFileClaimed);
    assert_eq!(
        actions_for(&claimed),
        vec![VideoAction::EditRungs, VideoAction::Rename]
    );
    assert_eq!(
        actions_for(&AppError::new(ErrorCode::InvalidInput)),
        vec![VideoAction::Retry]
    );
}

// ---------- a set nobody owns under a medium's name (T677) ----------

/// A video for a medium, as `video_add` leaves it when a set of the medium's name nobody
/// claims lies on the server: stopped on that, before its plan, with «Replace».
fn waiting_on_an_old_set(state: &AppState, server: &str, id: &str) {
    use vrcast_studio_lib::commands::error::AppError;
    use vrcast_studio_lib::domain::wording::Detail;
    let mut row = VideoRow::new(id, server, "C:/nowhere/film.mp4", "Film", "film");
    row.media_id = Some(String::from("m_film"));
    row.own_medium = true;
    row.state = VideoState::Problem;
    let error = AppError::new(ErrorCode::MediaHasSet)
        .with_detail(Detail::new(DetailCode::OldSetUnrecognized).with("name", "film"));
    row.problem_json = Some(
        serde_json::json!({
            "actions": vrcast_studio_lib::domain::video::actions_for(&error),
            "error": error,
        })
        .to_string(),
    );
    rows::save(&state.db, &row).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_medium_s_video_waiting_on_an_old_set_goes_on_only_through_replace() {
    let state = state();
    let server = server(&state);
    waiting_on_an_old_set(&state, &server, "held");

    let shown = video::video_get(&state, "held").unwrap();
    let problem = shown.problem.expect("no problem shown");
    assert_eq!(problem.error.code, ErrorCode::MediaHasSet);
    assert_eq!(problem.actions, vec![VideoAction::Replace]);
    assert_eq!(
        serde_json::to_value(&problem.error).unwrap()["details"][0]["key"],
        "OLD_SET_UNRECOGNIZED"
    );

    // «Retry» would build over the old set: it leaves the video where it was, and starts
    // nothing.
    let after = video::video_retry(&state, "held", false).unwrap();
    assert_eq!(after.state, VideoState::Problem);
    assert_eq!(
        after.problem.unwrap().actions,
        vec![VideoAction::Replace],
        "retry took the video off its old set"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(state.tasks.list().unwrap().is_empty(), "work was started");

    // «Replace» is taken for it although the video has its medium (T676's own case has none):
    // it goes to the server, which does not answer, and nothing changes.
    let err = video::video_replace(&state, "held", false)
        .await
        .expect_err("replace went ahead without asking the server");
    assert_ne!(err.code, ErrorCode::VideoNotNow, "{err:?}");
    let after = video::video_get(&state, "held").unwrap();
    assert_eq!(after.state, VideoState::Problem);
    assert_eq!(after.media_id.as_deref(), Some("m_film"));
    assert_eq!(after.problem.unwrap().actions, vec![VideoAction::Replace]);

    // A medium's video stopped on anything else is not offered «Replace».
    let mut other = VideoRow::new("other", &server, "C:/nowhere/b.mp4", "B", "b");
    other.state = VideoState::Problem;
    other.media_id = Some(String::from("m_b"));
    rows::save(&state.db, &other).unwrap();
    assert_eq!(
        video::video_replace(&state, "other", true)
            .await
            .unwrap_err()
            .code,
        ErrorCode::VideoNotNow
    );
}

// ---------- the library while a medium's set is on its way (T677) ----------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_library_says_a_medium_s_set_is_building_or_stopped_not_missing() {
    use vrcast_studio_lib::commands::library::{api as library, LibraryView, MediaView};
    use vrcast_studio_lib::domain::video::SetWorkState;
    use vrcast_studio_lib::store::library_cache;

    let state = state();
    let server = server(&state);
    let medium = |id: &str| MediaView {
        id: id.to_owned(),
        title: id.to_owned(),
        slug: id.to_owned(),
        files: Vec::new(),
        ladders: Vec::new(),
        set_files: Vec::new(),
        total_bytes: 0,
        created_at: String::from("2026-10-02T00:00:00Z"),
        set_work: None,
    };
    // What the server last said: four media, no sets on any — the state right after
    // «Replace» removed the old one.
    let known = LibraryView {
        server_id: server.clone(),
        media: vec![
            medium("m_go"),
            medium("m_stop"),
            medium("m_plan"),
            medium("m_none"),
        ],
        unrecognized: Vec::new(),
        disk: None,
        stale: false,
    };
    library_cache::save(&state.db, &server, &known).unwrap();

    let row = |id: &str, media: &str, st: VideoState, stage: VideoStage, started: bool| {
        let mut r = VideoRow::new(id, &server, "C:/nowhere/film.mp4", "Film", media);
        r.media_id = Some(media.to_owned());
        r.state = st;
        r.stage = stage;
        r.start_requested = started;
        rows::save(&state.db, &r).unwrap();
    };
    row(
        "v_go",
        "m_go",
        VideoState::Working,
        VideoStage::Encoding,
        true,
    );
    row(
        "v_stop",
        "m_stop",
        VideoState::Problem,
        VideoStage::Cutting,
        true,
    );
    // A plan nobody started yet says nothing about the set.
    row(
        "v_plan",
        "m_plan",
        VideoState::Ready,
        VideoStage::Planned,
        false,
    );

    let view = library::library_list_known(&state, &server).await.unwrap();
    let work = |id: &str| {
        view.media
            .iter()
            .find(|m| m.id == id)
            .unwrap()
            .set_work
            .clone()
    };
    let go = work("m_go").expect("a set on its way reads as nothing");
    assert_eq!(go.state, SetWorkState::Building);
    assert_eq!(go.video_id, "v_go");
    let stop = work("m_stop").expect("a stopped build reads as nothing");
    assert_eq!(stop.state, SetWorkState::Stopped);
    assert_eq!(stop.video_id, "v_stop");
    assert!(work("m_plan").is_none());
    assert!(work("m_none").is_none());
    // It crosses to the screen as plain words, and is never kept in the cache.
    let json = serde_json::to_value(&view.media[0]).unwrap();
    assert_eq!(json["set_work"]["state"], "building");
    assert_eq!(json["set_work"]["video_id"], "v_go");
    assert!(library_cache::load(&state.db, &server)
        .unwrap()
        .unwrap()
        .media
        .iter()
        .all(|m| m.set_work.is_none()));

    // When it changes, the library is told: «Retry» on the stopped one makes it building
    // (and the server, not answering, stops it again).
    let mut events = state.subscribe();
    video::video_retry(&state, "v_stop", false).unwrap();
    let told = std::iter::from_fn(|| events.try_recv().ok())
        .any(|e| matches!(e, AppEvent::LibraryChanged { ref server_id } if server_id == &server));
    assert!(
        told,
        "the library was not told its medium's set is on its way again"
    );
    until(
        &state,
        "v_stop",
        "stopping again",
        Duration::from_secs(20),
        |v| v.state == VideoState::Problem,
    )
    .await;
    let again = library::library_list_known(&state, &server).await.unwrap();
    assert_eq!(
        again
            .media
            .iter()
            .find(|m| m.id == "m_stop")
            .unwrap()
            .set_work
            .as_ref()
            .map(|w| w.state),
        Some(SetWorkState::Stopped)
    );
}

// ---------- the medium a video builds into is not the library's to delete (T684) ----------

/// QA-25 №5: the library deleted, or renamed, the medium a video was building its set into —
/// during the measurement there is no build task yet, and a medium made a moment ago has no
/// paths for the old guard to ask about. The medium is the video's for as long as the video is
/// on its way: planned and started, measuring, encoding, sending, cutting, checking, stopped on
/// a problem, paused, stopping.
#[tokio::test]
async fn the_library_does_not_delete_or_rename_the_medium_a_video_is_building_into() {
    use vrcast_studio_lib::commands::library::api as library;

    let state = state();
    let server = server(&state);
    let mut n = 0;
    let mut on = |st: VideoState, stage: VideoStage| {
        n += 1;
        let medium = format!("m{n}");
        let mut r = VideoRow::new(
            &format!("v{n}"),
            &server,
            "C:/nowhere/film.mp4",
            "Film",
            &medium,
        );
        r.media_id = Some(medium.clone());
        r.state = st;
        r.stage = stage;
        r.start_requested = true;
        rows::save(&state.db, &r).unwrap();
        medium
    };
    let busy = [
        on(VideoState::Working, VideoStage::Measuring),
        on(VideoState::Working, VideoStage::Encoding),
        on(VideoState::Working, VideoStage::Cutting),
        on(VideoState::Working, VideoStage::Verifying),
        on(VideoState::Paused, VideoStage::Uploading),
        on(VideoState::Problem, VideoStage::Encoding),
        on(VideoState::Cancelling, VideoStage::Cutting),
        on(VideoState::Planning, VideoStage::Planned),
        // Cancelled once begun: the library shows it «stopped», and «Retry» carries on into
        // the same medium.
        on(VideoState::Cancelled, VideoStage::Encoding),
    ];
    for medium in &busy {
        for confirmed in [false, true] {
            let e = library::media_delete(&state, &server, medium, confirmed)
                .await
                .expect_err("a medium a video builds into was deleted");
            assert_eq!(e.code, ErrorCode::MediaBusy, "{medium}: {e:?}");
            assert!(
                e.details
                    .iter()
                    .any(|d| d.key == DetailCode::MediaBusyVideo),
                "{medium}: {e:?}"
            );
        }
        let e = library::media_rename(&state, &server, medium, None, Some("other"), true)
            .await
            .expect_err("a medium a video builds into was renamed");
        assert_eq!(e.code, ErrorCode::MediaBusy, "{medium}: {e:?}");
    }
    // Finished: the medium is the library's again — the refusal that comes back is the
    // unreachable server's, not this one.
    {
        let medium = on(VideoState::Done, VideoStage::Done);
        let e = library::media_delete(&state, &server, &medium, true)
            .await
            .unwrap_err();
        assert_ne!(e.code, ErrorCode::MediaBusy, "{e:?}");
    }
    let mut waiting = VideoRow::new("v_wait", &server, "C:/nowhere/film.mp4", "Film", "m_wait");
    waiting.media_id = Some(String::from("m_wait"));
    waiting.state = VideoState::Ready;
    rows::save(&state.db, &waiting).unwrap();
    // A plan for a medium of the library waiting for «Start» builds into it all the same.
    let e = library::media_delete(&state, &server, "m_wait", true)
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::MediaBusy, "{e:?}");
    // Once the video is removed from the list, the medium is the library's again.
    video::video_remove(&state, "v_wait").unwrap();
    let e = library::media_delete(&state, &server, "m_wait", true)
        .await
        .unwrap_err();
    assert_ne!(e.code, ErrorCode::MediaBusy, "{e:?}");
}

/// QA-25 №5, the other half: a build that finished with no medium to file its set under — it
/// vanished meanwhile (another copy of the application) — is a problem of the video, not
/// «Done» with a set the catalogue does not know.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_build_that_finds_no_medium_to_file_its_set_under_is_a_problem_not_done() {
    use vrcast_studio_lib::tasks::state::TaskKind;
    use vrcast_studio_lib::tasks::store::Batch;

    let state = state();
    let server = server(&state);
    let mut r = VideoRow::new("v_gone", &server, "C:/nowhere/film.mp4", "Film", "film");
    r.media_id = Some(String::from("m_gone"));
    r.state = VideoState::Working;
    r.stage = VideoStage::Verifying;
    r.start_requested = true;
    rows::save(&state.db, &r).unwrap();
    video::video_list(&state).unwrap();

    // The build ends well and files nothing: `attach_built_set` found no medium of the slug.
    state
        .tasks
        .submit_in_batch(
            TaskKind::BuildLadder,
            Some(server.clone()),
            Some(Batch {
                id: r.id.clone(),
                label: r.title.clone(),
            }),
            |_ctx| async move { Ok(()) },
        )
        .await
        .unwrap();
    let ended = until(&state, "v_gone", "the end", Duration::from_secs(10), |v| {
        !matches!(v.state, VideoState::Working)
    })
    .await;
    assert_eq!(ended.state, VideoState::Problem, "{ended:?}");
    let problem = ended.problem.expect("no problem said");
    assert_eq!(problem.error.code, ErrorCode::VideoMediumGone);
    assert_eq!(problem.actions, vec![VideoAction::Retry]);
    assert!(ended.link.is_none());
}
