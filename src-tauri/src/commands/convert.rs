//! T117, T120 — commands for preparing a file (FR-022, FR-026, FR-027).
//!
//! Contract: `contracts/ipc-commands.md`, the file preparation section.
//!
//! Preparing takes minutes to hours, so it is a task rather than a call that
//! returns when it is done (FR-080). Everything that can refuse quickly refuses
//! before the task is created: an unreadable source, a request that contradicts
//! itself, a missing encoder. Learning any of those an hour in would waste the
//! hour and leave a half-written file behind.

use super::error::{AppError, DetailCode, ErrorCode, Result};
use crate::domain::convert_plan::{self, ConvertPlan};
use crate::domain::source::SourceFile;
use crate::domain::wording::Detail;
use crate::media::{convert, encoders, validate};
use crate::tasks::state::TaskKind;
use serde::{Deserialize, Serialize};

/// What the interface sends to start preparing a file.
#[derive(Debug, Clone, Deserialize)]
pub struct ConvertStart {
    pub path: String,
    /// Which audio track to keep, numbered among audio tracks from zero.
    pub audio_track: usize,
    /// Target video bitrate in kilobits. Empty means "do not aim for one".
    pub target_kbps: Option<u32>,
    /// Target frame height. Empty means "leave it alone".
    pub height: Option<u32>,
    pub out_path: String,
    /// False when the person asked for the processor on purpose.
    #[serde(default = "yes")]
    pub prefer_hardware: bool,
}

fn yes() -> bool {
    true
}

/// What preparing this file will actually involve.
///
/// Shown before anything starts. Re-encoding costs hours where copying costs
/// minutes, and the difference is worth knowing before agreeing to it.
#[derive(Debug, Clone, Serialize)]
pub struct ConvertPreview {
    pub plan: ConvertPlan,
    pub source: SourceFile,
    /// Which encoder will be used, and what to say about that choice.
    pub encoder: encoders::Encoder,
    pub encoder_notice: Option<Detail>,
    /// True when nothing is re-encoded: minutes rather than hours, no loss.
    pub lossless: bool,
}

pub mod api {
    use super::*;

    /// Work out what preparing this file would involve, without doing it.
    pub async fn convert_preview(request: &ConvertStart) -> Result<ConvertPreview> {
        let source = super::super::api::source_probe(&request.path).await?;
        let plan = build_plan(&source, request)?;
        let (encoder, notice) = pick_encoder(&plan, request.prefer_hardware).await?;

        Ok(ConvertPreview {
            lossless: plan.lossless(),
            plan,
            source,
            encoder,
            encoder_notice: notice,
        })
    }

    /// Start preparing the file. Returns a task number immediately (FR-080).
    pub async fn convert_start(
        state: &super::super::AppState,
        request: ConvertStart,
    ) -> Result<String> {
        // Everything that can refuse quickly refuses here, before a task exists.
        let preview = convert_preview(&request).await?;

        if request.out_path.trim().is_empty() {
            return Err(AppError::new(ErrorCode::InvalidInput).detail(DetailCode::ConvertNoOutPath));
        }
        if request.out_path == request.path {
            // Writing over the source destroys the only copy of the original the
            // moment the encoder opens the file for writing, and there is no way
            // back from that.
            return Err(AppError::new(ErrorCode::InvalidInput)
                .detail(DetailCode::ConvertOutOverwritesSource));
        }

        let source = preview.source.clone();
        let plan = preview.plan.clone();
        let encoder = preview.encoder.clone();
        let out_path = request.out_path.clone();

        let task_id = state
            .tasks
            .submit(TaskKind::Convert, None, move |ctx| async move {
                let job = convert::ConvertJob {
                    source: &source,
                    plan: &plan,
                    encoder: &encoder,
                    out_path: &out_path,
                };

                let said = convert::run(&job, &ctx).await.map_err(|e| match e {
                    convert::ConvertError::Cancelled => AppError::new(ErrorCode::TaskCancelled),
                    other => AppError::new(ErrorCode::Internal).with_cause(other),
                })?;

                // A task now has somewhere to put a notice (T415), so this no longer has to
                // borrow the stage line to say that the graphics card refused. The stage
                // said the code and nothing else; a notice carries the numbers with it.
                for notice in said {
                    ctx.add_notice(notice);
                }

                // Validation is not optional (FR-027). A broken encode opens fine,
                // reports the right duration, and falls apart where someone is
                // watching — the only way to know is to decode the whole thing.
                ctx.report_important(0.98, DetailCode::StageValidating);
                let verdict = validate::validate(std::path::Path::new(&out_path))
                    .await
                    .map_err(|e| AppError::new(ErrorCode::FfmpegBroken).with_cause(e))?;

                if !verdict.ok {
                    // The file is left on disk on purpose: it may be hours of work,
                    // and the person may want to look at it. What matters is that
                    // the application will not offer it for upload.
                    return Err(
                        AppError::new(ErrorCode::DecodeValidationFailed).with_detail(
                            Detail::new(DetailCode::ConvertValidationFailed)
                                .with("out_path", out_path.clone())
                                .with("problems", verdict.problems.join(" ")),
                        ),
                    );
                }

                ctx.report_important(1.0, DetailCode::StageDone);
                Ok(())
            })
            .await?;

        Ok(task_id)
    }

    /// Check that an already prepared file plays (FR-027).
    pub async fn convert_validate(path: &str) -> Result<validate::Validation> {
        validate::validate(std::path::Path::new(path))
            .await
            .map_err(|e| {
                AppError::new(ErrorCode::FfmpegBroken)
                    .detail(DetailCode::ConvertValidateNoFfmpeg)
                    .with_cause(e.to_string())
            })
    }

    /// Turn the request into a plan, or into every objection at once.
    fn build_plan(source: &SourceFile, request: &ConvertStart) -> Result<ConvertPlan> {
        let ask = convert_plan::ConvertRequest {
            audio_track: request.audio_track,
            target_kbps: request.target_kbps,
            height: request.height,
        };

        convert_plan::plan(source, &ask).map_err(|problems| {
            let code = if problems.contains(&convert_plan::PlanProblem::NoAudioTracks) {
                ErrorCode::NoAudioTracks
            } else {
                ErrorCode::InvalidInput
            };
            // All objections at once: there is often more than one, and finding
            // them one round at a time is work that need not exist.
            AppError::new(code).with_details(problems.iter().map(|p| p.detail()))
        })
    }

    /// Choose an encoder, and carry along whatever should be said about it.
    async fn pick_encoder(
        plan: &ConvertPlan,
        prefer_hardware: bool,
    ) -> Result<(encoders::Encoder, Option<Detail>)> {
        // Copying needs no encoder at all, and demanding one would refuse work
        // that requires nothing of the kind.
        if plan.lossless() {
            return Ok((encoders::Encoder::Software, None));
        }

        let info = super::super::api::ffmpeg_probe_self().await?;
        let choice =
            encoders::choose(&info.hardware, info.has_x264, prefer_hardware).map_err(|e| {
                AppError::new(ErrorCode::NoHwEncoder)
                    .detail(DetailCode::ConvertNoEncoder)
                    .with_cause(e.to_string())
            })?;
        Ok((choice.encoder, choice.notice))
    }
}

pub mod ipc {
    use super::*;
    use tauri::State;

    #[tauri::command]
    pub async fn convert_preview(request: ConvertStart) -> Result<ConvertPreview> {
        api::convert_preview(&request).await
    }

    #[tauri::command]
    pub async fn convert_start(
        state: State<'_, super::super::AppState>,
        request: ConvertStart,
    ) -> Result<String> {
        api::convert_start(&state, request).await
    }

    #[tauri::command]
    pub async fn convert_validate(path: String) -> Result<validate::Validation> {
        api::convert_validate(&path).await
    }
}
