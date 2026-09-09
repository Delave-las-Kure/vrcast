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
use crate::server::gate::{self, Intent};
use crate::server::upload::{self, UploadError, UploadPlan};
use crate::server::{checksum, disk, listing};
use crate::tasks::state::TaskKind;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

/// How many times reconnecting is attempted before the failure is admitted.
///
/// A break in a transfer that runs for hours is an ordinary thing rather than a fault;
/// giving up after the very first one would demand that a person sit by the button.
const MAX_ATTEMPTS: usize = 8;

/// How many times reconnecting is attempted during the checksum-or-publish phase (T570).
///
/// Kept apart from [`MAX_ATTEMPTS`] rather than shared with it: that constant is sized for
/// a byte transfer that can run for hours, where the number of windows a break might land in
/// is large. This phase is short by comparison — a checksum pass and one rename — and every
/// attempt here also does the extra round trip in [`api::locate`] to see what the server
/// actually has, so there is less to retry towards. The same number as `MAX_ATTEMPTS` all
/// the same: nothing about this phase's own failures on a real connection suggested it needs
/// either more patience or less, and two different ceilings would want two different
/// justifications the moment somebody asked why they differ.
const FINISH_MAX_ATTEMPTS: usize = MAX_ATTEMPTS;

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
        // Before anything is sent: how much room there is, and who is watching.
        let conn = gate::open(state.secrets.as_ref(), &profile, Intent::Read)
            .await?
            .conn;
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
            let conn = match gate::open(secrets.as_ref(), &profile, Intent::Change)
                .await
                .map(|opened| opened.conn)
            {
                Ok(c) => c,
                Err(e) => {
                    if attempt == MAX_ATTEMPTS {
                        return Err(e.into());
                    }
                    // The same cleanup as below, and found by the compiler when the wait
                    // stopped being a `Result` (T521): this caller dropped the cancellation
                    // too. On a second attempt or later there is already a part-file on the
                    // server, and the connection this arm is retrying is the one that failed
                    // — so a fresh one is opened to tidy up, exactly as after a break.
                    if wait_before_retry(&ctx, &mut delay).await == Waited::Cancelled {
                        sweep_after_cancelling(secrets.as_ref(), &profile, &plan).await;
                        return Ok(());
                    }
                    continue;
                }
            };

            // ⚠ **On every fresh connection, with no memory of whether it has been done**
            // (T521). It used to be `if attempt == 1`, which looks equivalent and is not: when
            // the first attempt fails at the gate it never reaches here, and every attempt
            // after has `attempt != 1` — so the directory was never made and the two
            // `stat -c %d` were never compared. That comparison is the whole reason the
            // staging sits beside the serving directory: on one file system entering serving
            // is a rename, across two it is a copy of the entire file.
            //
            // The fix is to keep no state rather than to keep it correctly. `mkdir -p` and two
            // `stat` are one round trip on a connection that has just been established after a
            // break — against an hour of transfer, and against a whole class of bug about
            // remembering whether something was done.
            if let Err(e) = upload::ensure_staging(&conn, &staging, &profile.video_dir).await {
                conn.close().await;
                return Err(AppError::new(ErrorCode::Internal).with_cause(e));
            }

            match upload::transfer_once(&conn, &ctx, &plan, &mut estimate).await {
                Ok(sent) => {
                    let outcome = finish(
                        &ctx,
                        Finishing {
                            conn: &conn,
                            secrets: secrets.as_ref(),
                            profile: &profile,
                            plan: &plan,
                            sent,
                            clean_name: &clean_name,
                            request: &request,
                            video_dir: &profile.video_dir,
                        },
                    )
                    .await;
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
                    // ⚠ **A cancellation here used to leave the part-file behind for good**
                    // (T521, FR-038). This is exactly when somebody presses stop: the transfer
                    // has visibly stalled and the pauses are doubling. The `?` carried the
                    // cancellation out past both places that clean up, and the connection had
                    // been closed a line above — so there was nothing left to clean up with,
                    // and nothing anywhere sweeps abandoned staging files later.
                    if wait_before_retry(&ctx, &mut delay).await == Waited::Cancelled {
                        sweep_after_cancelling(secrets.as_ref(), &profile, &plan).await;
                        return Ok(());
                    }
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
    ///
    /// Bundled into one argument rather than passed loose (T570 added two of these — the
    /// secrets and the profile, needed only for reconnecting inside this phase — and eight
    /// loose parameters is a shape `clippy::too_many_arguments` rightly complains about).
    struct Finishing<'a> {
        conn: &'a crate::ssh::Connection,
        secrets: &'a dyn crate::store::secrets::SecretStore,
        profile: &'a crate::domain::server_profile::ServerProfile,
        plan: &'a UploadPlan,
        sent: u64,
        clean_name: &'a str,
        request: &'a UploadRequest,
        video_dir: &'a str,
    }

    async fn finish(
        ctx: &crate::tasks::engine::TaskContext,
        job: Finishing<'_>,
    ) -> std::result::Result<(), AppError> {
        let Finishing {
            conn,
            secrets,
            profile,
            plan,
            sent,
            clean_name,
            request,
            video_dir,
        } = job;
        if sent != plan.total_bytes {
            return Err(AppError::new(ErrorCode::Internal).with_detail(
                Detail::new(DetailCode::UploadShort)
                    .with("sent", sent)
                    .with("total", plan.total_bytes),
            ));
        }

        // ⚠ **Asked twice on the way through, because there used to be nowhere it was asked
        // at all** (T503). The last phase is not instant: the checksum reads the whole file,
        // which on thirty gigabytes is minutes, and pressing stop through them did nothing —
        // the transfer went on, the file entered serving, and the engine then wrote the task
        // down as cancelled. A person saw "cancelled" about a film that was being served.
        if ctx.is_cancelled() {
            upload::cleanup(conn, &plan.remote_temp).await;
            return Ok(());
        }

        ctx.report_important(0.98, DetailCode::StageChecksum);

        let ours = checksum::local(&plan.local_path)
            .await
            .map_err(|e| AppError::new(ErrorCode::Internal).with_cause(e))?;

        // ⚠ **This phase used to have no reconnection of its own** (T570). It runs two
        // network calls — `checksum::remote`, which reads the whole file on the server and
        // can take minutes, and `publish`, the rename into serving — and a break in either
        // used to surface as `AppError::Internal` straight away: the retriable-ness that
        // `UploadError` already knows was thrown away, and the task failed even when the
        // file was sitting on the server intact. Falling back on `run_upload`'s own retry
        // loop will not do either: that loop exists to resume a byte transfer from
        // `uploaded_so_far`, and handing it a failure here would send the whole file again
        // — after every byte had already arrived. So the loop is here instead, and it reasons
        // about what the reconnected server actually shows rather than assuming the worst.
        let (working_conn, opened_here) =
            match checksum_and_publish(conn, ctx, secrets, profile, plan, &ours).await? {
                FinishOutcome::Published(c, o) => (c, o),
                // A cancellation was honoured somewhere inside the retry loop — cleanup
                // already ran there, on whichever connection was open at the time.
                FinishOutcome::Cancelled => return Ok(()),
            };

        // **The window that cannot be closed, said out loud instead of hidden.** Between the
        // check inside `checksum_and_publish` and the rename finishing there is a moment, and
        // a cancellation arriving in it is real: the engine will write the task down as
        // cancelled, truthfully as far as the person's press goes, while the file is serving.
        // The row then says "cancelled" and nothing else — so the note is what makes the two
        // agree. The alternative, un-publishing, would delete under a viewer who has already
        // started watching.
        if ctx.is_cancelled() {
            ctx.add_notice(
                Detail::new(DetailCode::NoticeCancelledAfterPublish).with("name", clean_name),
            );
        }

        // **The medium the person chose, written into the catalogue** (T505, FR-019).
        //
        // ⚠ **This used to be `let _ = request;`** — the upload screen offered a choice of
        // medium, `media_id` was documented as "which medium to file it under", and it went
        // nowhere at all. Every uploaded file landed in "not recognised" and had to be
        // assigned by hand, which is also half of what FR-019 asks for: the tie must survive
        // being read from another machine, and a tie that was never written survives nothing.
        //
        // **After the rename, not before.** The catalogue may only ever claim files that are
        // there: an entry written first and a rename that then failed would leave the library
        // pointing at nothing, and `exists_on_server` false is what a person reads as "the
        // file was deleted behind my back" (FR-018).
        //
        // **Failing to file it is not a failed upload.** The bytes are across and the film is
        // serving; refusing here would report a failure about work that succeeded and invite
        // a repeat that has nothing left to do. It is said instead, as a notice, and the file
        // shows up unrecognised — which is exactly where it used to land every time.
        if let Some(media_id) = request.media_id.as_deref() {
            if let Err(e) = file_it_under(&working_conn, video_dir, media_id, clean_name).await {
                tracing::warn!(error = %e, media_id, "the file was not filed under its medium");
                ctx.add_notice(
                    Detail::new(DetailCode::NoticeNotFiledUnderMedium).with("name", clean_name),
                );
            } else {
                // The tie is now known to hold: a person watching this task may go and
                // look at what it filed (T519(3)). Recorded only on this branch — a
                // failure to file leaves nothing to point at, and the notice above already
                // says so.
                ctx.set_result(crate::tasks::store::TaskResult {
                    media_id: media_id.to_owned(),
                });
            }
        }

        ctx.report_important(1.0, DetailCode::StageDone);

        // Ours to close only if we are the ones who opened it: the connection `finish` was
        // handed belongs to `run_upload`, which closes it once this function returns.
        if opened_here {
            working_conn.close().await;
        }
        Ok(())
    }

    /// What one attempt at comparing the checksum and publishing found.
    enum FinishStep {
        /// Published, whole and matching.
        Done,
        /// A cancellation landed after the checksum was found to match but before the
        /// publish — the one point past the checksum where stopping can still be honoured
        /// cleanly (see the comment in [`finish_once`]).
        CancelledBeforePublish,
        /// The staged file's checksum diverges from the source's. Not a break — a spoilt
        /// transfer, exactly as the ordinary path already treats it.
        ChecksumMismatch,
    }

    /// Why an attempt at the checksum-or-publish phase could not be completed.
    enum FinishBreak {
        /// A break in the connection, or the server not answering — worth trying again.
        Retriable(String),
        /// Something else: not a connection trouble, and retrying it would not help.
        Fatal(AppError),
    }

    /// Turn a break during the checksum comparison into what it means for retrying.
    fn classify_ssh(e: crate::ssh::SshError) -> FinishBreak {
        classify_upload(UploadError::from(e))
    }

    /// The same, for a break during publishing — `upload::publish` already returns
    /// `UploadError`, so there is nothing to convert.
    fn classify_upload(e: UploadError) -> FinishBreak {
        if e.is_retriable() {
            FinishBreak::Retriable(e.to_string())
        } else {
            FinishBreak::Fatal(AppError::new(ErrorCode::Internal).with_cause(e))
        }
    }

    /// One attempt: compare the checksum (when asked to) and publish.
    ///
    /// `verify_checksum` is `false` only right after a fresh reconnect found the file
    /// already published — there is nothing left on this attempt to verify by hand, because
    /// [`locate`] has already looked.
    async fn finish_once(
        conn: &crate::ssh::Connection,
        ctx: &crate::tasks::engine::TaskContext,
        plan: &UploadPlan,
        ours: &str,
        verify_checksum: bool,
    ) -> std::result::Result<FinishStep, FinishBreak> {
        if verify_checksum {
            match checksum::remote(conn, &plan.remote_temp).await {
                Ok(theirs) => {
                    if !checksum::matches(ours, &theirs) {
                        return Ok(FinishStep::ChecksumMismatch);
                    }
                }
                Err(e) => {
                    // ⚠ **A checksum failure is not always a checksum failure** (T570, found
                    // by the reconnect test itself:
                    // `a_cancellation_after_a_rediscovered_publish_ends_cancelled_with_a_
                    // notice` failed here on its first real run). Nothing else in this whole
                    // codebase ever renames `remote_temp` away except `publish`, a few lines
                    // below — so on the very same connection, in the very same attempt, "the
                    // staged file is gone" can only mean one thing: `publish` from an EARLIER
                    // attempt actually went through on the server, and only the
                    // acknowledgement of *that* attempt's success — not this one's — never
                    // made it back before the connection seemed to die and a retry began.
                    // Classifying that as a plain `Failed` (not retriable) would report a
                    // completed upload as an internal error, one connection recovery away
                    // from the every other case this whole phase exists to catch. So it is
                    // asked about directly, the same way the reconnect loop above already
                    // asks: not guessed at from the sha256sum's stderr text, but from what
                    // the server actually has.
                    return match locate(conn, plan).await {
                        Ok(Progress::Published) => Ok(FinishStep::Done),
                        // Genuinely still staged: the checksum failure was about something
                        // else — corruption, permissions — and the original classification
                        // stands.
                        Ok(Progress::Staged) => Err(classify_ssh(e)),
                        // Neither file is there any longer: not an ordinary break, and not a
                        // rediscovered publish either — something else removed the staged
                        // file. The same honest failure `checksum_and_publish`'s own
                        // reconnect loop gives this case, reached here instead because this
                        // attempt never left the connection it started on.
                        Ok(Progress::Gone) => Err(FinishBreak::Fatal(
                            AppError::new(ErrorCode::Internal).with_cause(format!(
                                "neither {} nor {} is on the server any longer — \
                                     something else removed the staged file while this \
                                     upload was running",
                                plan.remote_final, plan.remote_temp
                            )),
                        )),
                        // The server would not say either way: nothing has been learnt
                        // beyond what the original failure already said, so that is what is
                        // reported.
                        Err(_) => Err(classify_ssh(e)),
                    };
                }
            }
        }

        // The last point a cancellation can still be turned away cleanly (T503): after this,
        // the file is either about to be renamed into serving or already has been, and there
        // is no undoing a rename a viewer may already be watching through.
        if ctx.is_cancelled() {
            return Ok(FinishStep::CancelledBeforePublish);
        }

        upload::publish(conn, plan).await.map_err(classify_upload)?;
        Ok(FinishStep::Done)
    }

    /// What actually stands on the server for this plan (T570), told apart in the fewest
    /// round trips: published already, still staged, or neither.
    enum Progress {
        /// `remote_final` is there, the size the source is.
        Published,
        /// `remote_final` is not there, but `remote_temp` still is.
        Staged,
        /// Neither is there.
        Gone,
    }

    /// Ask the server what it actually has, after reconnecting.
    ///
    /// **Size is enough, and does not need a second checksum pass.** `publish` is a rename
    /// on one file system (`ensure_staging` refuses any other kind), and nothing but this
    /// same upload ever writes to `remote_temp` under this name — so a `remote_final` of
    /// exactly the source's length is that same file, moved, not a coincidence to be
    /// re-verified at the cost of reading it all again.
    async fn locate(conn: &crate::ssh::Connection, plan: &UploadPlan) -> upload::Result<Progress> {
        if let Some(size) = upload::remote_file_size(conn, &plan.remote_final).await? {
            if size == plan.total_bytes {
                return Ok(Progress::Published);
            }
            // Present, but not the size expected: not evidence either way about *this*
            // publish (a leftover of some other name, or a previous version) — fall through
            // exactly as though nothing had been found under the final name at all.
        }
        if upload::remote_file_size(conn, &plan.remote_temp)
            .await?
            .is_some()
        {
            return Ok(Progress::Staged);
        }
        Ok(Progress::Gone)
    }

    /// Where the checksum-and-publish phase, run through its own retry loop, ended up.
    enum FinishOutcome {
        /// The file is in serving. `bool` — whether the connection carrying it was opened
        /// inside this loop and so must be closed by the caller (the one passed in belongs
        /// to `run_upload`, which closes it itself).
        Published(crate::ssh::Connection, bool),
        /// A cancellation was honoured — cleanup already ran on whichever connection was
        /// open when it was.
        Cancelled,
    }

    fn too_many_finish_breaks() -> AppError {
        AppError::new(ErrorCode::SshUnreachable).with_detail(
            Detail::new(DetailCode::UploadTooManyBreaks)
                .with("attempts", FINISH_MAX_ATTEMPTS as u64),
        )
    }

    /// The checksum comparison and the entry into serving, reconnecting through a break at
    /// either point (T570).
    ///
    /// **Reasons about what the server actually has, rather than assuming the worst.** A
    /// break can land after the very thing it interrupted has already gone through — the
    /// `mv` in `publish` finishes on the server microseconds before the acknowledgement is
    /// lost — and blindly repeating the work would at best waste a checksum pass and at
    /// worst publish over a file a viewer has already started watching. So every reconnect
    /// asks [`locate`] first and only redoes what [`locate`] shows is still undone.
    async fn checksum_and_publish(
        conn: &crate::ssh::Connection,
        ctx: &crate::tasks::engine::TaskContext,
        secrets: &dyn crate::store::secrets::SecretStore,
        profile: &crate::domain::server_profile::ServerProfile,
        plan: &UploadPlan,
        ours: &str,
    ) -> std::result::Result<FinishOutcome, AppError> {
        let mut active = conn.clone();
        let mut opened_here = false;
        let mut have_connection = true;
        let mut verify_checksum = true;
        let mut delay = FIRST_RETRY_DELAY;

        for attempt in 1..=FINISH_MAX_ATTEMPTS {
            if have_connection {
                match finish_once(&active, ctx, plan, ours, verify_checksum).await {
                    Ok(FinishStep::Done) => {
                        return Ok(FinishOutcome::Published(active, opened_here))
                    }
                    Ok(FinishStep::CancelledBeforePublish) => {
                        upload::cleanup(&active, &plan.remote_temp).await;
                        if opened_here {
                            active.close().await;
                        }
                        return Ok(FinishOutcome::Cancelled);
                    }
                    Ok(FinishStep::ChecksumMismatch) => {
                        // The file does not enter serving, and we clean up after ourselves:
                        // a spoilt transfer must leave no trace (FR-032, FR-038).
                        upload::cleanup(&active, &plan.remote_temp).await;
                        if opened_here {
                            active.close().await;
                        }
                        return Err(AppError::new(ErrorCode::ChecksumMismatch)
                            .detail(DetailCode::UploadChecksumMismatch));
                    }
                    Err(FinishBreak::Fatal(e)) => {
                        if opened_here {
                            active.close().await;
                        }
                        return Err(e);
                    }
                    Err(FinishBreak::Retriable(reason)) => {
                        tracing::warn!(
                            attempt,
                            reason = %reason,
                            "the checksum-or-publish phase broke off; reconnecting"
                        );
                        if opened_here {
                            active.close().await;
                        }
                        have_connection = false;
                    }
                }
            }

            if attempt == FINISH_MAX_ATTEMPTS {
                return Err(too_many_finish_breaks());
            }
            if wait_before_retry(ctx, &mut delay).await == Waited::Cancelled {
                return cancel_during_finish(secrets, profile, plan).await;
            }

            match gate::open(secrets, profile, Intent::Change).await {
                Ok(opened) => match locate(&opened.conn, plan).await {
                    Ok(Progress::Published) => {
                        return Ok(FinishOutcome::Published(opened.conn, true))
                    }
                    Ok(Progress::Staged) => {
                        active = opened.conn;
                        opened_here = true;
                        have_connection = true;
                        verify_checksum = true;
                    }
                    Ok(Progress::Gone) => {
                        // Not "an ordinary break" any longer: something else removed the
                        // staged file between attempts, and there is nothing left to retry
                        // towards. Not retried forever — a bar the person is owed an honest
                        // answer about rather than a task that spins until the attempt
                        // ceiling.
                        opened.conn.close().await;
                        return Err(AppError::new(ErrorCode::Internal).with_cause(format!(
                            "neither {} nor {} is on the server any longer — something else \
                             removed the staged file while this upload was reconnecting",
                            plan.remote_final, plan.remote_temp
                        )));
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "could not learn what is on the server after reconnecting"
                        );
                        opened.conn.close().await;
                    }
                },
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "could not reconnect during the checksum-or-publish phase"
                    );
                }
            }
        }
        unreachable!("the loop above returns on every path by its last iteration")
    }

    /// Whether a cancellation during the checksum-or-publish phase's own retry wait may be
    /// honoured (T570).
    ///
    /// **The same "window that cannot be closed" the ordinary path already lives with**,
    /// reached from a different direction: the break that put this phase into its retry loop
    /// may itself have landed after the publish already went through and only its
    /// acknowledgement was lost. So before honouring the stop, one more connection is opened
    /// for the sole purpose of finding out — best effort, exactly like `sweep_after_cancelling`:
    /// a failure to check or to clean up must not turn an honoured cancellation into a
    /// reported failure.
    async fn cancel_during_finish(
        secrets: &dyn crate::store::secrets::SecretStore,
        profile: &crate::domain::server_profile::ServerProfile,
        plan: &UploadPlan,
    ) -> std::result::Result<FinishOutcome, AppError> {
        match gate::open(secrets, profile, Intent::Change).await {
            Ok(opened) => match locate(&opened.conn, plan).await {
                Ok(Progress::Published) => Ok(FinishOutcome::Published(opened.conn, true)),
                Ok(Progress::Staged) | Ok(Progress::Gone) => {
                    upload::cleanup(&opened.conn, &plan.remote_temp).await;
                    opened.conn.close().await;
                    Ok(FinishOutcome::Cancelled)
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "could not learn what is on the server while honouring a cancellation"
                    );
                    opened.conn.close().await;
                    Ok(FinishOutcome::Cancelled)
                }
            },
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "a cancelled transfer's leftovers could not be checked or removed after \
                     the checksum-or-publish phase broke off"
                );
                Ok(FinishOutcome::Cancelled)
            }
        }
    }

    /// Put a file into the catalogue under the medium it belongs to.
    ///
    /// **Safe to repeat** (principle V). The path is removed from wherever it was before it is
    /// added, so a second run of the same upload leaves one entry and not two — and a file
    /// moved by hand since is not silently moved back by a retry, because a retry of an upload
    /// that already finished does not reach this at all.
    ///
    /// A missing medium is not invented: somebody deleted it between the choice and the end of
    /// the transfer, and creating it again would resurrect a thing they got rid of. The file
    /// stays unrecognised, which is a state the library shows plainly.
    async fn file_it_under(
        conn: &crate::ssh::Connection,
        video_dir: &str,
        media_id: &str,
        name: &str,
    ) -> std::result::Result<(), AppError> {
        let manifest = crate::server::manifest_io::read(conn, video_dir).await?;
        let next = manifest
            .with_file_under(media_id, name, false)
            .ok_or_else(|| {
                AppError::new(ErrorCode::InvalidInput)
                    .detail(DetailCode::MediaNotFound)
                    .with_cause(media_id)
            })?;
        crate::server::manifest_io::write(conn, video_dir, &next, manifest.generation).await?;
        Ok(())
    }

    /// Remove what a cancelled transfer left on the server, on a connection of its own.
    ///
    /// **A second connection, because by here there is no first one** (T521, FR-038). The
    /// cancellation that reaches this arrives during the pause between attempts, and the pause
    /// deliberately holds no connection: waiting out a doubling backoff with one open would
    /// take a channel from the server for minutes, and there are eight (R-04).
    ///
    /// **Failing to clean up is not reported, and that is the same rule `upload::cleanup`
    /// states**: the cancellation has already happened, the person asked for it and got it,
    /// and turning "we could not tidy up afterwards" into a failure would tell them their stop
    /// did not work. It is logged, which is where somebody looking for a stray file will look.
    async fn sweep_after_cancelling(
        secrets: &dyn crate::store::secrets::SecretStore,
        profile: &crate::domain::server_profile::ServerProfile,
        plan: &UploadPlan,
    ) {
        match gate::open(secrets, profile, Intent::Change).await {
            Ok(opened) => {
                upload::cleanup(&opened.conn, &plan.remote_temp).await;
                opened.conn.close().await;
            }
            Err(e) => {
                tracing::warn!(error = %e, "a cancelled transfer's leftovers could not be removed")
            }
        }
    }

    /// How a wait between attempts ended.
    ///
    /// ⚠ **An enumeration and not a `Result`, so that the cancellation cannot be `?`-ed
    /// away** (T521). It was a `Result<(), AppError>`, and the one caller wrote
    /// `wait_before_retry(..).await?` — which carried the cancellation straight out of the
    /// function, past both places that clean up, leaving the part-file on the server for good
    /// (FR-038). `?` was the natural thing to write and the compiler had no opinion. It has
    /// one now: this is not an error, so there is nothing to propagate, and a caller has to
    /// say what happens in each case.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[must_use]
    enum Waited {
        /// The pause ran its course; try again.
        Elapsed,
        /// Somebody pressed stop while we were waiting.
        Cancelled,
    }

    /// Wait before retrying, without missing a cancellation.
    async fn wait_before_retry(
        ctx: &crate::tasks::engine::TaskContext,
        delay: &mut Duration,
    ) -> Waited {
        let cancel = ctx.cancel_token();
        tokio::select! {
            _ = tokio::time::sleep(*delay) => {}
            _ = cancel.cancelled() => return Waited::Cancelled,
        }
        *delay = (*delay * 2).min(MAX_RETRY_DELAY);
        Waited::Elapsed
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
