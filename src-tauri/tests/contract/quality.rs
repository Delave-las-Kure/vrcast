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
            prefer_hardware: true,
            batch: None,
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
            prefer_hardware: true,
            batch: None,
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
            prefer_hardware: true,
            batch: None,
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
