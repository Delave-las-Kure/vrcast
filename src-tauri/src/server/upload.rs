//! T086–T092 — sending a file to the server, carrying on after a break (R-05).
//!
//! How it works: the file is written into a staged file **outside the serving directory**,
//! in windows of a few megabytes, written at an offset. The position to carry on from is
//! taken from the size of that staged file on the server. When it ends comes the checksum
//! comparison and the entry into serving, by a single rename.
//!
//! Why not cutting into pieces and gluing them afterwards, the way the old script did:
//! pieces take up a second copy of the same volume on the server's disk, gluing is another
//! pass over the whole file, and the position to carry on from needs an account of its own.
//! Writing at an offset gives the same thing for free: the position is the file's size.
//!
//! **One attempt, not the whole transfer.** This module makes one attempt and returns how
//! far it got. Reconnecting and retrying live one floor up (`commands::upload`), where the
//! server profile is known. Mixing that in here would drag both the secrets and the retry
//! rules along with it.

use super::{join_remote, shell_quote};
use crate::domain::progress_estimate::ProgressEstimate;
use crate::domain::rate_limit::RateLimiter;
use crate::domain::transfer::{decide_resume, ResumeDecision, WINDOW_BYTES};
use crate::ssh::{Connection, SshError};
use crate::tasks::engine::TaskContext;
use russh_sftp::protocol::OpenFlags;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// What to send, and where.
#[derive(Debug, Clone)]
pub struct UploadPlan {
    pub local_path: PathBuf,
    /// The full path of the staged file on the server.
    pub remote_temp: String,
    /// The full path of the final file in the serving directory.
    pub remote_final: String,
    pub total_bytes: u64,
    pub limit_bps: Option<u64>,
}

/// How an attempt ended.
#[derive(Debug, thiserror::Error)]
pub enum UploadError {
    /// The connection broke. That is not a fault but an ordinary thing on a transfer that
    /// runs for hours: reconnect and carry on from where it got to.
    #[error("transfer interrupted: {0}")]
    Interrupted(String),

    /// There is more on the server than there is in the source. Carrying on is impossible:
    /// the result would be two different files glued together, and it would only be found
    /// at the comparison — when the time has already been spent.
    #[error("the staged file on the server ({temp} B) is larger than the source ({total} B)")]
    SourceChanged { temp: u64, total: u64 },

    #[error("task cancelled")]
    Cancelled,

    #[error("{0}")]
    Failed(String),
}

impl UploadError {
    /// Whether it is worth trying again.
    pub fn is_retriable(&self) -> bool {
        matches!(self, Self::Interrupted(_))
    }
}

pub type Result<T> = std::result::Result<T, UploadError>;

impl From<SshError> for UploadError {
    fn from(e: SshError) -> Self {
        let text = crate::store::redact::safe_display(&e);
        match e {
            // A break, and everything that comes with it, is a reason to retry.
            SshError::Unreachable { .. } | SshError::Protocol(_) | SshError::Sftp { .. } => {
                Self::Interrupted(text)
            }
            _ => Self::Failed(text),
        }
    }
}

/// Prepare the staging directory and make sure it is on the same file system.
///
/// The check is not a formality: a rename is indivisible only within one file system.
/// Across a boundary it turns into a copy followed by a delete — that is, into the very
/// minutes during which a half-copied file sits in the serving directory. Quietly getting
/// that instead of an indivisible entry into serving is the worst outcome, because it shows
/// up for a viewer.
pub async fn ensure_staging(conn: &Connection, staging_dir: &str, video_dir: &str) -> Result<()> {
    let out = conn
        .exec(&format!(
            "mkdir -p -- {staging} && stat -c %d {staging} && stat -c %d {videos}",
            staging = shell_quote(staging_dir),
            videos = shell_quote(video_dir)
        ))
        .await?;

    if !out.ok() {
        return Err(UploadError::Failed(format!(
            "could not prepare the staging directory {staging_dir}: {}",
            out.stderr.trim()
        )));
    }

    let mut lines = out.stdout.lines();
    let staging_fs = lines.next().unwrap_or_default().trim().to_owned();
    let videos_fs = lines.next().unwrap_or_default().trim().to_owned();

    if staging_fs.is_empty() || videos_fs.is_empty() {
        return Err(UploadError::Failed(String::from(
            "could not learn which file system the directories on the server are on",
        )));
    }
    if staging_fs != videos_fs {
        return Err(UploadError::Failed(format!(
            "the staging directory {staging_dir} and the serving directory {video_dir} are on \
             different file systems. Entry into serving would stop being indivisible, and a \
             viewer could get a half-copied file"
        )));
    }
    Ok(())
}

/// How much already lies in the staged file on the server.
pub async fn uploaded_so_far(conn: &Connection, remote_temp: &str) -> Result<u64> {
    // A missing file is a legitimate answer of "zero" rather than an error: that is what a
    // first attempt looks like. So the size is asked for with a command that stays quiet in
    // that case.
    let out = conn
        .exec(&format!(
            "stat -c %s -- {} 2>/dev/null || echo 0",
            shell_quote(remote_temp)
        ))
        .await?;
    Ok(out.trimmed().trim().parse::<u64>().unwrap_or(0))
}

/// Write one window, and give up on it when the connection has died under it.
///
/// ⚠ **Without this the transfer does not fail — it stops** (T483, measured 2026-09-04). The
/// write waits for an acknowledgement from the far end, and when that end is gone the
/// acknowledgement never comes and the wait never ends. Nothing above notices: retrying lives
/// a floor up and is reached by an `Interrupted`, which is never returned, so the task sits at
/// `Running` with its progress frozen for as long as anybody leaves it. Measured: the staged
/// file stopped at 4,896,849,920 bytes and had not moved three hours later, with the task
/// still reporting 11.7% and no error at all. "Without manual intervention" (SC-003) then
/// means "not at all".
///
/// Two ways out, because they cover different failures. The keepalives close the connection
/// about ninety seconds after the far end stops answering (`ssh::fingerprint::client_config`),
/// and that is the ordinary case: a break is noticed in a minute and a half and the retry
/// takes over. The ceiling is for a connection that is neither alive nor closed — a state no
/// keepalive reports — and is deliberately far above any real window: four megabytes at even
/// two hundred kilobits a second is under three minutes.
///
/// **Abandoning a write half-done is safe here and nowhere else.** What is resumed from is the
/// size of the staged file on the server, read afresh (`decide_resume`), so a torn tail costs
/// the window and not the transfer.
async fn write_window<W: tokio::io::AsyncWrite + Unpin>(
    conn: &Connection,
    remote: &mut W,
    bytes: &[u8],
) -> Result<()> {
    use tokio::io::AsyncWriteExt;

    alive(conn, remote.write_all(bytes))
        .await
        .map_err(|e| UploadError::Interrupted(format!("the write to the server broke off: {e}")))
}

/// Wait on the far end only while there is a far end to wait on.
///
/// ⚠ **Measured, and narrower than it first looked.** `exec` does not need this: a command in
/// flight comes back by itself when its session is killed, because the channel closes and the
/// loop reading it ends — checked on 2026-09-04, it returned in ten seconds with a failure.
/// What does need it is anything holding an SFTP file, which waits for a reply to a request
/// the far end will never read. That is the whole difference, and it is why this sits here
/// rather than inside the connection.
async fn alive<T, E, F>(conn: &Connection, work: F) -> std::result::Result<T, WriteGaveUp<E>>
where
    F: std::future::Future<Output = std::result::Result<T, E>>,
{
    let dead = async {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            if !conn.is_alive() {
                return;
            }
        }
    };

    tokio::select! {
        done = work => done.map_err(WriteGaveUp::Failed),
        _ = dead => Err(WriteGaveUp::ConnectionDied),
        _ = tokio::time::sleep(WINDOW_CEILING) => Err(WriteGaveUp::TookTooLong),
    }
}

/// Why waiting on the far end ended without an answer.
enum WriteGaveUp<E> {
    Failed(E),
    ConnectionDied,
    TookTooLong,
}

impl<E: std::fmt::Display> std::fmt::Display for WriteGaveUp<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Failed(e) => write!(f, "{e}"),
            Self::ConnectionDied => write!(f, "the connection to the server died"),
            Self::TookTooLong => write!(f, "it took longer than any window can"),
        }
    }
}

/// The longest a single window may take before the connection is presumed gone.
///
/// See [`write_window`]. Generous on purpose: it is the answer for a connection that neither
/// carries data nor reports itself closed, not the ordinary way a break is noticed.
const WINDOW_CEILING: Duration = Duration::from_secs(600);

/// One attempt at the transfer. Returns how much lies in the staged file in all.
///
/// `estimate` is passed in from outside and survives reconnections: otherwise the time
/// estimate would start from nothing after every break and show nonsense for the first few
/// seconds.
pub async fn transfer_once(
    conn: &Connection,
    ctx: &TaskContext,
    plan: &UploadPlan,
    estimate: &mut ProgressEstimate,
) -> Result<u64> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

    let already = uploaded_so_far(conn, &plan.remote_temp).await?;
    let offset = match decide_resume(already, plan.total_bytes, WINDOW_BYTES) {
        ResumeDecision::AlreadyComplete => return Ok(already),
        ResumeDecision::Mismatch { temp, total } => {
            return Err(UploadError::SourceChanged { temp, total })
        }
        ResumeDecision::FromStart => 0,
        ResumeDecision::Continue { offset } => offset,
    };

    let mut local = tokio::fs::File::open(&plan.local_path)
        .await
        .map_err(|e| UploadError::Failed(format!("the source would not open: {e}")))?;
    local
        .seek(std::io::SeekFrom::Start(offset))
        .await
        .map_err(|e| {
            UploadError::Failed(format!("could not seek to the position in the source: {e}"))
        })?;

    let sftp = conn.sftp().await?;
    let mut remote = sftp
        .open_with_flags(
            plan.remote_temp.clone(),
            OpenFlags::WRITE | OpenFlags::CREATE,
        )
        .await
        .map_err(|e| UploadError::Interrupted(crate::store::redact::safe_display(&e)))?;
    remote
        .seek(std::io::SeekFrom::Start(offset))
        .await
        .map_err(|e| {
            UploadError::Interrupted(format!("could not seek to the position on the server: {e}"))
        })?;

    let mut limiter = RateLimiter::new(plan.limit_bps);
    let mut sent = offset;
    let mut buf = vec![0u8; WINDOW_BYTES as usize];

    loop {
        // Cancelling and pausing are checked between windows: tearing off a write in the
        // middle would leave a broken tail in the file that has to be written over
        // afterwards.
        ctx.wait_while_paused().await;
        if ctx.is_cancelled() {
            return Err(UploadError::Cancelled);
        }

        let read = local
            .read(&mut buf)
            .await
            .map_err(|e| UploadError::Failed(format!("the source will not read: {e}")))?;
        if read == 0 {
            break;
        }

        let wait = limiter.delay_for(read as u64, Instant::now());
        if !wait.is_zero() {
            // The wait keeps an eye on cancelling: otherwise, with a limit of a hundred
            // kilobytes, a cancel would wait its turn for tens of seconds.
            //
            // The token is named in a variable of its own deliberately: a temporary value
            // inside `select!` lives only to the end of the expression and does not last
            // until the end of the wait.
            let cancel = ctx.cancel_token();
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = cancel.cancelled() => return Err(UploadError::Cancelled),
            }
        }

        write_window(conn, &mut remote, &buf[..read]).await?;

        sent += read as u64;
        estimate.record(Instant::now(), sent);
        report(ctx, plan, estimate, sent);
    }

    // The same guard as the windows: these wait on the far end exactly as a write does, and a
    // connection that died between the last window and the close would hold them just as long.
    alive(conn, remote.flush()).await.map_err(|e| {
        UploadError::Interrupted(format!("the write to the server did not finish: {e}"))
    })?;
    alive(conn, remote.shutdown()).await.map_err(|e| {
        UploadError::Interrupted(format!("the file on the server would not close: {e}"))
    })?;

    Ok(sent)
}

fn report(ctx: &TaskContext, plan: &UploadPlan, estimate: &ProgressEstimate, sent: u64) {
    let progress = if plan.total_bytes == 0 {
        1.0
    } else {
        sent as f64 / plan.total_bytes as f64
    };
    let remaining = plan.total_bytes.saturating_sub(sent);
    ctx.report_transfer(
        progress,
        estimate.speed_bps().unwrap_or(0) as i64,
        estimate.eta(remaining).map_or(0, |d| d.as_secs() as i64),
    );
    // And separately — to disk, far less often. An upload runs for hours, and after the
    // application restarts a person must see how much has already been sent, not zero.
    ctx.save_progress(progress);
}

/// Enter the file into serving in one indivisible act (FR-033).
///
/// Until that moment the file is unreachable by its link: it lies outside the serving
/// directory. After it, it is reachable whole. There is no state in between, and that is
/// the whole reason the staging happens off to the side.
pub async fn publish(conn: &Connection, plan: &UploadPlan) -> Result<()> {
    let out = conn
        .exec(&format!(
            "mv -f -- {} {}",
            shell_quote(&plan.remote_temp),
            shell_quote(&plan.remote_final)
        ))
        .await?;
    if !out.ok() {
        return Err(UploadError::Failed(format!(
            "the file could not be entered into serving: {}",
            out.stderr.trim()
        )));
    }
    tracing::info!(file = %plan.remote_final, "the file entered serving");
    Ok(())
}

/// Clean up after a cancellation (FR-038).
///
/// A failure to clean up is not returned: the cancellation has already happened, and there
/// is no point turning it into a failure because a staged file would not delete. But
/// keeping quiet will not do either — litter piles up unnoticed.
pub async fn cleanup(conn: &Connection, remote_temp: &str) {
    let result = conn
        .exec(&format!("rm -f -- {}", shell_quote(remote_temp)))
        .await;
    match result {
        Ok(out) if out.ok() => {}
        Ok(out) => {
            tracing::warn!(file = remote_temp, stderr = %out.stderr.trim(), "the staged file would not delete")
        }
        Err(e) => {
            tracing::warn!(file = remote_temp, error = %e, "the staged file would not delete")
        }
    }
}

/// The full path of the final file in the serving directory.
pub fn final_path(video_dir: &str, remote_name: &str) -> String {
    join_remote(
        video_dir,
        &crate::domain::remote_name::sanitize(remote_name),
    )
}
