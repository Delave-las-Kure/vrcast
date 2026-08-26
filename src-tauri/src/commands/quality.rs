//! T237, T238 — commands for measuring quality (FR-141, FR-146, FR-147).
//!
//! Contract: `contracts/ipc-commands.md`, the quality ladder section.
//!
//! A measurement is the longest thing this application does, so what can be refused is
//! refused before the task exists, and how long it will take is said before it is agreed
//! to rather than after (FR-147).

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::error::{AppError, DetailCode, ErrorCode, Result};
use crate::domain::chunks::{reference_chunks, CHUNK_S};
use crate::domain::ladder::SourceFacts;
use crate::domain::measure_grid::grid;
use crate::domain::wording::Detail;
use crate::media::{encoders, ffmpeg, measure, probe_complexity};
use crate::store::measurements::{self, LendRefusal, Run};
use crate::tasks::quality_measure::{self, MeasureJob};
use crate::tasks::state::TaskKind;

/// What the interface sends to have a film measured.
#[derive(Debug, Clone, Deserialize)]
pub struct MeasureRequest {
    pub path: String,
    /// The codec the ladder is being measured **for**. A measurement does not carry from
    /// one to another.
    #[serde(default = "h264")]
    pub codec: String,
    /// The height the material really has, when it was upscaled to its present size.
    /// Told by the person: no measurement finds it reliably.
    pub native_height: Option<u32>,
    /// False when the person asked for the processor on purpose.
    #[serde(default = "yes")]
    pub prefer_hardware: bool,
}

fn h264() -> String {
    String::from("h264")
}

fn yes() -> bool {
    true
}

/// What measuring this film will involve, before anything is started.
#[derive(Debug, Clone, Serialize)]
pub struct MeasurePreview {
    pub source_key: String,
    /// How many points of the grid there are, and how many are already answered.
    pub points: usize,
    pub already_measured: usize,
    /// Roughly how long the rest will take, in seconds.
    ///
    /// **Roughly, and said so.** A number that is honestly approximate beats no number:
    /// without one a person cannot tell whether to start this before dinner or before
    /// bed.
    pub about_seconds: u64,
    /// How many timed points on this machine the estimate rests on.
    ///
    /// Zero means it rests on the cost model this project measured on its own machine,
    /// which is a different machine. The interface says which, because the difference
    /// between twenty minutes and two hours is the whole decision.
    pub estimate_from_points: usize,
    /// Where the reference chunks fall, in seconds into the film.
    pub chunk_starts: Vec<u64>,
    pub anchor_mbps: u64,
    pub encoder: encoders::Encoder,
    pub notices: Vec<Detail>,
}

/// A measurement as the interface sees it.
#[derive(Debug, Clone, Serialize)]
pub struct MeasurementView {
    pub run: Run,
    pub points: Vec<crate::domain::measured_ladder::Point>,
    /// The ladder these points choose, or nothing when the grid is still empty.
    pub selection: Option<crate::domain::measured_ladder::Selection>,
    /// The same ladder as rungs ready to be built — each carrying what it measured.
    pub ladder: Option<crate::domain::ladder::Plan>,
    pub notices: Vec<Detail>,
}

pub mod api {
    use super::*;

    /// What measuring this film would involve. Nothing is started.
    pub async fn quality_measure_preview(
        state: &super::super::AppState,
        request: &MeasureRequest,
    ) -> Result<MeasurePreview> {
        let (run, encoder, notices) = prepare(request).await?;
        let facts = facts_of(&run);
        let points = grid(&facts, run.anchor_mbps).len();
        let already = measurements::points(&state.db, &run.source_key, &run.codec)
            .map(|p| p.len())
            .unwrap_or(0);

        // What a point costs on the machine the model was measured on, corrected by
        // what this machine has really done — once it has done anything.
        let per_point = crate::domain::measure_grid::seconds_per_point(
            run.width,
            run.height,
            run.fps,
            run.chunk_s,
            run.chunk_starts.len(),
        );
        let (factor, from_points) = measurements::machine_factor(&state.db)
            .ok()
            .flatten()
            .unwrap_or((1.0, 0));

        Ok(MeasurePreview {
            source_key: run.source_key.clone(),
            points,
            already_measured: already,
            about_seconds: (points.saturating_sub(already) as f64 * per_point * factor) as u64,
            estimate_from_points: from_points,
            chunk_starts: run.chunk_starts.clone(),
            anchor_mbps: run.anchor_mbps,
            encoder,
            notices,
        })
    }

    /// Start measuring. Returns a task number immediately (FR-080).
    pub async fn quality_measure_start(
        state: &super::super::AppState,
        request: MeasureRequest,
    ) -> Result<String> {
        let (run, encoder, _) = prepare(&request).await?;
        let source = request.path.clone();
        let db = state.db.clone();

        let task_id = state
            .tasks
            .submit(TaskKind::MeasureQuality, None, move |ctx| async move {
                let job = MeasureJob {
                    source: Path::new(&source),
                    run: &run,
                    encoder: &encoder,
                    db: &db,
                };
                let outcome = quality_measure::run(&job, &ctx).await.map_err(to_error)?;
                ctx.report_important(1.0, DetailCode::StageDone);
                tracing::info!(
                    measured = outcome.measured,
                    total = outcome.total,
                    rungs = outcome.selection.rungs.len(),
                    "quality measured"
                );
                Ok(())
            })
            .await?;

        Ok(task_id)
    }

    /// What a measurement found, and the ladder it chooses.
    pub async fn quality_measure_result(
        state: &super::super::AppState,
        source_key: &str,
        codec: &str,
    ) -> Result<MeasurementView> {
        let run = measurements::run(&state.db, source_key, codec)
            .map_err(|e| AppError::new(ErrorCode::Internal).with_cause(e))?
            .ok_or_else(|| AppError::new(ErrorCode::MeasurementNotFound))?;
        let points = measurements::points(&state.db, source_key, codec)
            .map_err(|e| AppError::new(ErrorCode::Internal).with_cause(e))?;

        let mut notices = Vec::new();
        if let Some(from) = &run.borrowed_from {
            notices.push(
                Detail::new(DetailCode::NoticeMeasurementBorrowed).with("from", from.clone()),
            );
        }

        let selection = (!points.is_empty()).then(|| {
            crate::domain::measured_ladder::select(
                &points,
                crate::domain::measured_ladder::TARGET_VMAF,
                crate::domain::measured_ladder::VMAF_STEP,
            )
        });
        let ladder = selection.as_ref().and_then(|chosen| {
            crate::domain::ladder::from_measurement(
                &chosen.rungs,
                &facts_of(&run),
                None,
                run.borrowed_from.is_some(),
            )
            .ok()
        });

        Ok(MeasurementView {
            selection,
            ladder,
            run,
            points,
            notices,
        })
    }

    /// Every measurement kept — what the next episode can be offered.
    pub async fn quality_measurements(state: &super::super::AppState) -> Result<Vec<Run>> {
        measurements::all(&state.db).map_err(|e| AppError::new(ErrorCode::Internal).with_cause(e))
    }

    /// Lend the measurement of one film to another.
    ///
    /// For the next episode of a season this is usually right — the same source, the same
    /// upscale, the same encoder settings. It is **not** right across a season boundary or
    /// between different kinds of material, and the result is marked as borrowed either
    /// way, because a rung standing on somebody else's measurement is not a measured rung
    /// (FR-145).
    pub async fn quality_measure_reuse(
        state: &super::super::AppState,
        from_key: &str,
        request: MeasureRequest,
    ) -> Result<MeasurementView> {
        let (run, _, _) = prepare(&request).await?;
        match measurements::lend(&state.db, from_key, &run.codec, &run) {
            Ok(borrowed) => {
                quality_measure_result(state, &borrowed.source_key, &borrowed.codec).await
            }
            Err(LendRefusal::NothingToLend) => Err(AppError::new(ErrorCode::MeasurementNotFound)),
            Err(LendRefusal::DifferentMaterial) => {
                Err(AppError::new(ErrorCode::MeasurementDifferentMaterial))
            }
        }
    }

    /// Throw a measurement away, so that it is taken again.
    pub async fn quality_measure_forget(
        state: &super::super::AppState,
        source_key: &str,
        codec: &str,
    ) -> Result<()> {
        measurements::forget(&state.db, source_key, codec)
            .map_err(|e| AppError::new(ErrorCode::Internal).with_cause(e))
    }
}

/// Everything that has to be known before a measurement can begin.
///
/// Refusals happen here rather than inside the task: learning half an hour in that this
/// build cannot measure quality would waste the half hour.
async fn prepare(request: &MeasureRequest) -> Result<(Run, encoders::Encoder, Vec<Detail>)> {
    let path = Path::new(&request.path);
    if !ffmpeg::probe_self()
        .await
        .map_err(|e| AppError::new(ErrorCode::FfmpegBroken).with_cause(e))?
        .has_libvmaf
    {
        return Err(AppError::new(ErrorCode::VmafUnavailable));
    }

    let source = super::api::source_probe(&request.path).await?;
    let source_key = measurements::key_for(path)
        .map_err(|e| AppError::new(ErrorCode::InvalidInput).with_cause(e))?;

    // Where the light, middling and heavy chunks fall. The packets are read rather than
    // guessed at: positions are a guess about where a film is hard, weight is a reading.
    let seconds = measure::seconds_of(path)
        .await
        .map_err(|e| AppError::new(ErrorCode::FfmpegBroken).with_cause(e))?;
    let chunk_starts = reference_chunks(&seconds, CHUNK_S);

    let (encoder, notices) = pick_encoder(request.prefer_hardware).await?;
    let probed = probe_complexity::probe(path, source.duration_s, &encoder).await;
    let anchor_mbps = probed
        .measured_bps
        .map(|bps| (bps / 1_000_000).max(1))
        .unwrap_or(crate::domain::ladder::FALLBACK_MBPS);

    let mut notices = notices;
    notices.extend(probed.notice.clone());

    Ok((
        Run {
            source_key,
            codec: request.codec.clone(),
            source_path: request.path.clone(),
            width: source.width,
            height: source.height,
            fps: source.fps,
            source_bitrate_bps: source.bitrate_bps,
            heavier_codec: source.video_codec.eq_ignore_ascii_case("hevc"),
            native_height: request.native_height,
            anchor_mbps,
            chunk_starts,
            chunk_s: CHUNK_S as u64,
            borrowed_from: None,
        },
        encoder,
        notices,
    ))
}

fn facts_of(run: &Run) -> SourceFacts {
    SourceFacts {
        width: run.width,
        height: run.height,
        fps: run.fps,
        bitrate_bps: run.source_bitrate_bps,
        heavier_codec: run.heavier_codec,
        native_height: run.native_height,
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

fn to_error(e: quality_measure::MeasureError) -> AppError {
    use quality_measure::MeasureError as E;
    match e {
        E::Cancelled => AppError::new(ErrorCode::TaskCancelled),
        E::Unavailable => AppError::new(ErrorCode::VmafUnavailable),
        E::NothingMeasured => AppError::new(ErrorCode::LadderNotMeasured),
        other => AppError::new(ErrorCode::Internal).with_cause(other),
    }
}

pub mod ipc {
    use super::*;
    use tauri::State;

    #[tauri::command]
    pub async fn quality_measure_preview(
        state: State<'_, super::super::AppState>,
        request: MeasureRequest,
    ) -> Result<MeasurePreview> {
        api::quality_measure_preview(&state, &request).await
    }

    #[tauri::command]
    pub async fn quality_measure_start(
        state: State<'_, super::super::AppState>,
        request: MeasureRequest,
    ) -> Result<String> {
        api::quality_measure_start(&state, request).await
    }

    #[tauri::command]
    pub async fn quality_measure_result(
        state: State<'_, super::super::AppState>,
        source_key: String,
        codec: String,
    ) -> Result<MeasurementView> {
        api::quality_measure_result(&state, &source_key, &codec).await
    }

    #[tauri::command]
    pub async fn quality_measurements(
        state: State<'_, super::super::AppState>,
    ) -> Result<Vec<Run>> {
        api::quality_measurements(&state).await
    }

    #[tauri::command]
    pub async fn quality_measure_reuse(
        state: State<'_, super::super::AppState>,
        from_key: String,
        request: MeasureRequest,
    ) -> Result<MeasurementView> {
        api::quality_measure_reuse(&state, &from_key, request).await
    }

    #[tauri::command]
    pub async fn quality_measure_forget(
        state: State<'_, super::super::AppState>,
        source_key: String,
        codec: String,
    ) -> Result<()> {
        api::quality_measure_forget(&state, &source_key, &codec).await
    }
}
