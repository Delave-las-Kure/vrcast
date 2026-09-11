//! T196 — getting the cutting onto a server and watching it from a distance.
//!
//! The rules and the script itself are in [`crate::domain::hls_package`]; here is only the
//! plumbing.
//!
//! **The work is detached from the connection that started it.** That is not tidiness: the
//! cutting used to run in the foreground of an SSH session, and on mandoup a flood of
//! brute-force attempts cut the session in the middle of the third variant. Two finished
//! variants were rescued by hand and the ladder was left without its third rung. A detached
//! process outlives the connection; what breaks then is the watching, and the watching can
//! simply reconnect.

use std::time::Duration;

use crate::domain::hls_package::{self, CutFacts, Progress, ToCut};
use crate::ssh::{Connection, Result, SshError};
use crate::tasks::engine::TaskContext;

/// What is being cut, and where.
pub struct Cutting<'a> {
    pub conn: &'a Connection,
    pub video_dir: &'a str,
    /// `user:group` the finished files must belong to, as the serving user.
    pub owner: &'a str,
    /// The media's own directory under the serving one.
    pub base: &'a str,
    pub variants: &'a [ToCut],
}

/// How [`Cutting::run`] ended.
///
/// A type of its own rather than reusing [`SshError`] (T597) — by the same precedent
/// `UploadError` sets in `server::upload`: that one carries its own `Cancelled` apart from
/// the SSH failures it also wraps, for the same reason. `SshError` has no shape for "a
/// person asked to stop"; bolting one on there would widen a type every ordinary command
/// failure also has to match on, for the sake of the one caller that watches a
/// `TaskContext`.
#[derive(Debug, thiserror::Error)]
pub enum CuttingError {
    #[error(transparent)]
    Ssh(#[from] SshError),

    #[error("the cutting was cancelled")]
    Cancelled,
}

pub type CuttingResult<T> = std::result::Result<T, CuttingError>;

/// How often the log is asked for.
///
/// **A brief channel each time rather than one held open.** A connection may have only so
/// many channels at once (R-04), and two of them are already held for as long as viewers
/// are being watched. A build that took a third would leave five for everything else, and
/// the thing that then fails is whatever the person does next.
const ASK_EVERY: Duration = Duration::from_secs(5);

impl Cutting<'_> {
    fn script_path(&self) -> String {
        format!("/tmp/vrcast-hls-{}.sh", self.base)
    }

    fn log_path(&self) -> String {
        format!("/tmp/vrcast-hls-{}.log", self.base)
    }

    /// Put the script on the server and start it, detached.
    pub async fn start(&self) -> Result<()> {
        guard_base(self.base)?;

        let script = self.script_path();
        let log = self.log_path();
        write_file(self.conn, &script, &hls_package::script_text()).await?;

        let mut args = vec![
            super::shell_quote(self.video_dir),
            super::shell_quote(self.owner),
            super::shell_quote(self.base),
        ];
        for variant in self.variants {
            args.push(super::shell_quote(&format!(
                "{}={}",
                variant.sub, variant.file
            )));
        }

        // `setsid` and `nohup` together: the first takes it out of our session so that the
        // session ending does not reach it, the second detaches it from the terminal. One
        // without the other has let work die on this project before.
        self.conn
            .exec(&format!(
                "setsid nohup bash {} {} > {} 2>&1 < /dev/null & echo started",
                super::shell_quote(&script),
                args.join(" "),
                super::shell_quote(&log),
            ))
            .await?
            .require_ok("could not start the cutting")?;
        Ok(())
    }

    /// What the script has said so far.
    pub async fn progress(&self) -> Result<Progress> {
        let out = self
            .conn
            .exec(&format!(
                "cat {} 2>/dev/null || true",
                super::shell_quote(&self.log_path())
            ))
            .await?;
        Ok(hls_package::read_log(&out.stdout))
    }

    /// Whether the script is still running.
    ///
    /// ⚠ **The pattern is self-excluding, and that is not decoration.** `conn.exec` runs
    /// every command through the remote shell (`bash -lc '<command>'`), so the invocation
    /// `pgrep -f "vrcast-hls-{base}.sh"` has its own command line containing the very text
    /// it is searching for — `pgrep -f` matches against the whole command line of every
    /// process, and that includes the shell that is running `pgrep` itself. Found while
    /// writing T597's own test: `still_running()` reported `true` for seconds after the
    /// actual script and its `ffmpeg` had both already exited, because the check was really
    /// observing its own invocation, not the thing it meant to ask about. The classic fix —
    /// `pgrep -f '[v]rcast-hls-…'` — makes the search regex match the plain text
    /// `vrcast-hls-…` wherever it occurs, while `pgrep`'s own command line contains the
    /// bracketed form `[v]rcast-hls-…`, which the regex does not match against itself.
    pub async fn still_running(&self) -> Result<bool> {
        let out = self
            .conn
            .exec(&format!(
                "pgrep -f {} >/dev/null && echo yes || echo no",
                super::shell_quote(&self_excluding_pattern(self.base))
            ))
            .await?;
        Ok(out.trimmed() == "yes")
    }

    /// Ask the server to end the detached cutting process, best-effort (T597).
    ///
    /// The same self-excluding pattern [`Self::still_running`] builds and explains — `pkill`
    /// is `pgrep`'s twin, built from the same command-line search, and needs the identical
    /// guard for the identical reason.
    ///
    /// **Best-effort, like `tidy_up`/`upload::cleanup`, and deliberately not a hard error.**
    /// By the time anything calls this, a cancellation has already happened (or is about to
    /// be reported); failing to reach a process that may already have finished on its own
    /// must not turn an honest cancellation into a reported failure. A refusal is logged and
    /// swallowed rather than propagated.
    ///
    /// `pkill`'s own exit code 1 ("no process matched") is not a failure here — it is what a
    /// cutting that had already finished, or was never started, looks like, and is exactly as
    /// welcome an outcome as actually having killed something.
    pub async fn stop_remote(&self) {
        let pattern = self_excluding_pattern(self.base);
        match self
            .conn
            .exec(&format!("pkill -f {}", super::shell_quote(&pattern)))
            .await
        {
            Ok(out) if out.ok() || out.exit_code == Some(1) => {}
            Ok(out) => {
                tracing::warn!(
                    base = self.base,
                    exit_code = ?out.exit_code,
                    stderr = out.stderr.trim(),
                    "pkill against the detached cutting process did not end cleanly"
                );
            }
            Err(e) => {
                tracing::warn!(
                    base = self.base,
                    error = %e,
                    "could not ask the server to stop the detached cutting process"
                );
            }
        }
    }

    /// What each variant turned out to be, read back from the server.
    pub async fn facts(&self) -> Result<Vec<CutFacts>> {
        let mut all = Vec::new();
        for variant in self.variants {
            let path = format!(
                "{}/{}/{}/.facts",
                self.video_dir.trim_end_matches('/'),
                self.base,
                variant.sub
            );
            let out = self
                .conn
                .exec(&format!("cat {}", super::shell_quote(&path)))
                .await?
                .require_ok("could not read what the cutting reported")?;
            all.push(
                hls_package::read_facts(&out.stdout)
                    .map_err(|e| SshError::Exec(format!("{}: {e}", variant.sub)))?,
            );
        }
        Ok(all)
    }

    /// Start the cutting and wait for it, telling the caller as each variant lands.
    ///
    /// Resumes rather than restarts: the script itself skips a variant that is already cut
    /// whole, so running this again after a break picks up where it stopped (FR-048).
    ///
    /// ⚠ **T597 — takes a `TaskContext` and actually answers a cancellation, rather than only
    /// polling every [`ASK_EVERY`].** `server::upload` already imports
    /// `crate::tasks::engine::TaskContext` directly (`server/upload.rs`), so the `server`
    /// layer being handed a live task's context is not new — the same precedent this follows.
    /// Before this, `ladder_build::run`'s only cancellation check was `bail_if_cancelled()`
    /// **before** the cutting phase started (`tasks/ladder_build.rs:176`); once inside this
    /// loop, a stop pressed on the interface did nothing but wait out the sleep, and the
    /// detached server-side process (`setsid nohup`) went on consuming CPU and disk on the
    /// server after the task had already reported itself cancelled — the very drift a
    /// person cancelling a build believes they just stopped.
    pub async fn run<F>(
        &self,
        ctx: &TaskContext,
        mut on_progress: F,
    ) -> CuttingResult<Vec<CutFacts>>
    where
        F: FnMut(&Progress),
    {
        self.start().await?;

        let mut last_seen = 0usize;
        let cancel_token = ctx.cancel_token();
        loop {
            // Raced rather than checked only between iterations: waiting out the whole of
            // `ASK_EVERY` before noticing a cancellation would answer "stop" several seconds
            // late on every single poll, and this is the one thing T597 exists to shorten.
            tokio::select! {
                _ = tokio::time::sleep(ASK_EVERY) => {}
                _ = cancel_token.cancelled() => {
                    // Best-effort, like `tidy_up`/`upload::cleanup` — see `stop_remote`'s own
                    // doc comment for why a failure here must not turn an honest cancellation
                    // into a reported one.
                    self.stop_remote().await;
                    return Err(CuttingError::Cancelled);
                }
            }

            // A broken poll is not a broken build: the work is detached, so we simply ask
            // again. Only the work itself ending decides anything.
            let Ok(progress) = self.progress().await else {
                continue;
            };
            if let Some(why) = &progress.failed {
                return Err(SshError::Exec(format!("the cutting stopped: {why}")).into());
            }
            if progress.cut.len() > last_seen {
                last_seen = progress.cut.len();
                on_progress(&progress);
            }
            if progress.all_done {
                break;
            }

            // No marker and no process: it was killed, ran out of room, or the machine was
            // restarted under it. Whatever it was, it is not going to finish on its own,
            // and waiting for a marker that will never come is the worst way to find out.
            if !self.still_running().await.unwrap_or(true) {
                return Err(SshError::Exec(String::from(
                    "the cutting is no longer running and never said it had finished",
                ))
                .into());
            }
        }

        Ok(self.facts().await?)
    }

    /// Remove what the cutting left behind on the server.
    ///
    /// The segments stay; the script and its log do not. They live in `/tmp` and would go
    /// on their own eventually, but "eventually" on a server that is never restarted is a
    /// long time.
    pub async fn tidy_up(&self) -> Result<()> {
        self.conn
            .exec(&format!(
                "rm -f {} {}",
                super::shell_quote(&self.script_path()),
                super::shell_quote(&self.log_path())
            ))
            .await?;
        Ok(())
    }
}

/// Build a `pgrep -f`/`pkill -f` pattern that does not match its own invocation.
///
/// See [`Cutting::still_running`]'s doc comment for the whole story: `pgrep -f pattern` run
/// through `conn.exec` (itself `bash -lc '<command>'` on the far end) has `pattern` sitting
/// right there in its own command line, and matches itself. Bracketing the first character
/// turns it into a one-character character class in the regex `pgrep -f` builds internally,
/// which still matches the plain text everywhere else but no longer matches the bracketed
/// text of `pgrep`'s own argument.
fn self_excluding_pattern(base: &str) -> String {
    let full = format!("vrcast-hls-{base}.sh");
    match full.chars().next() {
        Some(c) => format!("[{c}]{}", &full[c.len_utf8()..]),
        None => full,
    }
}

/// The media's directory name goes into a path on the server and into a process name.
///
/// It comes from a slug, which is already restricted — but this is the last place before it
/// becomes part of a command, and a check here costs nothing while the alternative is the
/// kind of mistake that is only ever found the hard way.
fn guard_base(base: &str) -> Result<()> {
    let sound = !base.is_empty()
        && base.len() <= crate::domain::media::MAX_SLUG_LEN
        && base
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if sound {
        Ok(())
    } else {
        Err(SshError::Exec(format!(
            "\"{base}\" is not a name a directory on the server may have"
        )))
    }
}

/// Write a file to the server.
async fn write_file(conn: &Connection, path: &str, body: &str) -> Result<()> {
    use tokio::io::AsyncWriteExt;

    let sftp = conn.sftp().await?;
    let written = async {
        // `create` rather than `write`: the library's `write` does not make a file that is
        // not there, and gives "no such file" on a path that does not exist yet — a name
        // that promises one thing and does another, found on a live server on 2026-08-25.
        let mut file = sftp.create(path.to_owned()).await?;
        file.write_all(body.as_bytes()).await?;
        file.flush().await?;
        file.shutdown().await?;
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;

    written.map_err(|e| SshError::sftp(crate::store::redact::safe_display(&*e)))
}
