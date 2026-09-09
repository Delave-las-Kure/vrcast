//! T189 — contract tests for the quality-ladder commands.
//!
//! Contract: `contracts/ipc-commands.md`, "Наборы качеств".
//!
//! Only what is visible from outside: the shape of the answer, and which complaint carries
//! which code. A code is not a detail — it decides whether the interface highlights one
//! field or refuses the whole build — and a typo in one is not a failure anybody notices
//! until a person is looking at a screen that says nothing.
//!
//! `ladder_build` and `ladder_verify` are not here yet: they arrive with T194–T198, and a
//! contract test for a command that does not exist is a comment pretending to be a check.

use vrcast_studio_lib::commands::error::ErrorCode;
use vrcast_studio_lib::commands::ladder::{
    api as ladder, LadderCheck, LadderRequest, LadderSource,
};
use vrcast_studio_lib::domain::ladder::{
    plan, NotBuildable, Objection, Quality, Rung, SourceFacts,
};
use vrcast_studio_lib::media::{encoders, ffmpeg, probe_complexity};

use super::support::state;

/// Is the bundled build present? Half these checks have nothing to do without it.
fn has_ffmpeg() -> bool {
    if ffmpeg::locate("ffprobe").is_ok() && ffmpeg::locate("ffmpeg").is_ok() {
        return true;
    }
    eprintln!("SKIPPED: no bundled FFmpeg. Run `npm run ffmpeg` for this check to check anything.");
    false
}

fn source() -> SourceFacts {
    SourceFacts {
        width: 3840,
        height: 2160,
        fps: 24,
        bitrate_bps: 60_000_000,
        heavier_codec: false,
        native_height: None,
    }
}

fn a_rung(index: usize, bitrate_bps: u64, height: u32, level: &str) -> Rung {
    Rung {
        index,
        bitrate_bps,
        maxrate_bps: bitrate_bps * 11 / 10,
        bufsize_bps: bitrate_bps * 11 / 10,
        width: height * 16 / 9,
        height,
        level: level.to_owned(),
        reasons: Vec::new(),
        quality: Quality::MeasuredHere { vmaf_x100: 9500 },
    }
}

#[tokio::test]
async fn measuring_a_missing_file_is_a_failure_with_a_code_rather_than_a_panic() {
    if !has_ffmpeg() {
        return;
    }
    let err = ladder::ladder_measure("F:/nowhere/no-such-film.mp4")
        .await
        .expect_err("a file that does not exist was measured");
    assert_eq!(err.code, ErrorCode::FfmpegBroken);
    assert!(
        !format!("{err:?}").is_empty(),
        "the failure carries nothing a person could be shown"
    );
}

#[tokio::test]
async fn planning_a_missing_file_says_so_rather_than_offering_a_ladder() {
    if !has_ffmpeg() {
        return;
    }
    let state = state();
    let err = ladder::ladder_plan(
        &state,
        &LadderRequest {
            path: String::from("F:/nowhere/no-such-film.mp4"),
            codec: String::from("h264"),
            native_height: None,
            declared_layout: None,
            measured_peak_bps: None,
            prefer_hardware: true,
        },
    )
    .await
    .expect_err("a ladder was planned for a file that does not exist");
    assert!(
        matches!(err.code, ErrorCode::InvalidInput | ErrorCode::FfmpegBroken),
        "the wrong code came back: {:?}",
        err.code
    );
}

/// A short, real clip to plan a ladder from (T522). Software x264 throughout — not
/// hardware — so the probe's own answer is the same on every machine this runs on,
/// which is exactly what `a_supplied_measured_peak_overrules_the_probe_s_own_reading`
/// below needs to hold still while it changes the one thing T522 is about.
async fn a_real_clip() -> std::path::PathBuf {
    let ff = ffmpeg::locate("ffmpeg").expect("no bundled ffmpeg");
    let dir = std::env::temp_dir().join("vrcast-t522-fixture");
    std::fs::create_dir_all(&dir).expect("could not make a working directory");
    let src = dir.join("source.mp4");
    if src.is_file() {
        return src;
    }
    let made = std::process::Command::new(&ff)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "mandelbrot=size=960x540:rate=24",
            "-t",
            "12",
            "-c:v",
            "libx264",
            "-b:v",
            "16M",
            "-minrate",
            "16M",
            "-maxrate",
            "16M",
            "-bufsize",
            "32M",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&src)
        .output()
        .expect("could not run the bundled FFmpeg");
    assert!(made.status.success(), "could not prepare the fixture clip");
    src
}

/// T522, rule 1: when `LadderRequest.measured_peak_bps` is supplied, it — not the
/// complexity probe's own few-seconds-of-trial-encodes reading — determines
/// `plan.anchor_bps`/`anchor_mbps`, unconditionally, no closeness threshold.
///
/// Against a real clip and a real probe (`probe_complexity::probe`), not an invented
/// `Probed`: the whole point of T522 is that a real, honest measurement beats the
/// probe's own guess, so the guess has to be real too or the comparison proves nothing.
#[tokio::test]
async fn a_supplied_measured_peak_overrules_the_probe_s_own_reading() {
    if !has_ffmpeg() {
        return;
    }
    let src = a_real_clip().await;
    let path = src.to_string_lossy().into_owned();

    // What the probe finds on its own, so the test can show the supplied peak actually
    // changed the outcome rather than coincidentally landing on the same rung.
    let state = state();
    let baseline = ladder::ladder_plan(
        &state,
        &LadderRequest {
            path: path.clone(),
            codec: String::from("h264"),
            native_height: None,
            declared_layout: None,
            measured_peak_bps: None,
            prefer_hardware: false,
        },
    )
    .await
    .expect("the baseline ladder would not plan");
    assert_eq!(
        baseline.from,
        LadderSource::Formula,
        "an unmeasured file was not planned from the formula"
    );

    // Below `source_cap_mbps` for this clip (~16 Mbit/s average) so the supplied number
    // is not silently trimmed back down to the cap — and deliberately unlike whatever
    // the probe found above, so a passing test proves the override rather than a
    // coincidence.
    let supplied_peak_bps: u64 = 9_000_000;
    assert_ne!(
        baseline.anchor_mbps,
        Some(supplied_peak_bps / 1_000_000),
        "the fixture's probe reading collided with the test's supplied number by chance; \
         change supplied_peak_bps so the two differ"
    );

    let overridden = ladder::ladder_plan(
        &state,
        &LadderRequest {
            path,
            codec: String::from("h264"),
            native_height: None,
            declared_layout: None,
            measured_peak_bps: Some(supplied_peak_bps),
            prefer_hardware: false,
        },
    )
    .await
    .expect("the overridden ladder would not plan");

    assert_eq!(
        overridden.anchor_mbps,
        Some(supplied_peak_bps / 1_000_000),
        "the supplied measured_peak_bps did not become the anchor: {:?}",
        overridden.anchor_mbps
    );
    assert_eq!(
        overridden.plan.anchor_bps,
        (supplied_peak_bps / 1_000_000) * 1_000_000,
        "plan.anchor_bps disagrees with anchor_mbps — the two were computed from \
         different numbers, exactly the desync T522 exists to prevent"
    );
    let top = overridden
        .plan
        .rungs
        .iter()
        .map(|r| r.bitrate_bps)
        .max()
        .expect("the overridden plan chose no rungs at all");
    assert_eq!(
        top,
        (supplied_peak_bps / 1_000_000) * 1_000_000,
        "the top rung was not built from the supplied peak: {top}"
    );
}

/// T522: `measured_peak_bps: None` leaves the old behaviour untouched — the anchor is
/// still whatever the complexity probe itself found, exactly as before this change.
#[tokio::test]
async fn a_missing_measured_peak_leaves_the_probe_s_own_reading_in_charge() {
    if !has_ffmpeg() {
        return;
    }
    let src = a_real_clip().await;
    let path = src.to_string_lossy().into_owned();
    let state = state();

    let request = LadderRequest {
        path: path.clone(),
        codec: String::from("h264"),
        native_height: None,
        declared_layout: None,
        measured_peak_bps: None,
        prefer_hardware: false,
    };
    let preview = ladder::ladder_plan(&state, &request)
        .await
        .expect("the ladder would not plan");

    // Worked out the same way `ladder_plan` itself works it out, independently, so the
    // assertion is against the probe's real answer and not a number copied from a
    // previous run of this test.
    let probed = vrcast_studio_lib::commands::api::source_probe(&path)
        .await
        .expect("the fixture clip would not probe");
    let probe =
        probe_complexity::probe(&src, probed.duration_s, &encoders::Encoder::Software).await;

    assert_eq!(
        preview.anchor_mbps,
        probe.measured_bps.map(|bps| (bps / 1_000_000).max(1)),
        "with no measured_peak_bps supplied, the anchor no longer matches the probe's \
         own reading — the fallback path changed behaviour"
    );
}

#[tokio::test]
async fn checking_is_a_pure_function_and_answers_in_the_shape_the_contract_names() {
    // Called on every edit a person makes (FR-044): it must not reach for a file, a server
    // or anything else that could be slow or absent. If this ever needs a fixture, the
    // check has stopped being pure and the interface will lag behind the typing.
    let src = source();
    let sound = plan(Some(22_000_000), &src, None).expect("a sound source was refused");

    let verdict = ladder::ladder_validate(&LadderCheck {
        rungs: sound.rungs.clone(),
        source: src,
    })
    .await
    .expect("checking a sound ladder failed");

    assert!(
        verdict.objections.is_empty(),
        "the planner's own ladder was objected to: {:?}",
        verdict.objections
    );
    // ...and it is still not buildable, because nobody measured it. The two answers are
    // separate on purpose: this ladder is sound *and* a guess.
    assert!(
        matches!(
            verdict.not_buildable,
            Some(NotBuildable::RungsNotMeasured { .. })
        ),
        "a ladder out of the formula was declared buildable"
    );
}

#[tokio::test]
async fn every_objection_the_contract_names_can_actually_be_raised() {
    // `RUNG_ABOVE_SOURCE`, `BUFSIZE_TOO_LARGE`, `LEVEL_EXCEEDED` — the contract lists them,
    // and a code nothing ever produces is a promise to an interface that will never be
    // kept.
    let src = source();

    let above = ladder::ladder_validate(&LadderCheck {
        rungs: vec![a_rung(0, 90_000_000, 2160, "5.1")],
        source: src,
    })
    .await
    .expect("checking failed");
    assert!(
        above
            .objections
            .iter()
            .any(|o| matches!(o, Objection::RungAboveSource { .. })),
        "a rung above the source passed: {:?}",
        above.objections
    );

    let mut fat = a_rung(0, 22_000_000, 2160, "5.1");
    fat.maxrate_bps = 24_000_000;
    fat.bufsize_bps = 60_000_000;
    let buffered = ladder::ladder_validate(&LadderCheck {
        rungs: vec![fat],
        source: src,
    })
    .await
    .expect("checking failed");
    assert!(
        buffered
            .objections
            .iter()
            .any(|o| matches!(o, Objection::BufsizeTooLarge { .. })),
        "a buffer that lets peaks through passed: {:?}",
        buffered.objections
    );

    // The level says 4.1 and the variant does not fit it. The complaint has to say WHICH of
    // the two limits it breaks — per frame or per second — because they are fixed by
    // different means: one by the frame size, the other by the frame rate.
    let wrong_level = ladder::ladder_validate(&LadderCheck {
        rungs: vec![a_rung(0, 22_000_000, 2160, "4.1")],
        source: src,
    })
    .await
    .expect("checking failed");
    let level = wrong_level
        .objections
        .iter()
        .find_map(|o| match o {
            Objection::LevelExceeded { limits, .. } => Some(limits),
            _ => None,
        })
        .expect("a variant that does not fit its level passed");
    assert!(
        !level.is_empty(),
        "the complaint does not say which limit was broken"
    );
}

#[tokio::test]
async fn a_ladder_with_a_hole_in_it_is_objected_to_by_the_step_rule() {
    let src = source();
    let verdict = ladder::ladder_validate(&LadderCheck {
        rungs: vec![
            a_rung(0, 20_000_000, 2160, "5.1"),
            a_rung(1, 4_000_000, 1080, "4.0"),
        ],
        source: src,
    })
    .await
    .expect("checking failed");
    assert!(
        verdict
            .objections
            .iter()
            .any(|o| matches!(o, Objection::BadStep { .. })),
        "a fivefold hole passed: {:?}",
        verdict.objections
    );
}

#[tokio::test]
async fn rungs_out_of_order_are_objected_to_rather_than_quietly_sorted() {
    // Sorting them would hide a person's mistake and then build something they did not ask
    // for. The order is theirs to fix.
    let src = source();
    let verdict = ladder::ladder_validate(&LadderCheck {
        rungs: vec![
            a_rung(0, 8_000_000, 1440, "5.0"),
            a_rung(1, 16_000_000, 2160, "5.1"),
        ],
        source: src,
    })
    .await
    .expect("checking failed");
    assert!(
        verdict
            .objections
            .iter()
            .any(|o| matches!(o, Objection::OutOfOrder { .. })),
        "an ascending ladder passed: {:?}",
        verdict.objections
    );
}

#[tokio::test]
async fn an_empty_ladder_is_refused_as_empty_rather_than_as_unmeasured() {
    // The two want different things said to a person: one is "there is nothing here", the
    // other "there is something here that nobody has looked at".
    let verdict = ladder::ladder_validate(&LadderCheck {
        rungs: Vec::new(),
        source: source(),
    })
    .await
    .expect("checking failed");
    assert!(verdict.objections.is_empty());
    assert_eq!(verdict.not_buildable, Some(NotBuildable::NoRungs));
}

#[tokio::test]
async fn a_measured_ladder_is_buildable_and_says_where_it_came_from() {
    let src = source();
    let measured = vec![
        a_rung(0, 22_000_000, 2160, "5.1"),
        a_rung(1, 12_000_000, 1440, "5.0"),
        a_rung(2, 7_000_000, 1080, "4.2"),
    ];
    let verdict = ladder::ladder_validate(&LadderCheck {
        rungs: measured,
        source: src,
    })
    .await
    .expect("checking failed");
    assert!(
        verdict.objections.is_empty(),
        "a sound measured ladder was objected to: {:?}",
        verdict.objections
    );
    assert_eq!(verdict.not_buildable, None);

    // And the three ways a ladder can arrive are distinct values rather than a flag: an
    // interface has to tell "measured here" from "borrowed" from "a guess".
    assert_ne!(LadderSource::Measured, LadderSource::Borrowed);
    assert_ne!(LadderSource::Borrowed, LadderSource::Formula);
}

// ---------- T571: BuildRequest.confirmed ----------

/// A `BuildRequest` payload with no `confirmed` key at all — the exact shape `ipc.ts` sends
/// today, since the frontend side of T571 is a separate ticket and has not been touched.
///
/// **The precise form the field takes, for the record.** `confirmed: bool`, `#[serde(default)]`
/// — absent on the wire reads as `false`, exactly like `UploadRequest.confirmed`, which this
/// was modelled on field-for-field.
#[test]
fn confirmed_is_false_by_default_when_the_field_is_absent() {
    let json = serde_json::json!({
        "server_id": "srv",
        "path": "F:/films/film.mp4",
        "slug": "demo",
        "rungs": [],
        "audio_track": 0,
        "prefer_hardware": true
        // no "confirmed" key at all — nothing sent it yet on the wire.
    });
    let request: vrcast_studio_lib::commands::ladder::BuildRequest = serde_json::from_value(json)
        .expect(
            "a BuildRequest with no confirmed field at all did not read — the default was \
             not applied, so an old caller (or the frontend before its own T571 ticket \
             lands) would fail to submit a build at all",
        );
    assert!(
        !request.confirmed,
        "an absent confirmed field defaulted to true — that would silently waive the \
         active-viewers warning for every caller that has not been updated to send it"
    );
}

/// The same field, explicitly sent both ways — the shape a frontend that HAS been updated
/// for T571 would send.
#[test]
fn confirmed_reads_back_explicitly_both_ways() {
    for sent in [true, false] {
        let json = serde_json::json!({
            "server_id": "srv",
            "path": "F:/films/film.mp4",
            "slug": "demo",
            "rungs": [],
            "audio_track": 0,
            "prefer_hardware": true,
            "confirmed": sent,
        });
        let request: vrcast_studio_lib::commands::ladder::BuildRequest =
            serde_json::from_value(json).expect("an explicit confirmed value did not read");
        assert_eq!(
            request.confirmed, sent,
            "an explicitly sent confirmed: {sent} was not read back as {sent}"
        );
    }
}
