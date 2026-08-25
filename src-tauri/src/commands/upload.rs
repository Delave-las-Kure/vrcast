//! T088, T094, T095 — the upload commands.
//!
//! The contract: `contracts/ipc-commands.md`, the "Upload" section.
//!
//! What cannot be left in the transfer layer lives here: the rules for retrying and
//! reconnecting. Reconnecting needs the profile and the secret, and dragging those into the
//! transfer layer would spread the handling of credentials over two places instead of one.
//!
//! **Every check happens before the transfer starts** (FR-036, FR-037, FR-039). Learning
//! there is not enough room halfway through a thirty-gigabyte upload means losing an hour
//! and leaving an unfinished tail on the server.

use super::error::{AppError, DetailCode, ErrorCode, Result};
use super::AppState;
use crate::domain::progress_estimate::ProgressEstimate;
use crate::domain::remote_name::{self, NameVerdict};
use crate::domain::transfer::ResumeToken;
use crate::domain::wording::Detail;
use crate::server::free_space::{self, SpaceVerdict};
use crate::server::upload::{self, UploadError, UploadPlan};
use crate::server::{checksum, connect, disk, listing};
use crate::tasks::state::TaskKind;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

/// How many times reconnecting is attempted before the failure is admitted.
///
/// A break in a transfer that runs for hours is an ordinary thing rather than a fault;
/// giving up after the very first one would demand that a person sit by the button.
const MAX_ATTEMPTS: usize = 8;

/// The pause retrying starts from, and the one it grows to.
const FIRST_RETRY_DELAY: Duration = Duration::from_secs(2);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(60);

/// What the interface sends to start an upload.
#[derive(Debug, Clone, Deserialize)]
pub struct UploadRequest {
    pub server_id: String,
    /// The local path to the finished file.
    pub local_path: String,
    /// The name the file will be visible to viewers under.
    pub remote_name: String,
    /// Which medium to attribute it to. Empty means it lands in "not recognised".
    pub media_id: Option<String>,
    /// The speed limit in bytes per second. Empty means no limit.
    pub limit_bps: Option<u64>,
    /// Consent to the consequences warned about before the start.
    #[serde(default)]
    pub confirmed: bool,
}

/// What the application must say **before** the transfer starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preflight {
    /// There is not enough room: how much is needed and how much there is.
    pub not_enough_space: Option<SpaceShortage>,
    /// How many connections the server is serving right now: an upload will wash out of
    /// its memory what people are watching, and their playback will stall (FR-037).
    pub active_connections: usize,
    /// A file of that name is already being served (FR-039).
    pub name_exists: bool,
    /// With a CDN set, a replacement will be served from the old file's cache for a while.
    pub cdn_cached: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceShortage {
    pub needed: u64,
    pub free: u64,
    pub short_by: u64,
}

impl Preflight {
    /// Whether there is anything to warn about.
    pub fn has_warnings(&self) -> bool {
        self.not_enough_space.is_some() || self.active_connections > 0 || self.name_exists
    }

    /// Too little room is not a warning but a bar: confirming does not lift it.
    pub fn is_blocking(&self) -> bool {
        self.not_enough_space.is_some()
    }
}

pub mod api {
    use super::*;
    use crate::store::profiles;

    /// Start an upload.
    ///
    /// It returns the task identifier immediately; the transfer itself runs in the task
    /// engine (FR-080). Every check happens **before** the task is submitted: a refusal must
    /// come at once rather than an hour later.
    pub async fn upload_start(state: &AppState, request: UploadRequest) -> Result<String> {
        let profile = profiles::get(&state.db, &request.server_id)?
            .ok_or_else(|| crate::commands::servers::no_such_server(&request.server_id))?;

        let local_path = PathBuf::from(&request.local_path);
        let meta = tokio::fs::metadata(&local_path).await.map_err(|e| {
            AppError::new(ErrorCode::InvalidInput)
                .detail(DetailCode::UploadFileUnreadable)
                .with_cause(format!("{}: {e}", request.local_path))
        })?;
        if !meta.is_file() {
            return Err(AppError::new(ErrorCode::InvalidInput).detail(DetailCode::UploadNotAFile));
        }

        let clean_name = remote_name::sanitize(&request.remote_name);
        if clean_name.is_empty() {
            return Err(AppError::new(ErrorCode::InvalidInput).detail(DetailCode::UploadNameEmpty));
        }

        // The pre-transfer checks go over a live connection.
        let conn = connect(state.secrets.as_ref(), &profile).await?;
        let checks = preflight(&profile, &conn, &clean_name, meta.len()).await?;
        conn.close().await;

        if let Some(shortage) = checks.not_enough_space {
            return Err(space_error(shortage));
        }

        if checks.has_warnings() && !request.confirmed {
            return Err(warning_error(&checks, &clean_name));
        }

        // Two uploads under one name to one server would write into one staged file and
        // wipe out each other's work — and it would only come to light at the checksum
        // comparison. It is forbidden outright: the staged file's name deliberately depends
        // only on the final name (see `remote_name::staging_file`), and telling them apart
        // by task identifier is pointless — they would collide at the entry into serving
        // regardless.
        if let Some(busy) = running_upload_for(state, &profile.id, &clean_name)? {
            return Err(AppError::new(ErrorCode::NameExists)
                .with_detail(
                    Detail::new(DetailCode::UploadAlreadyRunning).with("name", clean_name.clone()),
                )
                .with_cause(busy));
        }

        // Everything is checked — the task goes in.
        let db = state.db.clone();
        let secrets = state.secrets.clone();
        let plan_request = request.clone();
        let name_for_task = clean_name.clone();
        let total = meta.len();

        let task_id = state
            .tasks
            .submit(TaskKind::Upload, Some(profile.id.clone()), move |ctx| {
                let request = plan_request;
                let name = name_for_task;
                async move { run_upload(db, secrets, ctx, request, name, total).await }
            })
            .await?;

        // The resume position is written **right after submitting** rather than when the
        // task gets round to working. The difference shows only on a restart: an upload that
        // stood in the queue and never once started holds nothing without this record —
        // neither the path to the source nor the name — and there is nothing to raise it
        // with at the next start. It would stay in the list forever, never moving.
        //
        // The task may manage to write its own position before we do: the contents come out
        // the same, because they are taken from the same request and the same file.
        if let Some(staging) = remote_name::staging_dir(&profile.video_dir) {
            let token = ResumeToken {
                remote_temp: remote_name::staging_file(&staging, &clean_name),
                remote_name: clean_name.clone(),
                local_path: Some(request.local_path.clone()),
                media_id: request.media_id.clone(),
                limit_bps: request.limit_bps,
                source_size: total,
                source_modified: modified_at(&meta),
            };
            let _ = crate::tasks::store::save_resume_token(&state.db, &task_id, &token.to_json());
        }

        Ok(task_id)
    }

    /// Carry on a paused or interrupted upload.
    pub fn upload_resume(state: &AppState, task_id: &str) -> Result<()> {
        Ok(state.tasks.resume(task_id)?)
    }

    /// Bring back to life the uploads left over from the previous run (FR-031).
    ///
    /// Called once when the application starts. Without it an upload, after the application
    /// is closed and started again, shows in the list as paused with nothing to carry it on:
    /// the working part lives only in memory and dies along with the application. To a
    /// person that would look like "the task is there but the button does nothing".
    ///
    /// The tasks come back **paused** and wait for a person's decision: carrying on a
    /// transfer that runs for hours unbidden at start-up will not do — the application may
    /// have been closed precisely to stop it.
    ///
    /// It returns how many uploads were raised.
    pub fn restore_uploads(state: &AppState) -> Result<usize> {
        let mut restored = 0;

        for task in state.tasks.list()? {
            if task.kind != TaskKind::Upload || task.state.is_final() {
                continue;
            }
            let Some(token) = task.resume_token.as_deref().and_then(ResumeToken::parse) else {
                // Without a resume position there is nothing to carry on: neither where it
                // was sending nor under what name is known. Such a task stays in the list,
                // and it can be dropped.
                tracing::debug!(task = %task.id, "an upload with no resume position was not raised");
                continue;
            };
            let Some(server_id) = task.server_id.clone() else {
                continue;
            };
            let Ok(Some(_)) = profiles::get(&state.db, &server_id) else {
                tracing::debug!(task = %task.id, "this upload's server was deleted; there is nowhere to raise it");
                continue;
            };

            // Only the resume position knows the path to the source. Records from earlier
            // versions do not hold one — such an upload has nothing to carry it on, but it
            // stays in the list, and it can be dropped.
            let Some(local_path) = token.local_path.clone() else {
                tracing::warn!(task = %task.id, "the resume position holds no path to the source");
                continue;
            };

            let request = UploadRequest {
                server_id,
                local_path,
                remote_name: token.remote_name.clone(),
                media_id: token.media_id.clone(),
                limit_bps: token.limit_bps,
                // The person agreed to the consequences when they started: asking a second
                // time about the same file means not remembering their answer.
                confirmed: true,
            };

            let db = state.db.clone();
            let secrets = state.secrets.clone();
            let name = token.remote_name.clone();
            let total = token.source_size;

            let result = state
                .tasks
                .resubmit_paused(&task.id, move |ctx| async move {
                    run_upload(db, secrets, ctx, request, name, total).await
                });

            match result {
                Ok(()) => restored += 1,
                Err(e) => {
                    tracing::warn!(task = %task.id, error = %e, "the upload could not be raised")
                }
            }
        }

        if restored > 0 {
            tracing::info!(
                restored,
                "uploads from the previous run are waiting to carry on"
            );
        }
        Ok(restored)
    }

    /// Whether an unfinished upload under this name to this server already exists.
    ///
    /// A note about the gap: the resume position is written inside the task, so two uploads
    /// begun at the very same instant will both pass this check. The gap is narrow and not
    /// the last line of defence: the checksum comparison will catch the divergence, and such
    /// a file never enters serving. Closing it with a lock held for the whole submission
    /// costs more than the case is worth.
    fn running_upload_for(state: &AppState, server_id: &str, name: &str) -> Result<Option<String>> {
        for task in state.tasks.list()? {
            if task.kind != TaskKind::Upload
                || task.state.is_final()
                || task.server_id.as_deref() != Some(server_id)
            {
                continue;
            }
            let same_target = task
                .resume_token
                .as_deref()
                .and_then(ResumeToken::parse)
                .is_some_and(|t| t.remote_name == name);
            if same_target {
                return Ok(Some(task.id));
            }
        }
        Ok(None)
    }

    /// The checks that must pass before the transfer starts.
    ///
    /// Deliberately not exposed as a command of its own: the interface learns the
    /// consequences the same way it does for a deletion — by calling without confirmation
    /// and getting back a refusal with the wording ready. Two different ways of asking "are
    /// you sure?" would drift apart in their phrasing.
    async fn preflight(
        profile: &crate::domain::server_profile::ServerProfile,
        conn: &crate::ssh::Connection,
        clean_name: &str,
        file_size: u64,
    ) -> Result<Preflight> {
        let usage = disk::usage(conn, &profile.video_dir).await?;

        // How much already lies in the staged file: on a carry-on that room is taken, and
        // demanding it afresh would refuse to finish a file that had almost arrived.
        let staging = remote_name::staging_dir(&profile.video_dir).ok_or_else(|| {
            AppError::new(ErrorCode::InvalidInput).detail(DetailCode::VideoDirAtRoot)
        })?;
        let already =
            upload::uploaded_so_far(conn, &remote_name::staging_file(&staging, clean_name))
                .await
                .unwrap_or(0);

        let not_enough_space = match free_space::check(&usage, file_size, already) {
            SpaceVerdict::Fits => None,
            SpaceVerdict::NotEnough {
                needed,
                free,
                short_by,
            } => Some(SpaceShortage {
                needed,
                free,
                short_by,
            }),
        };

        let entries = listing::list(conn, &profile.video_dir).await?;
        let existing: Vec<String> = entries.into_iter().map(|e| e.name).collect();
        let verdict = remote_name::check_name(clean_name, &existing, profile.cdn_base.is_some());

        let (name_exists, cdn_cached) = match verdict {
            NameVerdict::Exists { cdn_cached } => (true, cdn_cached),
            NameVerdict::Reserved => {
                return Err(
                    AppError::new(ErrorCode::InvalidInput).detail(DetailCode::UploadNameReserved)
                )
            }
            _ => (false, false),
        };

        Ok(Preflight {
            not_enough_space,
            active_connections: crate::server::active_use::serving_connections(conn).await,
            name_exists,
            cdn_cached,
        })
    }

    /// The source's fingerprint: its size and its modification time.
    ///
    /// The size alone is not enough — a file may have been rebuilt to the same size, and
    /// then carrying on would glue two different versions together. The modification time is
    /// taken as it stands, unparsed: it is a mark for comparing, not a date for showing.
    async fn source_fingerprint(
        path: &str,
    ) -> std::result::Result<(u64, Option<String>), AppError> {
        let meta = tokio::fs::metadata(path).await.map_err(|e| {
            AppError::new(ErrorCode::InvalidInput)
                .with_detail(
                    Detail::new(DetailCode::UploadSourceUnreadable).with("path", path.to_owned()),
                )
                .with_cause(e)
        })?;
        Ok((meta.len(), modified_at(&meta)))
    }

    /// A file's modification time, as a mark for comparing.
    ///
    /// Not as a date for showing: there is no point parsing and printing it here, while two
    /// such marks can be compared as they stand. A missing time is a legitimate case: not
    /// every file system keeps one.
    pub(super) fn modified_at(meta: &std::fs::Metadata) -> Option<String> {
        meta.modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs().to_string())
    }

    /// The transfer itself: attempts with reconnection, the comparison, the entry into serving.
    async fn run_upload(
        db: std::sync::Arc<crate::store::db::Db>,
        secrets: std::sync::Arc<dyn crate::store::secrets::SecretStore>,
        ctx: crate::tasks::engine::TaskContext,
        request: UploadRequest,
        clean_name: String,
        total: u64,
    ) -> std::result::Result<(), AppError> {
        let profile = match crate::store::profiles::get(&db, &request.server_id) {
            Ok(Some(p)) => p,
            Ok(None) => {
                return Err(AppError::new(ErrorCode::InvalidInput)
                    .detail(DetailCode::ProfileNotFound)
                    .with_cause(&request.server_id))
            }
            Err(e) => return Err(e.into()),
        };

        let staging = remote_name::staging_dir(&profile.video_dir).ok_or_else(|| {
            AppError::new(ErrorCode::InvalidInput).detail(DetailCode::VideoDirAtRoot)
        })?;

        // Is this the file we started from?
        //
        // The check comes **before** connecting to the server: had the source been swapped,
        // carrying on would append the tail of one file to the beginning of another. The
        // checksum comparison would catch it too — but only after the whole transfer had
        // finished, that is, after an hour of wasted work.
        let (size_now, modified_now) = source_fingerprint(&request.local_path).await?;
        let previous = ctx
            .resume_token()
            .ok()
            .flatten()
            .as_deref()
            .and_then(ResumeToken::parse);
        let source_changed = match &previous {
            // Carrying on an earlier transfer — compared against what was written then.
            Some(prev) => !prev.matches_source(size_now, modified_now.as_deref()),
            // A first attempt — the source could change between the checks and the start.
            None => size_now != total,
        };
        if source_changed {
            return Err(
                AppError::new(ErrorCode::ChecksumMismatch).detail(DetailCode::UploadSourceChanged)
            );
        }

        let plan = UploadPlan {
            local_path: PathBuf::from(&request.local_path),
            remote_temp: remote_name::staging_file(&staging, &clean_name),
            remote_final: upload::final_path(&profile.video_dir, &clean_name),
            total_bytes: total,
            limit_bps: request.limit_bps,
        };

        // The resume position is written at once: should the application be killed before
        // the first window, the next start has to know where to look.
        if previous.is_none() {
            let token = ResumeToken {
                remote_temp: plan.remote_temp.clone(),
                remote_name: clean_name.clone(),
                local_path: Some(request.local_path.clone()),
                media_id: request.media_id.clone(),
                limit_bps: request.limit_bps,
                source_size: size_now,
                source_modified: modified_now,
            };
            let _ = ctx.save_resume_token(&token.to_json());
        }

        let mut estimate = ProgressEstimate::default();
        let mut delay = FIRST_RETRY_DELAY;

        for attempt in 1..=MAX_ATTEMPTS {
            let conn = match connect(secrets.as_ref(), &profile).await {
                Ok(c) => c,
                Err(e) => {
                    if attempt == MAX_ATTEMPTS {
                        return Err(e.into());
                    }
                    wait_before_retry(&ctx, &mut delay).await?;
                    continue;
                }
            };

            if attempt == 1 {
                if let Err(e) = upload::ensure_staging(&conn, &staging, &profile.video_dir).await {
                    conn.close().await;
                    return Err(AppError::new(ErrorCode::Internal).with_cause(e));
                }
            }

            match upload::transfer_once(&conn, &ctx, &plan, &mut estimate).await {
                Ok(sent) => {
                    let outcome = finish(&conn, &ctx, &plan, sent, &clean_name, &request).await;
                    conn.close().await;
                    return outcome;
                }
                Err(UploadError::Cancelled) => {
                    upload::cleanup(&conn, &plan.remote_temp).await;
                    conn.close().await;
                    return Ok(());
                }
                Err(e) if e.is_retriable() && attempt < MAX_ATTEMPTS => {
                    tracing::warn!(attempt, error = %e, "the transfer broke off; trying again");
                    // The time estimate is reset: what was gathered before the break no
                    // longer describes what is happening.
                    estimate.reset();
                    conn.close().await;
                    wait_before_retry(&ctx, &mut delay).await?;
                }
                Err(e) => {
                    conn.close().await;
                    return Err(AppError::new(ErrorCode::Internal).with_cause(e));
                }
            }
        }

        Err(AppError::new(ErrorCode::SshUnreachable).with_detail(
            Detail::new(DetailCode::UploadTooManyBreaks).with("attempts", MAX_ATTEMPTS),
        ))
    }

    /// Compare the checksums and enter the file into serving.
    async fn finish(
        conn: &crate::ssh::Connection,
        ctx: &crate::tasks::engine::TaskContext,
        plan: &UploadPlan,
        sent: u64,
        clean_name: &str,
        request: &UploadRequest,
    ) -> std::result::Result<(), AppError> {
        if sent != plan.total_bytes {
            return Err(AppError::new(ErrorCode::Internal).with_detail(
                Detail::new(DetailCode::UploadShort)
                    .with("sent", sent)
                    .with("total", plan.total_bytes),
            ));
        }

        ctx.report_important(0.98, DetailCode::StageChecksum);

        let ours = checksum::local(&plan.local_path)
            .await
            .map_err(|e| AppError::new(ErrorCode::Internal).with_cause(e))?;
        let theirs = checksum::remote(conn, &plan.remote_temp)
            .await
            .map_err(|e| AppError::new(ErrorCode::Internal).with_cause(e))?;

        if !checksum::matches(&ours, &theirs) {
            // The file does not enter serving, and we clean up after ourselves: a spoilt
            // transfer must leave no trace (FR-032, FR-038).
            upload::cleanup(conn, &plan.remote_temp).await;
            return Err(AppError::new(ErrorCode::ChecksumMismatch)
                .detail(DetailCode::UploadChecksumMismatch));
        }

        upload::publish(conn, plan)
            .await
            .map_err(|e| AppError::new(ErrorCode::Internal).with_cause(e))?;

        ctx.report_important(1.0, DetailCode::StageDone);
        let _ = request;
        let _ = clean_name;
        Ok(())
    }

    /// Wait before retrying, without missing a cancellation.
    async fn wait_before_retry(
        ctx: &crate::tasks::engine::TaskContext,
        delay: &mut Duration,
    ) -> std::result::Result<(), AppError> {
        let cancel = ctx.cancel_token();
        tokio::select! {
            _ = tokio::time::sleep(*delay) => {}
            _ = cancel.cancelled() => return Err(AppError::new(ErrorCode::TaskCancelled)),
        }
        *delay = (*delay * 2).min(MAX_RETRY_DELAY);
        Ok(())
    }
}

/// The thin wrappers the shell exposes to the interface.
pub mod ipc {
    use super::*;
    use tauri::State;

    #[tauri::command]
    pub async fn upload_start(
        state: State<'_, AppState>,
        request: UploadRequest,
    ) -> Result<String> {
        api::upload_start(&state, request).await
    }

    #[tauri::command]
    pub fn upload_resume(state: State<'_, AppState>, task_id: String) -> Result<()> {
        api::upload_resume(&state, &task_id)
    }
}

/// A refusal over too little room on the server.
///
/// Confirming does not lift it: room does not appear out of consent. That is exactly why it
/// is kept apart from [`warning_error`] — confusing a bar with a warning would offer a
/// person an "upload anyway" button, after which the transfer runs into the end of the disk
/// halfway through.
pub fn space_error(shortage: SpaceShortage) -> AppError {
    AppError::new(ErrorCode::RemoteDiskFull)
        .with_detail(
            Detail::new(DetailCode::NotEnoughSpace)
                .with("short_by", shortage.short_by)
                .with("needed", shortage.needed)
                .with("free", shortage.free),
        )
        .with_cause(format!("short_by={}", shortage.short_by))
}

/// A refusal that names the consequences and is lifted by confirming.
pub fn warning_error(checks: &Preflight, name: &str) -> AppError {
    let mut details: Vec<Detail> = Vec::new();

    if checks.name_exists {
        details.push(Detail::new(DetailCode::NameWillBeReplaced).with("name", name.to_string()));
        if checks.cdn_cached {
            details.push(Detail::new(DetailCode::CdnKeepsOldCopy));
        }
    }
    if checks.active_connections > 0 {
        details.push(
            Detail::new(DetailCode::ViewersActiveUpload)
                .with("connections", checks.active_connections),
        );
    }

    let code = if checks.name_exists {
        ErrorCode::NameExists
    } else {
        ErrorCode::ViewersActive
    };
    AppError::new(code).with_details(details)
}
