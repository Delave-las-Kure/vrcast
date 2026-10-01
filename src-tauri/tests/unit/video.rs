//! T672 — the rules of a video in work, checked without a film, a server or a running task.

use std::sync::Arc;

use vrcast_studio_lib::commands::error::{AppError, ErrorCode};
use vrcast_studio_lib::domain::ladder::{Quality, Rung};
use vrcast_studio_lib::domain::source::{AudioTrack, SourceFile};
use vrcast_studio_lib::domain::video::{
    self, after_restart, allowed, Act, AfterRestart, VideoAction, VideoStage, VideoState,
};
use vrcast_studio_lib::domain::wording::DetailCode;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::videos::{self, VideoRow};

// ---------- the stage a task's own code belongs to ----------

#[test]
fn every_stage_code_the_pipeline_runs_through_lands_on_its_stage() {
    // The mapping T672 names, one line each — the screen's whole bar stands on it.
    let expected = [
        (DetailCode::StagePreparingMeasurement, VideoStage::Measuring),
        (DetailCode::StageMeasuringQuality, VideoStage::Measuring),
        (DetailCode::StageCheckingLoan, VideoStage::Measuring),
        (DetailCode::StageBuildingLadder, VideoStage::Encoding),
        (DetailCode::StageConverting, VideoStage::Encoding),
        (DetailCode::StageValidating, VideoStage::Encoding),
        (DetailCode::StageSendingVariant, VideoStage::Uploading),
        (DetailCode::StageCuttingSegments, VideoStage::Cutting),
        (DetailCode::StageVerifyingLadder, VideoStage::Verifying),
    ];
    for (code, stage) in expected {
        assert_eq!(video::stage_of(code), Some(stage), "{code:?}");
    }
}

#[test]
fn codes_that_say_nothing_about_where_the_video_is_move_nothing() {
    // The end of a task and a stop still being confirmed are not stages: taking them for one
    // would put a finished build back at «planned» or a cutting at nowhere.
    for code in [
        DetailCode::StageDone,
        DetailCode::StageStopUnconfirmed,
        DetailCode::StageChecksum,
        DetailCode::StageDeploying,
    ] {
        assert_eq!(video::stage_of(code), None, "{code:?}");
    }
}

#[test]
fn the_stages_go_in_the_order_the_bar_draws_them() {
    use VideoStage::*;
    let order = [
        Planned, Measuring, Encoding, Uploading, Cutting, Verifying, Done,
    ];
    for pair in order.windows(2) {
        assert!(
            pair[0] < pair[1],
            "{:?} is not before {:?}",
            pair[0],
            pair[1]
        );
    }
    assert_eq!(VideoStage::ALL, &order);
}

#[test]
fn the_rung_in_hand_is_read_back_out_of_the_builds_own_share() {
    // The build reports done/(n+1) as it takes up each rung.
    let n = 3;
    for done in 0..n {
        let share = done as f64 / (n as f64 + 1.0);
        assert_eq!(video::rung_at(share, n), done as u32 + 1);
    }
    // Never past the last, never for a set of nothing.
    assert_eq!(video::rung_at(1.0, n), 3);
    assert_eq!(video::rung_at(0.5, 0), 0);
}

// ---------- what a state allows ----------

#[test]
fn only_a_going_video_is_paused_and_only_a_paused_one_continued() {
    for state in VideoState::ALL {
        assert_eq!(
            allowed(Act::Pause, *state, VideoStage::Encoding, true),
            *state == VideoState::Working,
            "pause in {state:?}"
        );
        assert_eq!(
            allowed(Act::Resume, *state, VideoStage::Encoding, true),
            *state == VideoState::Paused,
            "resume in {state:?}"
        );
    }
}

#[test]
fn retry_is_offered_after_a_problem_or_a_stop_and_nowhere_else() {
    for state in VideoState::ALL {
        let want = matches!(state, VideoState::Problem | VideoState::Cancelled);
        assert_eq!(
            allowed(Act::Retry, *state, VideoStage::Uploading, true),
            want,
            "{state:?}"
        );
    }
}

#[test]
fn a_video_is_not_taken_off_the_list_while_its_work_is_alive() {
    // Removing it would leave a build running for something nobody can see any more.
    assert!(!allowed(
        Act::Remove,
        VideoState::Working,
        VideoStage::Encoding,
        true
    ));
    assert!(!allowed(
        Act::Remove,
        VideoState::Cancelling,
        VideoStage::Cutting,
        true
    ));
    for state in [
        VideoState::Ready,
        VideoState::Paused,
        VideoState::Problem,
        VideoState::Cancelled,
        VideoState::Done,
        VideoState::Planning,
    ] {
        assert!(
            allowed(Act::Remove, state, VideoStage::Encoding, true),
            "{state:?}"
        );
    }
}

#[test]
fn the_audio_track_is_fixed_once_anything_is_encoded() {
    // A variant already on the server would be recognised as done with the old track in it.
    assert!(allowed(
        Act::SetAudio,
        VideoState::Ready,
        VideoStage::Planned,
        false
    ));
    assert!(allowed(
        Act::SetAudio,
        VideoState::Problem,
        VideoStage::Measuring,
        true
    ));
    assert!(!allowed(
        Act::SetAudio,
        VideoState::Problem,
        VideoStage::Encoding,
        true
    ));
    assert!(!allowed(
        Act::SetAudio,
        VideoState::Working,
        VideoStage::Measuring,
        true
    ));
}

#[test]
fn the_name_is_fixed_once_the_medium_exists() {
    assert!(allowed(
        Act::SetName,
        VideoState::Problem,
        VideoStage::Planned,
        false
    ));
    assert!(!allowed(
        Act::SetName,
        VideoState::Problem,
        VideoStage::Planned,
        true
    ));
    assert!(!allowed(
        Act::SetName,
        VideoState::Working,
        VideoStage::Planned,
        false
    ));
}

#[test]
fn rungs_are_never_edited_under_a_running_build() {
    assert!(!allowed(
        Act::SetRungs,
        VideoState::Working,
        VideoStage::Encoding,
        true
    ));
    assert!(!allowed(
        Act::SetRungs,
        VideoState::Paused,
        VideoStage::Encoding,
        true
    ));
    assert!(allowed(
        Act::SetRungs,
        VideoState::Ready,
        VideoStage::Planned,
        false
    ));
    assert!(allowed(
        Act::SetRungs,
        VideoState::Problem,
        VideoStage::Encoding,
        true
    ));
}

#[test]
fn start_is_for_a_video_waiting_for_it_only() {
    for state in VideoState::ALL {
        let want = matches!(state, VideoState::Ready | VideoState::Planning);
        assert_eq!(
            allowed(Act::Start, *state, VideoStage::Planned, false),
            want,
            "{state:?}"
        );
    }
}

// ---------- after a restart ----------

#[test]
fn after_a_restart_a_going_video_carries_on_and_a_paused_one_waits() {
    // The owner's decision of 2026-10-01: unfinished videos carry on by themselves from their
    // stage — except those a person paused, which stay paused.
    assert_eq!(after_restart(VideoState::Working), AfterRestart::CarryOn);
    assert_eq!(after_restart(VideoState::Paused), AfterRestart::Leave);
    assert_eq!(after_restart(VideoState::Planning), AfterRestart::PlanAgain);
    assert_eq!(
        after_restart(VideoState::Cancelling),
        AfterRestart::NowCancelled
    );
    for state in [
        VideoState::Ready,
        VideoState::Problem,
        VideoState::Cancelled,
        VideoState::Done,
    ] {
        assert_eq!(after_restart(state), AfterRestart::Leave, "{state:?}");
    }
}

// ---------- what a problem offers ----------

#[test]
fn a_taken_name_offers_a_choice_and_not_a_retry() {
    // Pressing retry would only meet the same name again.
    let actions = video::actions_for(&AppError::new(ErrorCode::SlugTaken));
    assert_eq!(actions, vec![VideoAction::Replace, VideoAction::Rename]);
}

#[test]
fn viewers_on_the_server_and_objections_offer_build_anyway() {
    assert!(video::actions_for(&AppError::new(ErrorCode::FileInUse))
        .contains(&VideoAction::BuildAnyway));
    let objection = video::actions_for(&AppError::new(ErrorCode::LadderObjection));
    assert!(objection.contains(&VideoAction::BuildAnyway));
    assert!(objection.contains(&VideoAction::EditRungs));
}

#[test]
fn everything_else_is_retried_from_where_it_stopped() {
    for code in [
        ErrorCode::SshUnreachable,
        ErrorCode::RemoteDiskFull,
        ErrorCode::LocalDiskFull,
        ErrorCode::LadderIncomplete,
        ErrorCode::DecodeValidationFailed,
        ErrorCode::Internal,
    ] {
        assert!(
            video::actions_for(&AppError::new(code)).contains(&VideoAction::Retry),
            "{code:?}"
        );
    }
}

// ---------- names, tracks and time ----------

#[test]
fn a_file_is_offered_under_its_own_name() {
    assert_eq!(
        video::title_of("C:/films/Blue Eye S01E03.mkv"),
        "Blue Eye S01E03"
    );
    assert_eq!(video::title_of("/v/a.b.c.mp4"), "a.b.c");
}

#[test]
fn a_name_with_nothing_usable_in_it_still_gets_a_short_name() {
    // Refusing a film for its name would stop it before its plan.
    let slug = video::slug_for("……", "v_0123ABCDEF");
    assert!(slug.starts_with("video-"), "{slug}");
    vrcast_studio_lib::domain::media::validate_slug(&slug).expect("a made-up name must be valid");
    assert_eq!(video::slug_for("Синий глаз", "x"), "siniy-glaz");
}

fn source(duration_s: f64) -> SourceFile {
    SourceFile {
        path: String::from("/f.mp4"),
        size_bytes: 1,
        duration_s,
        width: 1920,
        height: 1080,
        fps: 24,
        bitrate_bps: 20_000_000,
        peak_bps: None,
        video_codec: String::from("h264"),
        pix_fmt: String::from("yuv420p"),
        color_transfer: None,
        audio_tracks: vec![
            AudioTrack {
                index: 0,
                codec: String::from("aac"),
                profile: None,
                channels: 2,
                bitrate_bps: None,
                language: None,
                title: None,
                is_default: false,
            },
            AudioTrack {
                index: 1,
                codec: String::from("aac"),
                profile: None,
                channels: 2,
                bitrate_bps: None,
                language: None,
                title: None,
                is_default: true,
            },
        ],
    }
}

fn rung(bitrate_bps: u64, width: u32, height: u32) -> Rung {
    Rung {
        index: 0,
        bitrate_bps,
        maxrate_bps: bitrate_bps,
        bufsize_bps: bitrate_bps,
        width,
        height,
        level: String::from("4.1"),
        reasons: Vec::new(),
        quality: Quality::MeasuredHere { vmaf_x100: 9500 },
    }
}

#[test]
fn the_track_marked_default_is_the_one_offered() {
    assert_eq!(video::default_audio(&source(60.0)), 1);
}

#[test]
fn encoding_time_counts_the_pixels_made_and_skips_a_rung_carried_across() {
    let film = source(100.0);
    let speed = 1920.0 * 1080.0 * 24.0; // real time at 1080p24
    let one = video::encode_seconds(&[rung(8_000_000, 1920, 1080)], &film, speed).unwrap();
    // 100 s of film at real time, plus the decode check after it.
    assert_eq!(one, (100.0 * (1.0 + video::VALIDATE_SHARE)).round() as u64);
    // A rung that *is* the source is copied, and costs nothing here.
    let copy = rung(20_000_000, 1920, 1080);
    assert!(video::is_copy(&copy, &film));
    assert_eq!(video::encode_seconds(&[copy], &film, speed), Some(0));
    // No length, no number — a made-up one would look like an estimate.
    assert_eq!(
        video::encode_seconds(&[rung(8_000_000, 1920, 1080)], &source(0.0), speed),
        None
    );
}

#[test]
fn this_machines_speed_is_the_middle_of_what_it_did() {
    assert_eq!(video::middle_speed(vec![]), None);
    assert_eq!(video::middle_speed(vec![3.0, 1.0, 100.0]), Some(3.0));
    assert_eq!(video::middle_speed(vec![f64::NAN, -1.0]), None);
}

#[test]
fn time_left_is_said_only_once_there_is_something_to_go_by() {
    assert_eq!(video::eta_s(1.0, 0.0, 0.5), None, "too soon");
    assert_eq!(video::eta_s(10.0, 0.2, 0.2), None, "nothing moved");
    assert_eq!(video::eta_s(10.0, 0.0, 0.5), Some(10));
}

// ---------- the store ----------

fn db() -> Arc<Db> {
    Arc::new(Db::open_in_memory().unwrap())
}

fn with_server(db: &Db) -> String {
    let id = String::from("s1");
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO server_profiles
                (id, name, host, port, username, auth_kind, secret_ref, domain, video_dir,
                 is_active, created_at)
             VALUES ('s1', 'S', '127.0.0.1', 22, 'root', 'password', 'ref', 'x.example',
                     '/v', 1, '2026-01-01T00:00:00Z')",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    id
}

#[test]
fn a_video_is_kept_whole_and_comes_back_the_same() {
    let db = db();
    let server = with_server(&db);
    let mut row = VideoRow::new("v1", &server, "C:/f.mp4", "F", "f");
    row.audio_track = 2;
    row.stage = VideoStage::Cutting;
    row.state = VideoState::Paused;
    row.paused_by_person = true;
    row.measured = true;
    row.own_medium = true;
    row.media_id = Some(String::from("m_1"));
    row.task_id = Some(String::from("t_1"));
    row.plan_json = Some(String::from("{}"));
    videos::save(&db, &row).unwrap();

    let back = videos::get(&db, "v1").unwrap().unwrap();
    assert_eq!(back.stage, VideoStage::Cutting);
    assert_eq!(back.state, VideoState::Paused);
    assert!(back.paused_by_person && back.measured && back.own_medium);
    assert_eq!(back.audio_track, 2);
    assert_eq!(back.media_id.as_deref(), Some("m_1"));
    assert_eq!(back.task_id.as_deref(), Some("t_1"));
}

#[test]
fn the_list_keeps_the_order_videos_were_added_in() {
    let db = db();
    let server = with_server(&db);
    for id in ["b", "a", "c"] {
        videos::save(&db, &VideoRow::new(id, &server, id, id, id)).unwrap();
    }
    let ids: Vec<String> = videos::list(&db)
        .unwrap()
        .into_iter()
        .map(|v| v.id)
        .collect();
    assert_eq!(ids, vec!["b", "a", "c"]);
}

#[test]
fn the_order_holds_when_the_times_are_the_same_or_read_backwards() {
    // The flake (2026-10-01): the list was ordered by `created_at`, a string. Here the times
    // are equal for two and, for the third, earlier as text than the first — the order of
    // adding is what comes back regardless.
    let db = db();
    let server = with_server(&db);
    for (id, at) in [
        ("first", "2026-10-01T10:00:05.123456789Z"),
        ("second", "2026-10-01T10:00:05.123456789Z"),
        ("third", "2026-10-01T10:00:05.000000000Z"),
    ] {
        let mut row = VideoRow::new(id, &server, id, id, id);
        row.created_at = at.to_owned();
        videos::save(&db, &row).unwrap();
    }
    // Saving again (an update) does not move a video in the list.
    let mut first = videos::get(&db, "first").unwrap().unwrap();
    first.title = String::from("renamed");
    videos::save(&db, &first).unwrap();
    let ids: Vec<String> = videos::list(&db)
        .unwrap()
        .into_iter()
        .map(|v| v.id)
        .collect();
    assert_eq!(ids, vec!["first", "second", "third"]);
}

#[test]
fn a_removed_video_s_place_is_not_given_to_an_earlier_one() {
    let db = db();
    let server = with_server(&db);
    for id in ["a", "b", "c"] {
        videos::save(&db, &VideoRow::new(id, &server, id, id, id)).unwrap();
    }
    videos::remove(&db, "c").unwrap();
    videos::save(&db, &VideoRow::new("d", &server, "d", "d", "d")).unwrap();
    let ids: Vec<String> = videos::list(&db)
        .unwrap()
        .into_iter()
        .map(|v| v.id)
        .collect();
    assert_eq!(ids, vec!["a", "b", "d"]);
}

#[test]
fn stored_times_compare_as_text_the_way_they_do_as_time() {
    // The cause of the flake: the fraction of a second used to lose its trailing zeros, and
    // `…05.1234Z` came after `…05.12345678Z` as text.
    use vrcast_studio_lib::store::db::{now_rfc3339, parse_rfc3339, rfc3339_fixed};
    let base = time::OffsetDateTime::from_unix_timestamp(1_790_000_005).unwrap();
    let earlier = rfc3339_fixed(base + time::Duration::nanoseconds(123_456_780));
    let later = rfc3339_fixed(base + time::Duration::nanoseconds(123_456_789));
    let much_later = rfc3339_fixed(base + time::Duration::nanoseconds(500_000_000));
    assert!(
        earlier < later && later < much_later,
        "{earlier} {later} {much_later}"
    );
    assert_eq!(earlier.len(), "2026-09-21T13:20:05.123456780Z".len());
    assert_eq!(
        rfc3339_fixed(base),
        format!("{}.000000000Z", &rfc3339_fixed(base)[..19])
    );
    // Still RFC 3339, read back the same.
    assert_eq!(parse_rfc3339(&later).unwrap(), 1_790_000_005);
    let now = now_rfc3339();
    assert!(parse_rfc3339(&now).is_ok(), "{now}");
    assert!(now.ends_with('Z') && now.len() == earlier.len(), "{now}");
}

#[test]
fn removing_a_video_removes_only_the_row() {
    let db = db();
    let server = with_server(&db);
    videos::save(&db, &VideoRow::new("v1", &server, "p", "t", "s")).unwrap();
    assert!(videos::remove(&db, "v1").unwrap());
    assert!(videos::get(&db, "v1").unwrap().is_none());
    assert!(
        !videos::remove(&db, "v1").unwrap(),
        "removing twice is not an error"
    );
}

#[test]
fn a_video_goes_with_its_server() {
    let db = db();
    let server = with_server(&db);
    videos::save(&db, &VideoRow::new("v1", &server, "p", "t", "s")).unwrap();
    db.with_conn(|c| {
        c.execute("DELETE FROM server_profiles WHERE id = 's1'", [])?;
        Ok(())
    })
    .unwrap();
    assert!(videos::list(&db).unwrap().is_empty());
}

#[test]
fn the_database_refuses_a_stage_it_does_not_know() {
    let db = db();
    with_server(&db);
    let refused = db.with_conn(|c| {
        Ok(c.execute(
            "INSERT INTO videos (id, server_id, source_path, title, slug, stage, created_at,
                                 updated_at)
             VALUES ('x', 's1', 'p', 't', 's', 'teleporting', 'n', 'n')",
            [],
        ))
    });
    assert!(matches!(refused, Ok(Err(_))), "{refused:?}");
}

#[test]
fn every_stage_and_state_is_one_the_database_takes() {
    let db = db();
    let server = with_server(&db);
    for (i, stage) in VideoStage::ALL.iter().enumerate() {
        for (j, state) in VideoState::ALL.iter().enumerate() {
            let mut row = VideoRow::new(&format!("v{i}-{j}"), &server, "p", "t", "s");
            row.stage = *stage;
            row.state = *state;
            videos::save(&db, &row).unwrap_or_else(|e| panic!("{stage:?}/{state:?}: {e}"));
        }
    }
}

#[test]
fn an_encoders_speed_is_remembered_per_encoder() {
    let db = db();
    assert_eq!(videos::encode_speed(&db, "libx264").unwrap(), None);
    for s in [10.0, 30.0, 20.0] {
        videos::record_encode_speed(&db, "libx264", s).unwrap();
    }
    videos::record_encode_speed(&db, "h264_nvenc", 999.0).unwrap();
    // Nonsense is not written.
    videos::record_encode_speed(&db, "libx264", f64::INFINITY).unwrap();
    videos::record_encode_speed(&db, "libx264", 0.0).unwrap();
    assert_eq!(videos::encode_speed(&db, "libx264").unwrap(), Some(20.0));
    assert_eq!(
        videos::encode_speed(&db, "h264_nvenc").unwrap(),
        Some(999.0)
    );
}

// ---------- «Replace»: what of an old set goes (T676) ----------

fn top(entries: &[(&str, bool)]) -> Vec<(String, bool)> {
    entries.iter().map(|(n, d)| (n.to_string(), *d)).collect()
}

#[test]
fn a_rung_file_is_the_slug_an_underscore_whole_megabits_and_mp4() {
    for yes in ["film_8.mp4", "film_12.mp4", "film_8.mp4.part"] {
        assert!(video::is_rung_file("film", yes), "{yes}");
    }
    for no in [
        "film.mp4",
        "film_.mp4",
        "film_8a.mp4",
        "film_8.mkv",
        "film-2_8.mp4",
        "films_8.mp4",
        "other_8.mp4",
        "film",
    ] {
        assert!(!video::is_rung_file("film", no), "{no}");
    }
}

#[test]
fn the_old_set_is_its_directory_and_its_unclaimed_rung_files() {
    let entries = top(&[
        ("film", true),
        ("film_8.mp4", false),
        ("film_4.mp4.part", false),
        ("film_2.mp4", false),
        // Another medium's, and the medium's own single file: not the set's.
        ("film-2", true),
        ("film-2_8.mp4", false),
        ("film.mp4", false),
        ("library.json", false),
    ]);
    let old = video::old_set("film", &entries, &["film.mp4", "film/master.m3u8"]);
    assert!(old.dir);
    assert_eq!(
        old.files,
        vec!["film_2.mp4", "film_4.mp4.part", "film_8.mp4"]
    );
    assert!(old.in_the_way.is_empty());
    assert_eq!(
        old.tops("film"),
        vec!["film", "film_2.mp4", "film_4.mp4.part", "film_8.mp4"]
    );
    assert!(!old.is_empty());
}

#[test]
fn a_claimed_file_with_a_rung_s_name_is_in_the_way_not_removed() {
    // Somebody filed `film_8.mp4` under a medium by hand: it is theirs (T577, part b), and a
    // build would take it for a finished rung.
    let entries = top(&[("film_8.mp4", false), ("film_4.mp4", false)]);
    let old = video::old_set("film", &entries, &["film_8.mp4"]);
    assert_eq!(old.in_the_way, vec!["film_8.mp4"]);
    assert_eq!(old.files, vec!["film_4.mp4"]);
}

#[test]
fn nothing_under_the_name_is_nothing_to_remove() {
    let old = video::old_set("film", &top(&[("other", true), ("film.mp4", false)]), &[]);
    assert!(old.is_empty());
    assert!(old.tops("film").is_empty());
    // A file called like the directory is not the directory.
    assert!(!video::old_set("film", &top(&[("film", false)]), &[]).dir);
}

// ---------- a set nobody owns under a medium's name (T677) ----------

#[test]
fn a_set_nobody_owns_under_a_medium_s_name_is_a_problem_with_replace_not_a_refusal() {
    let entries = top(&[
        ("film", true),
        ("film_8.mp4", false),
        // The medium's own single file, named like a rung: its own, not the old set's.
        ("film_4.mp4", false),
    ]);
    let problem = video::old_set_problem("film", &entries, &["film_4.mp4"])
        .expect("a set nobody owns was not seen");
    assert_eq!(problem.code, ErrorCode::MediaHasSet);
    assert_eq!(problem.details[0].key, DetailCode::OldSetUnrecognized);
    assert!(video::is_old_set_problem(&problem));
    assert_eq!(video::actions_for(&problem), vec![VideoAction::Replace]);
    // What «Replace» would remove: the directory and the loose rung, never the medium's file.
    let old = video::unclaimed_old_set("film", &entries, &["film_4.mp4"]);
    assert_eq!(old.tops("film"), vec!["film", "film_8.mp4"]);

    // «Replace» may be pressed on it although the video has its medium already; on any other
    // problem of such a video it may not.
    let error = Some(&problem);
    assert!(video::may_replace(
        VideoState::Problem,
        VideoStage::Planned,
        true,
        error
    ));
    assert!(!video::may_replace(
        VideoState::Ready,
        VideoStage::Planned,
        true,
        error
    ));
    let other = AppError::new(ErrorCode::SshUnreachable);
    assert!(!video::may_replace(
        VideoState::Problem,
        VideoStage::Planned,
        true,
        Some(&other)
    ));
    // A plain «this medium has a set» is a refusal, not this.
    assert!(!video::is_old_set_problem(&AppError::new(
        ErrorCode::MediaHasSet
    )));
}

#[test]
fn a_directory_filed_in_the_catalogue_is_somebody_s_set_not_nobody_s() {
    let entries = top(&[("film", true)]);
    assert!(video::old_set_problem("film", &entries, &["film/master.m3u8"]).is_none());
    assert!(video::unclaimed_old_set("film", &entries, &["film/master.m3u8"]).is_empty());
    // Nothing under the name: nothing to wait on.
    assert!(video::old_set_problem("film", &top(&[("other", true)]), &[]).is_none());
}

#[test]
fn a_rung_named_around_a_medium_s_file_is_a_rung_of_its_set() {
    // What «Replace» and the old-set check look for (T676/T677) knows the new names too.
    for yes in ["film_9v.mp4", "film_9v2.mp4", "film_9v.mp4.part"] {
        assert!(video::is_rung_file("film", yes), "{yes}");
    }
    let entries = top(&[("film_9v.mp4", false), ("film_9.mp4", false)]);
    let old = video::old_set("film", &entries, &["film_9.mp4"]);
    assert_eq!(old.files, vec!["film_9v.mp4"]);
    assert_eq!(old.in_the_way, vec!["film_9.mp4"]);
}

#[test]
fn a_video_says_its_medium_s_set_is_building_once_begun_and_stopped_while_it_waits() {
    use vrcast_studio_lib::domain::video::{set_work_of, SetWorkState as W};
    use VideoStage as G;
    use VideoState as S;
    // Going, paused, stopping: building.
    for st in [S::Working, S::Paused, S::Cancelling] {
        assert_eq!(
            set_work_of(st, G::Encoding, true),
            Some(W::Building),
            "{st:?}"
        );
    }
    // «Replace» on a video still waiting for its plan: building from the moment it is pressed.
    assert_eq!(
        set_work_of(S::Planning, G::Planned, true),
        Some(W::Building)
    );
    // Begun, then stopped on a problem or by a person: stopped.
    assert_eq!(set_work_of(S::Problem, G::Cutting, true), Some(W::Stopped));
    assert_eq!(
        set_work_of(S::Cancelled, G::Encoding, false),
        Some(W::Stopped)
    );
    assert_eq!(set_work_of(S::Problem, G::Planned, true), Some(W::Stopped));
    // Nothing begun: a plan, a plan that failed, one waiting on a set nobody owns; and done.
    assert_eq!(set_work_of(S::Planning, G::Planned, false), None);
    assert_eq!(set_work_of(S::Ready, G::Planned, false), None);
    assert_eq!(set_work_of(S::Problem, G::Planned, false), None);
    assert_eq!(set_work_of(S::Done, G::Done, true), None);
}
