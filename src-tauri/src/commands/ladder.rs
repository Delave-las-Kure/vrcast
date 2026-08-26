//! T193 — the three commands a ladder is worked out with (FR-040, FR-041, FR-044).
//!
//! Contract: `contracts/ipc-commands.md`, the quality ladder section.
//!
//! **Two of the three are pure functions and stay that way.** Checking runs on every edit a
//! person makes (FR-044), and a check that reached for a file — let alone a server — would
//! either lag behind the typing or stop it. What they need to know is passed in.
//!
//! The one that is not pure is [`api::ladder_plan`], and only because it has to look up
//! whether this film has been measured. A measured ladder is the real one; the formula is a
//! preview of where a measurement would look (R-21), and the answer says plainly which of
//! the two is being handed back.

use serde::{Deserialize, Serialize};

use super::error::{AppError, ErrorCode, Result};
use crate::domain::ladder::{self, Layout, NotBuildable, Objection, Plan, Rung, SourceFacts};
use crate::domain::wording::Detail;
use crate::media::{encoders, ffmpeg, measure, probe_complexity};
use crate::store::measurements;

/// What the interface sends to have a ladder worked out.
#[derive(Debug, Clone, Deserialize)]
pub struct LadderRequest {
    pub path: String,
    /// The codec the ladder is for. A measurement does not carry between them.
    #[serde(default = "h264")]
    pub codec: String,
    /// The height the material really has, when it was upscaled. Told by the person.
    pub native_height: Option<u32>,
    /// What the person says the picture is, when they know better than a guess.
    pub declared_layout: Option<Layout>,
    #[serde(default = "yes")]
    pub prefer_hardware: bool,
}

fn h264() -> String {
    String::from("h264")
}

fn yes() -> bool {
    true
}

/// Where a ladder's rungs came from.
///
/// Not decoration: it decides whether the ladder may be built at all (FR-141), and a person
/// looking at rungs deserves to know whether anybody has actually looked at what they are
/// worth on this film.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LadderSource {
    /// From a measurement of this material.
    Measured,
    /// From a measurement of another file, lent to this one.
    Borrowed,
    /// From the formula. A preview of where to measure, not a ladder to build.
    Formula,
}

/// A ladder as the interface receives it.
#[derive(Debug, Clone, Serialize)]
pub struct LadderPreview {
    pub plan: Plan,
    pub from: LadderSource,
    /// What the source turned out to be.
    pub source: SourceFacts,
    /// What the complexity probe found, when it ran. `None` when the ladder came from a
    /// measurement instead and the probe was not needed.
    pub anchor_mbps: Option<u64>,
    /// What the checker says about these rungs, so that an interface has it without asking
    /// again.
    pub verdict: LadderVerdict,
    pub notices: Vec<Detail>,
}

/// What a person needs to be checked on every edit (FR-044).
#[derive(Debug, Clone, Deserialize)]
pub struct LadderCheck {
    pub rungs: Vec<Rung>,
    pub source: SourceFacts,
}

/// Everything wrong with a ladder, in two kinds.
///
/// **The two are kept apart because they mean different things.** An objection says the
/// ladder is unsound — a rung above the source, a buffer that will let peaks through, a
/// hole a viewer would fall through. `not_buildable` says nobody has measured it. A ladder
/// can be perfectly sound and still be a guess.
#[derive(Debug, Clone, Serialize)]
pub struct LadderVerdict {
    pub objections: Vec<Objection>,
    pub not_buildable: Option<NotBuildable>,
}

impl LadderVerdict {
    fn of(rungs: &[Rung], source: &SourceFacts) -> Self {
        Self {
            objections: ladder::validate(rungs, source, source.fps),
            not_buildable: ladder::buildable(rungs).err(),
        }
    }
}

pub mod api {
    use super::*;

    /// Measure the source: what it averages, where it peaks (FR-040).
    ///
    /// Before the ladder is worked out, not after: a connection has to hold the peak rather
    /// than the average, and a film that averages 8 Mbit/s and reaches 40 in one scene
    /// freezes everyone whose line is under 40 when that scene arrives.
    pub async fn ladder_measure(path: &str) -> Result<measure::Measured> {
        measure::measure(std::path::Path::new(path))
            .await
            .map_err(|e| AppError::new(ErrorCode::FfmpegBroken).with_cause(e))
    }

    /// Work out a ladder for this film.
    ///
    /// Hands back the measured ladder when this material has been measured, and the
    /// formula's preview when it has not — saying which, every time.
    pub async fn ladder_plan(
        state: &super::super::AppState,
        request: &LadderRequest,
    ) -> Result<LadderPreview> {
        let probed = super::super::api::source_probe(&request.path).await?;
        let source = SourceFacts {
            width: probed.width,
            height: probed.height,
            fps: probed.fps,
            bitrate_bps: probed.bitrate_bps,
            heavier_codec: probed.video_codec.eq_ignore_ascii_case("hevc"),
            native_height: request.native_height,
        };

        // A measurement of this material, if there is one. This is the ladder; everything
        // below is what happens when there is not one.
        if let Some(measured) = measured_plan(state, request, &source)? {
            return Ok(measured);
        }

        let (encoder, mut notices) = pick_encoder(request.prefer_hardware).await?;
        let probe = probe_complexity::probe(
            std::path::Path::new(&request.path),
            probed.duration_s,
            &encoder,
        )
        .await;
        notices.extend(probe.notice.clone());

        let plan = ladder::plan(probe.measured_bps, &source, request.declared_layout).map_err(
            |refusal| match refusal {
                ladder::Refusal::SourceBitrateTooLow { .. } => {
                    AppError::new(ErrorCode::InvalidInput).with_cause(refusal_text(refusal))
                }
            },
        )?;

        Ok(LadderPreview {
            verdict: LadderVerdict::of(&plan.rungs, &source),
            plan,
            from: LadderSource::Formula,
            source,
            anchor_mbps: probe.measured_bps.map(|bps| (bps / 1_000_000).max(1)),
            notices,
        })
    }

    /// Check rungs a person has edited (FR-044).
    ///
    /// A pure function, and called on every edit rather than at the end: learning that a
    /// rung is impossible after agreeing to hours of encoding is learning it too late.
    pub async fn ladder_validate(check: &LadderCheck) -> Result<LadderVerdict> {
        Ok(LadderVerdict::of(&check.rungs, &check.source))
    }
}

/// The measured ladder for this material, when there is one.
fn measured_plan(
    state: &super::AppState,
    request: &LadderRequest,
    source: &SourceFacts,
) -> Result<Option<LadderPreview>> {
    let Ok(key) = measurements::key_for(std::path::Path::new(&request.path)) else {
        return Ok(None);
    };
    let Ok(Some(run)) = measurements::run(&state.db, &key, &request.codec) else {
        return Ok(None);
    };
    let points = measurements::points(&state.db, &key, &request.codec).unwrap_or_default();
    if points.is_empty() {
        return Ok(None);
    }

    let chosen = crate::domain::measured_ladder::select(
        &points,
        crate::domain::measured_ladder::TARGET_VMAF,
        crate::domain::measured_ladder::VMAF_STEP,
    );
    let borrowed = run.borrowed_from.is_some();
    let plan = ladder::from_measurement(&chosen.rungs, source, request.declared_layout, borrowed)
        .map_err(|refusal| {
        AppError::new(ErrorCode::InvalidInput).with_cause(refusal_text(refusal))
    })?;

    let mut notices = Vec::new();
    if let Some(from) = &run.borrowed_from {
        notices.push(
            Detail::new(crate::domain::wording::DetailCode::NoticeMeasurementBorrowed)
                .with("from", from.clone()),
        );
    }

    Ok(Some(LadderPreview {
        verdict: LadderVerdict::of(&plan.rungs, source),
        plan,
        from: if borrowed {
            LadderSource::Borrowed
        } else {
            LadderSource::Measured
        },
        source: *source,
        anchor_mbps: None,
        notices,
    }))
}

fn refusal_text(refusal: ladder::Refusal) -> String {
    match refusal {
        ladder::Refusal::SourceBitrateTooLow { bitrate_bps } => format!(
            "the source holds {bitrate_bps} bit/s, which is under a whole megabit: \
             there is nothing here to build a ladder out of"
        ),
    }
}

async fn pick_encoder(prefer_hardware: bool) -> Result<(encoders::Encoder, Vec<Detail>)> {
    let info = ffmpeg::probe_self()
        .await
        .map_err(|e| AppError::new(ErrorCode::FfmpegBroken).with_cause(e))?;
    let choice = encoders::choose(&info.hardware, info.has_x264, prefer_hardware)
        .map_err(|_| AppError::new(ErrorCode::NoHwEncoder))?;
    Ok((choice.encoder, choice.notice.into_iter().collect()))
}

pub mod ipc {
    use super::*;
    use tauri::State;

    #[tauri::command]
    pub async fn ladder_measure(path: String) -> Result<measure::Measured> {
        api::ladder_measure(&path).await
    }

    #[tauri::command]
    pub async fn ladder_plan(
        state: State<'_, super::super::AppState>,
        request: LadderRequest,
    ) -> Result<LadderPreview> {
        api::ladder_plan(&state, &request).await
    }

    #[tauri::command]
    pub async fn ladder_validate(check: LadderCheck) -> Result<LadderVerdict> {
        api::ladder_validate(&check).await
    }
}
