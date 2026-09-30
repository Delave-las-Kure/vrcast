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
    /// The person has agreed to replace a finished result already at `out_path` (T662).
    ///
    /// Without it a start over an existing file is refused with `CONFIRMATION_REQUIRED` /
    /// `CONVERT_OUT_EXISTS`: a finished preparation may be hours of work, and it used to be
    /// overwritten without a word — and deleted outright if the new attempt was cancelled.
    #[serde(default)]
    pub confirmed: bool,
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
        // Compared as the same file rather than as the same text (T662): `F:\a.mp4` and
        // `f:/A.mp4` are one file on Windows, and a literal comparison let the second
        // through to destroy the source.
        if same_file(&request.out_path, &request.path) {
            // Writing over the source destroys the only copy of the original the
            // moment the encoder opens the file for writing, and there is no way
            // back from that.
            return Err(AppError::new(ErrorCode::InvalidInput)
                .detail(DetailCode::ConvertOutOverwritesSource));
        }

        // **The output path is held by one preparation at a time** (T662). Two of them
        // writing one result would each replace the other's, and the core — not only the
        // button on one screen — is what can see both. Held by the task's work until it ends,
        // however it ends; dropped on every early return below.
        let claim = match state.tasks.try_claim(&output_key(&request.out_path)) {
            Ok(claim) => claim,
            Err(crate::tasks::engine::ClaimRefused::Forgetting) => {
                return Err(AppError::new(ErrorCode::ForgetInProgress));
            }
            Err(crate::tasks::engine::ClaimRefused::Taken) => {
                return Err(AppError::new(ErrorCode::InvalidInput).with_detail(
                    Detail::new(DetailCode::ConvertOutBusy)
                        .with("out_path", request.out_path.clone()),
                ));
            }
        };

        // **A finished result is not replaced without a yes** (T662, decision of 2026-09-30).
        if std::path::Path::new(&request.out_path).exists() && !request.confirmed {
            return Err(AppError::new(ErrorCode::ConfirmationRequired).with_detail(
                Detail::new(DetailCode::ConvertOutExists)
                    .with("out_path", request.out_path.clone()),
            ));
        }

        let source = preview.source.clone();
        let plan = preview.plan.clone();
        let encoder = preview.encoder.clone();
        let out_path = request.out_path.clone();

        let task_id = state
            .tasks
            .submit(TaskKind::Convert, None, move |ctx| async move {
                let _claim = claim;
                prepare(&ctx, &source, &plan, &encoder, &out_path).await
            })
            .await?;

        Ok(task_id)
    }

    /// The work of one preparation: encode into a file of its own, check it, and only then
    /// put it where the person asked for it (T662).
    ///
    /// **Why a file of its own.** The encoder used to write straight into `out_path`, with
    /// `-y`: a second preparation into the same place overwrote a finished result from its
    /// first second, and cancelling it — or its failing — then deleted the path, so the old
    /// result was gone as well. Now the attempt owns `attempt` and nothing else: a cancel or
    /// a failed encode removes that and only that, and the finished file is replaced by one
    /// rename once the new one has been decoded end to end.
    async fn prepare(
        ctx: &crate::tasks::engine::TaskContext,
        source: &SourceFile,
        plan: &ConvertPlan,
        encoder: &encoders::Encoder,
        out_path: &str,
    ) -> Result<()> {
        let attempt = attempt_path(out_path);
        let attempt_str = attempt.to_string_lossy().into_owned();
        let job = convert::ConvertJob {
            source,
            plan,
            encoder,
            out_path: &attempt_str,
        };

        let said = convert::run(&job, ctx).await.map_err(|e| match e {
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
        //
        // A stage of its own, counted from zero by the decode itself (T663): it used to sit
        // at 98 % for as long as the decode took, which on a long film is minutes of a bar
        // that does not move and a task that did not answer cancel or pause.
        ctx.report_important(0.0, DetailCode::StageValidating);
        let verdict = match validate::validate_in_task(&attempt, ctx, source.duration_s).await {
            Ok(verdict) => verdict,
            Err(validate::ValidateError::Cancelled) => {
                let _ = std::fs::remove_file(&attempt);
                return Err(AppError::new(ErrorCode::TaskCancelled));
            }
            Err(validate::ValidateError::Stalled(seconds)) => {
                // The encode is whole as far as anybody knows; only its check hung. Kept
                // under its own name, as a file that failed its check is, and never
                // under `out_path`.
                return Err(
                    AppError::new(ErrorCode::DecodeValidationFailed).with_detail(
                        Detail::new(DetailCode::ValidateStalled)
                            .with("seconds", seconds)
                            .with("out_path", attempt_str.clone()),
                    ),
                );
            }
            Err(e) => {
                let _ = std::fs::remove_file(&attempt);
                return Err(AppError::new(ErrorCode::FfmpegBroken).with_cause(e));
            }
        };

        if !verdict.ok {
            // The attempt is left on disk on purpose, under its own name and never under
            // `out_path`: it may be hours of work, and the person may want to look at it.
            // What matters is that nothing will take it for a result — the finished file
            // that was there before, if any, is untouched.
            return Err(
                AppError::new(ErrorCode::DecodeValidationFailed).with_detail(
                    Detail::new(DetailCode::ConvertValidationFailed)
                        .with("out_path", attempt_str.clone())
                        .with("problems", verdict.problems.join(" ")),
                ),
            );
        }

        // A cancel that arrived during the decode is still a cancel: the old result stays.
        if ctx.is_cancelled() {
            let _ = std::fs::remove_file(&attempt);
            return Err(AppError::new(ErrorCode::TaskCancelled));
        }

        // One rename, in one directory: a reader sees the old file or the new one, never a
        // half of either.
        if let Err(e) = std::fs::rename(&attempt, out_path) {
            // Checked and whole — kept, and where it is is said. Removing hours of good work
            // because the old file was held open by a player would be the loss this exists
            // to prevent.
            return Err(AppError::new(ErrorCode::StorageFailed)
                .with_detail(
                    Detail::new(DetailCode::ConvertReplaceFailed)
                        .with("out_path", out_path.to_owned())
                        .with("kept_at", attempt_str.clone()),
                )
                .with_cause(e));
        }

        ctx.report_important(1.0, DetailCode::StageDone);
        Ok(())
    }

    /// Where one attempt writes: beside the result, so the final rename stays in one
    /// directory (one volume, one atomic step), and named so it is never taken for one.
    pub fn attempt_path(out_path: &str) -> std::path::PathBuf {
        let out = std::path::Path::new(out_path);
        let name = out
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| String::from("prepared"));
        let short = uuid::Uuid::new_v4().simple().to_string();
        let own = format!("{name}.{}.vrcast-part", &short[..8]);
        match out.parent() {
            Some(dir) if !dir.as_os_str().is_empty() => dir.join(own),
            _ => std::path::PathBuf::from(own),
        }
    }

    /// The key a preparation holds its output path under (T662).
    ///
    /// The same file spelled two ways is one key: separators unified, relative paths made
    /// absolute, and — on Windows, where the file system does not care — case folded.
    pub fn output_key(out_path: &str) -> String {
        format!("convert-out:{}", normalized(out_path))
    }

    fn normalized(path: &str) -> String {
        let absolute = std::path::absolute(path.trim())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| path.trim().to_owned());
        let unified = absolute.replace('\\', "/");
        if cfg!(windows) {
            unified.to_lowercase()
        } else {
            unified
        }
    }

    /// Whether two paths name the same file, as far as can be told without the file
    /// system's help (the output usually does not exist yet).
    pub fn same_file(a: &str, b: &str) -> bool {
        normalized(a) == normalized(b)
    }

    /// Check that an already prepared file plays (FR-027).
    ///
    /// Outside any task, so nothing can cancel it — but it is bounded all the same: a decoder
    /// that stops moving is stopped after `validate::STALL_LIMIT` (T663).
    pub async fn convert_validate(path: &str) -> Result<validate::Validation> {
        validate::validate(std::path::Path::new(path))
            .await
            .map_err(|e| match e {
                validate::ValidateError::Stalled(seconds) => {
                    AppError::new(ErrorCode::DecodeValidationFailed).with_detail(
                        Detail::new(DetailCode::ValidateStalled)
                            .with("seconds", seconds)
                            .with("out_path", path.to_owned()),
                    )
                }
                other => AppError::new(ErrorCode::FfmpegBroken)
                    .detail(DetailCode::ConvertValidateNoFfmpeg)
                    .with_cause(other.to_string()),
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
