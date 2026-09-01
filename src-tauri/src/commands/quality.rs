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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Build the set as soon as the measurement is done, without asking again (T438).
    ///
    /// **The decision is taken in the core, between choosing the rungs and sending them.**
    /// Not on a screen: by then the window may be closed or in the tray, and a decision taken
    /// by a closed window is taken by nobody. That is the whole of what a batch is — put a
    /// season in, come back to a season on the server.
    #[serde(default)]
    pub then_build: Option<ThenBuild>,
    /// Which batch this measurement belongs to, and what to call the film (T445).
    ///
    /// Carried on to the build the chain starts, so that "stop the whole batch" reaches both
    /// halves of every film. A batch whose builds were outside it would go on encoding for
    /// hours after somebody pressed stop.
    #[serde(default)]
    pub batch: Option<crate::tasks::store::Batch>,
}

/// What the build after a measurement needs that the measurement does not already know.
///
/// The rungs are deliberately absent: they are what the measurement is *for*, and carrying a
/// set of them in would mean building something other than what was measured.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ThenBuild {
    pub server_id: String,
    /// The medium's own directory on the server.
    pub slug: String,
    #[serde(default)]
    pub audio_track: usize,
}

fn h264() -> String {
    String::from("h264")
}

fn yes() -> bool {
    true
}

/// What the estimate of how long a measurement will take is standing on.
///
/// **Three states, because there are three situations and they mean different things to the
/// person deciding.** Having no timings of your own is ordinary and the cost model is a fair
/// substitute. Not being able to read the timings is a fault, and telling somebody their
/// estimate comes from the model when it might have come from their own hundred points is a
/// wrong answer wearing the clothes of a right one.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum MachineSpeed {
    /// Corrected by this machine's own past runs.
    Known {
        /// Hundredths: 100 is "as the model says", 300 is three times slower.
        factor_x100: u32,
        /// How many timed points it rests on.
        points: usize,
        /// Tenths of a second: what a point has actually been taking here.
        seconds_per_point_x10: u32,
    },
    /// Nothing has been timed on this machine yet, so the cost model stands as it is.
    NothingTimedYet,
    /// The store could not be asked — which is not the same as its having nothing in it.
    NotAsked,
}

impl MachineSpeed {
    /// What to multiply the model's estimate by. One, wherever nothing is known.
    fn factor(self) -> f64 {
        match self {
            Self::Known { factor_x100, .. } => f64::from(factor_x100) / 100.0,
            Self::NothingTimedYet | Self::NotAsked => 1.0,
        }
    }
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
    /// What the estimate above rests on (T423, T424).
    pub machine: MachineSpeed,
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
        // **Three answers, and they used to be two.** `.ok().flatten().unwrap_or((1.0, 0))`
        // turned a store that could not be read into a store with nothing in it, and the
        // screen then said the estimate came from the cost model — which it did, but not for
        // the reason given. Somebody with a hundred timed points would be told their own
        // timings were not being used, with no hint that anything had gone wrong (T424).
        let machine = match measurements::machine_factor(&state.db) {
            Ok(Some(speed)) => MachineSpeed::Known {
                factor_x100: (speed.factor * 100.0)
                    .round()
                    .clamp(1.0, f64::from(u32::MAX)) as u32,
                points: speed.points,
                seconds_per_point_x10: (speed.seconds_per_point * 10.0)
                    .round()
                    .clamp(0.0, f64::from(u32::MAX)) as u32,
            },
            Ok(None) => MachineSpeed::NothingTimedYet,
            Err(e) => {
                tracing::warn!(error = %e, "the timings of past points could not be read");
                MachineSpeed::NotAsked
            }
        };
        let factor = machine.factor();

        Ok(MeasurePreview {
            source_key: run.source_key.clone(),
            points,
            already_measured: already,
            about_seconds: (points.saturating_sub(already) as f64 * per_point * factor) as u64,
            machine,
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
        // What the chain needs, taken here rather than inside: `AppState` is a handful of
        // `Arc`s, so a clone is cheap and the task owns everything it will need after the
        // window has gone.
        let onward = request.then_build.clone();
        let chained = state.clone();
        let for_build = request.clone();

        let batch = request.batch.clone();
        let task_id = state
            .tasks
            .submit_in_batch(
                TaskKind::MeasureQuality,
                None,
                batch,
                move |ctx| async move {
                    let job = MeasureJob {
                        source: Path::new(&source),
                        run: &run,
                        encoder: &encoder,
                        db: &db,
                    };
                    let outcome = quality_measure::run(&job, &ctx).await.map_err(to_error)?;
                    ctx.report_important(1.0, DetailCode::StageDone);
                    // A partial measurement is an argument against building from it — the
                    // optimum may be outside what was measured — and it used to go no further
                    // than this log line (T416).
                    for notice in &outcome.notices {
                        ctx.add_notice(notice.clone());
                    }
                    tracing::info!(
                        measured = outcome.measured,
                        total = outcome.total,
                        rungs = outcome.selection.rungs.len(),
                        "quality measured"
                    );

                    // **And on to the build, if that is what was asked for** (T438). At the very
                    // end, and after the cancellation check: a person who stopped the measurement
                    // did not ask for hours of encoding to start in its place.
                    if let Some(onward) = onward {
                        if ctx.is_cancelled() {
                            // Stopped means stopped. Somebody who broke off a measurement did not
                            // ask for hours of encoding to begin in its place.
                            return Ok(());
                        }
                        then_build(&chained, &for_build, &onward, &ctx).await?;
                    }
                    Ok(())
                },
            )
            .await?;

        Ok(task_id)
    }

    /// Put the build on the queue, unless the ladder that came out is one to object to.
    ///
    /// **The gate is here and not on the button** (T439, and this narrows what that task
    /// said). The owner asked for "automatic, stopping on an objection", and an objection is
    /// a trade rather than an impossibility: a step of 2.75× between rungs hurts a viewer
    /// whose connection falls in the gap, and a person looking at that on screen may know
    /// their audience and accept it. Taking that away from them would be barring a decision
    /// they can see and have made. What they cannot see is a ladder chosen while the window
    /// was shut — so the chain stops, and only the chain.
    ///
    /// Stopping is a failure of this task, not a quiet skip. A batch of ten seasons where the
    /// fourth silently did nothing is a batch nobody can trust; the task must end red and say
    /// which objection stopped it.
    async fn then_build(
        state: &super::super::AppState,
        measured: &MeasureRequest,
        onward: &ThenBuild,
        ctx: &crate::tasks::engine::TaskContext,
    ) -> Result<()> {
        // The ladder as the core would offer it — asked for rather than assembled here, so
        // that what is built is what a person would have been shown.
        let plan = super::super::ladder::api::ladder_plan(
            state,
            &super::super::ladder::LadderRequest {
                path: measured.path.clone(),
                codec: measured.codec.clone(),
                native_height: measured.native_height,
                prefer_hardware: measured.prefer_hardware,
                declared_layout: None,
            },
        )
        .await?;

        // The verdict the plan already carries, rather than one worked out again here. What
        // stops the chain has to be the same judgement a person would have been shown, and two
        // computations of it are two chances to disagree.
        if !crate::domain::ladder::may_build_unasked(&plan.verdict.objections) {
            for objection in &plan.verdict.objections {
                ctx.add_notice(objection.detail());
            }
            return Err(AppError::new(ErrorCode::LadderObjection)
                .detail(DetailCode::ChainStoppedByObjection));
        }

        super::super::ladder::api::ladder_build(
            state,
            super::super::ladder::BuildRequest {
                server_id: onward.server_id.clone(),
                path: measured.path.clone(),
                slug: onward.slug.clone(),
                rungs: plan.plan.rungs.clone(),
                audio_track: onward.audio_track,
                prefer_hardware: measured.prefer_hardware,
                // The same batch as the measurement that started it. A build outside its
                // batch would go on encoding for hours after somebody pressed stop.
                batch: measured.batch.clone(),
            },
        )
        .await?;
        Ok(())
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
                // **How far apart the two films actually are** (R-46). Every field lending
                // compares was equal on four episodes of one season, and one of the four
                // scored fourteen VMAF below its neighbours — three times the measurement's
                // own noise. Container equality is not material equality, and that is
                // measured rather than suspected.
                //
                // Said rather than judged: nobody has measured what a difference in shape
                // means, so a threshold here would be invented, and an invented threshold in
                // a check is worse than no check because it looks like knowledge. The person
                // knows their own material; this gives them the numbers to use.
                let donor = measurements::run(&state.db, from_key, &borrowed.codec)
                    .ok()
                    .flatten();
                let gap =
                    crate::domain::chunks::shape_gap(donor.and_then(|d| d.shape), borrowed.shape);
                let mut view =
                    quality_measure_result(state, &borrowed.source_key, &borrowed.codec).await?;
                if let Some(gap) = gap {
                    view.notices.push(
                        Detail::new(DetailCode::NoticeMaterialApart)
                            .with("median", gap.median_x100)
                            .with("p90", gap.p90_x100)
                            .with("ratio", gap.ratio_x100),
                    );
                }
                Ok(view)
            }
            Err(LendRefusal::NothingToLend) => Err(AppError::new(ErrorCode::MeasurementNotFound)),
            Err(LendRefusal::DifferentMaterial(why)) => {
                Err(AppError::new(ErrorCode::MeasurementDifferentMaterial).detail(why.code()))
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
            // Kept rather than thrown away (T434). Every one of these was read a few lines
            // above and dropped, and lending then compared five fields and called it "the
            // same material".
            material: Some(measurements::Material {
                codec: source.video_codec.clone(),
                pix_fmt: source.pix_fmt.clone(),
                color_transfer: source.color_transfer.clone(),
                duration_s: source.duration_s,
                peak_bps: source.peak_bps,
            }),
            // A freshly prepared run knows nothing about any loan. `begin` keeps what is
            // already stored rather than taking these (T430) — writing `None` here does not
            // erase a mark, and must not be made to.
            borrowed_from: None,
            donor_anchor_mbps: None,
            // Worked out from the very packets the chunks were chosen by, and kept this time
            // (T435). Reading them again would be a second pass over the whole film for
            // numbers already in hand.
            shape: crate::domain::chunks::shape_of(&seconds),
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
