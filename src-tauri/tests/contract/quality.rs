//! Contract tests for measuring quality, and for building a set once it is measured.
//!
//! Contract: `contracts/ipc-commands.md`, "Наборы качеств" and the measurement group.
//!
//! These were missing while everything under them was checked: the measurement's arithmetic,
//! its store, its task, the building task, the cutting and the serving all had their own
//! checks, and the commands that reach them had none. The seam between an interface and a
//! core is exactly where a promise gets broken quietly.

use vrcast_studio_lib::commands::error::ErrorCode;
use vrcast_studio_lib::commands::ladder::{api as ladder, BuildRequest};
use vrcast_studio_lib::commands::quality::{api as quality, MeasureRequest};
use vrcast_studio_lib::domain::ladder::{Quality, Rung};

use super::support::state;

fn measuring(path: &str) -> MeasureRequest {
    MeasureRequest {
        path: path.to_owned(),
        codec: String::from("h264"),
        native_height: None,
        prefer_hardware: true,
        then_build: None,
        batch: None,
    }
}

fn rung(index: usize, bitrate_bps: u64, quality: Quality) -> Rung {
    Rung {
        index,
        bitrate_bps,
        maxrate_bps: bitrate_bps * 11 / 10,
        bufsize_bps: bitrate_bps * 11 / 10,
        width: 1920,
        height: 1080,
        level: String::from("4.2"),
        reasons: Vec::new(),
        quality,
    }
}

#[tokio::test]
async fn measuring_a_file_that_is_not_there_is_a_failure_with_a_code() {
    let state = state();
    let err = quality::quality_measure_preview(&state, &measuring("F:/nowhere/no-such.mp4"))
        .await
        .expect_err("a measurement was offered for a file that does not exist");
    assert!(
        matches!(
            err.code,
            ErrorCode::InvalidInput | ErrorCode::FfmpegBroken | ErrorCode::VmafUnavailable
        ),
        "the wrong code came back: {:?}",
        err.code
    );
}

#[tokio::test]
async fn a_measurement_nobody_took_is_reported_as_missing_rather_than_as_empty() {
    // The difference decides what a person is shown: "nothing has been measured yet" invites
    // them to measure, while an empty result looks like a measurement that found nothing and
    // invites them to give up.
    let state = state();
    let err = quality::quality_measure_result(&state, "0:never-measured.mp4", "h264")
        .await
        .expect_err("a measurement that was never taken came back as a result");
    assert_eq!(err.code, ErrorCode::MeasurementNotFound);
}

#[tokio::test]
async fn borrowing_a_measurement_that_does_not_exist_says_so() {
    let state = state();
    let err = quality::quality_measure_reuse(
        &state,
        "0:never-measured.mp4",
        measuring("F:/nowhere/no-such.mp4"),
    )
    .await
    .expect_err("a measurement was borrowed from nowhere");
    // Either the file cannot be read or there is nothing to borrow; both are refusals with a
    // code, and neither is a measurement conjured out of nothing.
    assert!(
        matches!(
            err.code,
            ErrorCode::MeasurementNotFound | ErrorCode::InvalidInput | ErrorCode::FfmpegBroken
        ),
        "the wrong code came back: {:?}",
        err.code
    );
}

#[tokio::test]
async fn nothing_measured_means_nothing_listed() {
    let state = state();
    assert!(quality::quality_measurements(&state)
        .await
        .expect("listing measurements failed")
        .is_empty());
}

#[tokio::test]
async fn forgetting_a_measurement_that_is_not_there_is_not_a_failure() {
    // Somebody pressing "measure again" on a film measured on another machine must not be
    // met with a complaint: there was nothing to throw away, and that is the state they
    // wanted.
    let state = state();
    quality::quality_measure_forget(&state, "0:never-measured.mp4", "h264")
        .await
        .expect("forgetting nothing was reported as a failure");
}

// ---------- building on what was measured ----------

#[tokio::test]
async fn an_unmeasured_ladder_is_refused_before_any_server_is_touched() {
    // **The rule the whole measurement exists for** (FR-141). It has to bite here, in the
    // command, and not only in the task: the refusal costs nothing now and hours later.
    //
    // The server does not exist, so if this reached one it would fail with a different code
    // — which is what makes this check prove the order rather than merely the outcome.
    let state = state();
    let err = ladder::ladder_build(
        &state,
        BuildRequest {
            server_id: String::from("nowhere"),
            path: String::from("F:/films/film.mp4"),
            slug: String::from("demo"),
            rungs: vec![
                rung(0, 22_000_000, Quality::NotMeasured),
                rung(1, 12_000_000, Quality::NotMeasured),
            ],
            audio_track: 0,
            subtitle_track: None,
            prefer_hardware: true,
            batch: None,
            confirmed: true,
        },
    )
    .await
    .expect_err("an unmeasured ladder was sent off to be built");

    assert_eq!(
        err.code,
        ErrorCode::LadderNotMeasured,
        "the refusal was about something else, so it was not the measurement that stopped it"
    );
    // And it says which rungs, because "rebuild the lower one" and "measure everything" are
    // different pieces of work.
    assert!(
        format!("{err:?}").contains("1"),
        "the refusal does not say which rungs are unmeasured: {err:?}"
    );
}

#[tokio::test]
async fn a_ladder_with_no_rungs_is_refused_as_empty_rather_than_as_unmeasured() {
    let state = state();
    let err = ladder::ladder_build(
        &state,
        BuildRequest {
            server_id: String::from("nowhere"),
            path: String::from("F:/films/film.mp4"),
            slug: String::from("demo"),
            rungs: Vec::new(),
            audio_track: 0,
            subtitle_track: None,
            prefer_hardware: true,
            batch: None,
            confirmed: true,
        },
    )
    .await
    .expect_err("an empty ladder was sent off to be built");
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

#[tokio::test]
async fn a_measured_ladder_gets_past_the_refusal_and_fails_on_the_server_instead() {
    // What is checked is the order: the measurement is looked at first, and only then does
    // anything reach for a server. A measured ladder must get a *different* failure here.
    let state = state();
    let err = ladder::ladder_build(
        &state,
        BuildRequest {
            server_id: String::from("nowhere"),
            path: String::from("F:/films/film.mp4"),
            slug: String::from("demo"),
            rungs: vec![
                rung(0, 22_000_000, Quality::MeasuredHere { vmaf_x100: 9600 }),
                rung(1, 12_000_000, Quality::MeasuredHere { vmaf_x100: 9200 }),
            ],
            audio_track: 0,
            subtitle_track: None,
            prefer_hardware: true,
            batch: None,
            confirmed: true,
        },
    )
    .await
    .expect_err("a build was started for a server that does not exist");
    assert_ne!(
        err.code,
        ErrorCode::LadderNotMeasured,
        "a measured ladder was still refused as unmeasured"
    );
}

#[tokio::test]
async fn verifying_a_set_on_a_server_that_does_not_exist_fails_by_name() {
    let state = state();
    let err = ladder::ladder_verify(&state, "nowhere", "demo")
        .await
        .expect_err("a set was verified on a server that does not exist");
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

// ---------- T661: preparing a measurement is part of its task ----------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preparing_a_measurement_is_a_visible_stage_of_its_task_and_a_cancel_reaches_it() {
    use std::time::{Duration, Instant};
    use vrcast_studio_lib::commands::error::DetailCode;
    use vrcast_studio_lib::media::ffmpeg;
    use vrcast_studio_lib::tasks::engine::TaskEvent;
    use vrcast_studio_lib::tasks::state::TaskState;

    // QA-24B-02: reading every packet and trial-encoding three pieces ran inside the
    // command, before the task existed — invisible, and nothing could stop it.
    let (Ok(ff), Ok(info)) = (ffmpeg::locate("ffmpeg"), ffmpeg::probe_self().await) else {
        eprintln!("SKIPPED: no bundled FFmpeg. Run `npm run ffmpeg` for this to check anything.");
        return;
    };
    if !info.has_libvmaf {
        eprintln!("SKIPPED: this FFmpeg cannot measure quality");
        return;
    }

    let dir = std::env::temp_dir().join(format!("vrcast-t661-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    let film = dir.join("film.mp4");
    let made = std::process::Command::new(&ff)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=1280x720:rate=30",
            "-t",
            "60",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&film)
        .output()
        .expect("could not run the bundled FFmpeg");
    assert!(made.status.success());

    let state = state();
    let mut events = state.tasks.subscribe();
    let mut ask = measuring(&film.to_string_lossy());
    ask.prefer_hardware = false;
    let id = quality::quality_measure_start(&state, ask)
        .await
        .expect("the measurement did not start");

    // The first thing the task says is that it is preparing — the work is in the task now.
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut first_stage = None;
    while first_stage.is_none() && Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(200), events.recv()).await {
            Ok(Ok(TaskEvent::Progress {
                id: which,
                stage: Some(stage),
                ..
            })) if which == id => first_stage = Some(stage),
            _ => {}
        }
    }
    assert_eq!(
        first_stage,
        Some(DetailCode::StagePreparingMeasurement),
        "the task did not say it was preparing the measurement"
    );

    state.tasks.cancel(&id).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let ended = loop {
        if let Some(rec) = state.tasks.get(&id).unwrap() {
            if rec.state.is_final() {
                break rec.state;
            }
        }
        assert!(
            Instant::now() < deadline,
            "the cancel did not reach the preparation"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(ended, TaskState::Cancelled);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- T670(1): the loan's check stops with its task ----------

/// A donor measured well enough to have a top rung, and a borrower that took its points —
/// the state `quality_measure_reuse` leaves behind before the check runs.
fn a_loan(
    state: &vrcast_studio_lib::commands::AppState,
    film: &str,
) -> vrcast_studio_lib::store::measurements::Run {
    use vrcast_studio_lib::domain::measured_ladder::Point;
    use vrcast_studio_lib::store::measurements::{begin, lend, record, Material, Run};

    let a_run = |key: &str, path: &str| Run {
        source_key: key.to_owned(),
        codec: String::from("h264"),
        source_path: path.to_owned(),
        width: 1280,
        height: 720,
        fps: 30,
        source_bitrate_bps: 8_000_000,
        heavier_codec: false,
        native_height: None,
        anchor_mbps: 6,
        chunk_starts: vec![0, 20, 40],
        chunk_s: 10,
        borrowed_from: None,
        check_pending: false,
        donor_anchor_mbps: None,
        shape: None,
        material: Some(Material {
            codec: String::from("h264"),
            pix_fmt: String::from("yuv420p"),
            color_transfer: None,
            duration_s: 60.0,
            peak_bps: Some(9_000_000),
        }),
    };
    let donor = a_run("1:donor.mp4", "F:/films/donor.mp4");
    begin(&state.db, &donor).unwrap();
    for (bitrate_mbps, vmaf) in [(2, 90.0), (4, 94.0), (6, 96.5), (8, 97.5)] {
        record(
            &state.db,
            &donor.source_key,
            "h264",
            &Point {
                bitrate_mbps,
                height: 720,
                actual_bps: bitrate_mbps * 1_000_000,
                vmaf,
            },
            std::time::Duration::from_secs(1),
        )
        .unwrap();
    }
    let borrowed = lend(
        &state.db,
        &donor.source_key,
        "h264",
        &a_run("2:borrower.mp4", film),
    )
    .expect("the loan was refused");
    assert!(borrowed.check_pending);
    borrowed
}

#[tokio::test]
async fn a_cancelled_task_does_not_measure_the_loans_cell_and_leaves_the_loan_provisional() {
    use std::sync::Arc;
    use vrcast_studio_lib::media::ffmpeg;
    use vrcast_studio_lib::store::db::Db;
    use vrcast_studio_lib::tasks::engine::TaskContext;

    // The cancel is looked at before the first chunk, but the encoder binary is located
    // before that — without one nothing here gets as far as the question.
    if ffmpeg::locate("ffmpeg").is_err() {
        eprintln!("SKIPPED: no bundled FFmpeg. Run `npm run ffmpeg` for this to check anything.");
        return;
    }

    let state = state();
    // A film that is not there: before T670(1) the cell was measured under a token nobody
    // held, so it went on to encode — and ended `LADDER_NOT_MEASURED` on a missing file
    // rather than `TASK_CANCELLED`.
    let borrowed = a_loan(&state, "F:/nowhere/t670-borrower.mp4");
    let task = TaskContext::detached(Arc::new(Db::open_in_memory().unwrap()));
    task.cancel_token().cancel();

    let err = vrcast_studio_lib::commands::quality::held(
        &state,
        &borrowed,
        "1:donor.mp4",
        &vrcast_studio_lib::media::encoders::Encoder::Software,
        &task,
    )
    .await
    .expect_err("a cancelled check reported a verdict");
    assert_eq!(err.code, ErrorCode::TaskCancelled);

    // Not taken back (a cancel is not a verdict) and not vouched for either.
    let still = vrcast_studio_lib::store::measurements::run(&state.db, "2:borrower.mp4", "h264")
        .unwrap()
        .expect("a cancelled check threw the loan away");
    assert!(still.check_pending, "a cancelled check cleared the mark");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancel_during_the_loans_check_stops_it_between_chunks() {
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use vrcast_studio_lib::media::ffmpeg;
    use vrcast_studio_lib::store::db::Db;
    use vrcast_studio_lib::tasks::engine::TaskContext;

    let Ok(ff) = ffmpeg::locate("ffmpeg") else {
        eprintln!("SKIPPED: no bundled FFmpeg. Run `npm run ffmpeg` for this to check anything.");
        return;
    };
    let dir = std::env::temp_dir().join(format!("vrcast-t670-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    let film = dir.join("borrower.mp4");
    let made = std::process::Command::new(&ff)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=1280x720:rate=30",
            "-t",
            "60",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&film)
        .output()
        .expect("could not run the bundled FFmpeg");
    assert!(made.status.success());

    let state = state();
    let borrowed = a_loan(&state, &film.to_string_lossy());
    let task = TaskContext::detached(Arc::new(Db::open_in_memory().unwrap()));

    // Pressed while the first chunk is being encoded.
    let pressing = task.cancel_token();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        pressing.cancel();
    });
    let started = Instant::now();
    let outcome = vrcast_studio_lib::commands::quality::held(
        &state,
        &borrowed,
        "1:donor.mp4",
        &vrcast_studio_lib::media::encoders::Encoder::Software,
        &task,
    )
    .await;
    let took = started.elapsed();

    let err = outcome.expect_err("a check cancelled mid-way reported a verdict");
    assert_eq!(err.code, ErrorCode::TaskCancelled, "{err:?}");
    // One chunk's encode at most — the cell is three encodes and three scorings.
    assert!(
        took < Duration::from_secs(60),
        "the cancel was not heard until the cell was done: {took:?}"
    );
    let still = vrcast_studio_lib::store::measurements::run(&state.db, "2:borrower.mp4", "h264")
        .unwrap()
        .expect("a cancelled check threw the loan away");
    assert!(still.check_pending);
    let _ = std::fs::remove_dir_all(&dir);
}
