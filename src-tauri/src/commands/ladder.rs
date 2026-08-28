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
use crate::domain::wording::{Detail, DetailCode};
use crate::media::{encoders, ffmpeg, measure, probe_complexity};
use crate::store::measurements;
use crate::tasks::state::TaskKind;

/// What the interface sends to have a ladder worked out.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// What the interface sends to build a quality set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildRequest {
    pub server_id: String,
    /// The source on this machine that the variants are made from.
    pub path: String,
    /// The medium's own directory on the server.
    pub slug: String,
    /// The rungs, as the person has them on screen — measured or edited.
    pub rungs: Vec<Rung>,
    /// Which audio track to keep.
    #[serde(default)]
    pub audio_track: usize,
    #[serde(default = "yes")]
    pub prefer_hardware: bool,
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

    /// Build the set: prepare each variant, send it, cut it, and check it is served.
    ///
    /// Returns a task number at once (FR-080). Everything that can be refused quickly is
    /// refused here, before a task exists — above all an unmeasured ladder, because
    /// building one is hours of encoding spent on a guess (FR-141).
    pub async fn ladder_build(
        state: &super::super::AppState,
        request: BuildRequest,
    ) -> Result<String> {
        ladder::buildable(&request.rungs).map_err(|why| match why {
            ladder::NotBuildable::NoRungs => AppError::new(ErrorCode::InvalidInput),
            ladder::NotBuildable::RungsNotMeasured { indexes } => {
                AppError::new(ErrorCode::LadderNotMeasured).with_cause(format!(
                    "rungs {}",
                    indexes
                        .iter()
                        .map(|i| (i + 1).to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            }
        })?;

        let profile = super::super::library::api::profile_of(state, &request.server_id)?;
        let source = super::super::api::source_probe(&request.path).await?;
        let (encoder, _) = pick_encoder(request.prefer_hardware).await?;

        let master_url = crate::domain::links::for_path(
            &profile.domain,
            None,
            &format!("{}/master.m3u8", request.slug),
        )
        .origin;
        // Somewhere local for a variant while it is being made. Beside the other working
        // files rather than beside the source: a person's film directory is theirs, and a
        // half-made variant appearing in it is alarming even when it is swept away after.
        let work_dir = std::env::temp_dir().join("vrcast-ladder");
        let secrets = state.secrets.clone();

        let task_id = state
            .tasks
            .submit(
                TaskKind::BuildLadder,
                Some(request.server_id.clone()),
                move |ctx| async move {
                    let conn = crate::server::gate::open(
                        secrets.as_ref(),
                        &profile,
                        crate::server::gate::Intent::Change,
                    )
                    .await?
                    .conn;
                    let job = crate::tasks::ladder_build::BuildJob {
                        conn: &conn,
                        video_dir: &profile.video_dir,
                        owner: &format!("{}:{}", profile.user, profile.user),
                        slug: &request.slug,
                        source: &source,
                        rungs: &request.rungs,
                        encoder: &encoder,
                        audio_track: request.audio_track,
                        master_url: &master_url,
                        work_dir: &work_dir,
                    };
                    let outcome = crate::tasks::ladder_build::run(&job, &ctx).await;
                    conn.close().await;
                    outcome.map(|_| ()).map_err(build_error)
                },
            )
            .await?;
        Ok(task_id)
    }

    /// Ask the serving for every variant of a set (FR-047).
    ///
    /// Separate from building so that a set can be asked about at any time — a variant
    /// can stop being served long after it was made, and nothing else would notice.
    pub async fn ladder_verify(
        state: &super::super::AppState,
        server_id: &str,
        slug: &str,
    ) -> Result<crate::server::hls_verify::LadderVerdict> {
        let profile = super::super::library::api::profile_of(state, server_id)?;
        let master_url =
            crate::domain::links::for_path(&profile.domain, None, &format!("{slug}/master.m3u8"))
                .origin;

        // What the description itself names is what is expected: asking for a number from
        // elsewhere would let a set with a rung missing from its own description pass.
        let verdict = crate::server::hls_verify::verify(&master_url, 0)
            .await
            .map_err(|e| AppError::new(ErrorCode::DomainNotServing).with_cause(e))?;
        let expected = verdict.variants_in_master;
        let verdict = crate::server::hls_verify::LadderVerdict {
            variants_expected: expected,
            ..verdict
        };

        if !verdict.ok() {
            return Err(
                AppError::new(ErrorCode::LadderIncomplete).with_cause(verdict.broken().join(", "))
            );
        }
        Ok(verdict)
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

fn build_error(e: crate::tasks::ladder_build::BuildError) -> AppError {
    use crate::tasks::ladder_build::BuildError as E;
    match e {
        E::Cancelled => AppError::new(ErrorCode::TaskCancelled),
        E::NotBuildable(_) => AppError::new(ErrorCode::LadderNotMeasured),
        // The one failure that names names: a person is owed "the lower rung" rather
        // than "something went wrong", because the two ask for different work.
        E::Incomplete(which) => {
            AppError::new(ErrorCode::LadderIncomplete).with_cause(which.join(", "))
        }
        E::NotEnoughSpace {
            needed,
            free,
            short_by,
            rungs,
        } => AppError::new(ErrorCode::RemoteDiskFull)
            .with_detail(
                Detail::new(DetailCode::LadderNotEnoughSpace)
                    .with("short_by", short_by)
                    .with("needed", needed)
                    .with("free", free)
                    .with("rungs", rungs as u64),
            )
            .with_cause(format!("short_by={short_by}")),
        other => AppError::new(ErrorCode::Internal).with_cause(other),
    }
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
    pub async fn ladder_build(
        state: State<'_, super::super::AppState>,
        request: BuildRequest,
    ) -> Result<String> {
        api::ladder_build(&state, request).await
    }

    #[tauri::command]
    pub async fn ladder_verify(
        state: State<'_, super::super::AppState>,
        server_id: String,
        slug: String,
    ) -> Result<crate::server::hls_verify::LadderVerdict> {
        api::ladder_verify(&state, &server_id, &slug).await
    }

    #[tauri::command]
    pub async fn ladder_validate(check: LadderCheck) -> Result<LadderVerdict> {
        api::ladder_validate(&check).await
    }
}
