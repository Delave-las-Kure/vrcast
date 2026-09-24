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
//!
//! **And because it outlives the connection, ending it is a claim that has to be checked**
//! (T605). A detached process group does not stop because the task that started it says it
//! has; it stops when the server says every process of it is gone. Until then the build is
//! not over — not cancelled, not failed — whatever the connection is doing.

use std::time::Duration;

use crate::domain::hls_package::{self, CutFacts, GroupRecord, Progress, ToCut};
use crate::ssh::{Connection, Result, SshError};
use crate::tasks::engine::TaskContext;

pub use super::marked::StopProblem;
pub use crate::domain::hls_package::Stopped;

use super::marked::Patience;

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

    /// A cutting of the same directory is alive on the server already, and nothing was
    /// started (T605). See [`Cutting::start`] for why this refuses rather than stops it.
    #[error("a cutting of \"{base}\" is already running on the server")]
    AlreadyRunning { base: String },

    /// The work ended — cancelled or failed — but that its processes on the server are
    /// gone could **not** be confirmed (T605).
    ///
    /// Not a final answer and not to be reported as one: the caller settles it with
    /// [`PendingStop::confirm`], which returns only once the stop is confirmed, and only
    /// then hands back how the work really ended. Reporting `Cancelled` — or `Failed`,
    /// which frees the media's directory just the same (`running_build_for`) — while a
    /// process of the cutting may still be writing into that directory is exactly what
    /// constitution III forbids.
    #[error("the cutting ended ({}), but its stop on the server is not confirmed: {}", .0.then, .0.why)]
    StopUnconfirmed(PendingStop),
}

pub type CuttingResult<T> = std::result::Result<T, CuttingError>;

/// Which start of a cutting a process belongs to: `VRCAST_HLS_JOB=<base>:<uuid>`, carried
/// in the environment of every process the start made (see `domain::hls_package::JOB_VAR`).
///
/// One per start rather than one per directory: a stop aimed at this mark cannot reach a
/// cutting of the same directory started by somebody else — another instance of the
/// application on another machine, say — which is not ours to stop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobMark(String);

impl JobMark {
    /// A fresh mark for one start of a cutting of `base`.
    pub fn for_base(base: &str) -> Self {
        Self(format!("{base}:{}", uuid::Uuid::new_v4().simple()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A cutting that was started, and what it recorded about itself.
#[derive(Debug, Clone)]
pub struct Started {
    pub mark: JobMark,
    /// The wrapper's own record, already checked: it leads a group and a session of its
    /// own, apart from the shell that launched it.
    pub group: GroupRecord,
}

/// A stop that has to be confirmed before the work may be reported as ended.
#[derive(Debug)]
pub struct PendingStop {
    pub mark: JobMark,
    /// How the work ended — what is handed back once the stop is confirmed.
    pub then: Box<CuttingError>,
    /// Why the last attempt did not confirm it.
    pub why: String,
}

/// How often the log is asked for.
///
/// **A brief channel each time rather than one held open.** A connection may have only so
/// many channels at once (R-04), and two of them are already held for as long as viewers
/// are being watched. A build that took a third would leave five for everything else, and
/// the thing that then fails is whatever the person does next.
const ASK_EVERY: Duration = Duration::from_secs(5);

/// The ceiling on the start command. The launcher itself waits up to five seconds for the
/// wrapper's record (`RECORD_TICKS` in the domain module).
const START_CEILING: Duration = Duration::from_secs(60);

/// Run one of the domain module's scripts with arguments — see
/// [`super::marked::bash_with_args`] for why `bash` by name and what the sentinel is.
fn bash_with_args(script: &str, args: &[String]) -> String {
    super::marked::bash_with_args("vrcast-hls", script, args)
}

/// Stop every process of one start on the server, and confirm it (T605).
///
/// [`super::marked::stop_confirmed`] aimed at `VRCAST_HLS_JOB=<mark>`, signalling at once:
/// a cutting is only ever redone, never left half-written in a way a repeat cannot mend, so
/// there is nothing to wait for. See there for the outcomes; none but `Ok` is a stop.
pub async fn stop_confirmed(
    conn: &Connection,
    mark: &JobMark,
) -> std::result::Result<Stopped, StopProblem> {
    super::marked::stop_confirmed(conn, hls_package::JOB_VAR, mark.as_str(), Patience::NONE).await
}

/// Whether any live process carries this mark (a prefix: `base:` or `base:uuid`).
async fn anything_alive(conn: &Connection, want: &str) -> Result<bool> {
    super::marked::anything_alive(conn, hls_package::JOB_VAR, want).await
}

impl PendingStop {
    pub fn new(mark: JobMark, then: CuttingError, why: impl Into<String>) -> Self {
        Self {
            mark,
            then: Box::new(then),
            why: why.into(),
        }
    }

    /// Try the stop again, with a growing pause, until it is confirmed — then hand back how
    /// the work really ended.
    ///
    /// **Does not return until then.** The task stays running, its cancellation already
    /// raised, for as long as this takes: the engine writes `Cancelled` (or `Failed`) only
    /// once the work returns, and the work returns only from here — `TaskEngine::cancel`'s
    /// own rule, "the state is written only when the work has really stopped — the process
    /// tree included". No new task state is needed for it. Its place in the lane stays
    /// taken meanwhile (see the T605 report for what that costs and the alternatives).
    ///
    /// `attempt` is one try — typically: connect afresh, stop, close. It is the caller's to
    /// make because that is where the secrets and the profile are, and where the gate is
    /// passed — with the same intent the build itself used (`Intent::Change`): stopping a
    /// process on the server is an action on the server, not a read (the lesson of T601). A
    /// refusal or a failure to connect is one more unconfirmed attempt, not an end.
    pub async fn confirm<A, Fut>(self, mut attempt: A) -> CuttingError
    where
        A: FnMut(JobMark) -> Fut,
        Fut: std::future::Future<Output = std::result::Result<Stopped, String>>,
    {
        let mark = self.mark.clone();
        super::marked::retry_until_confirmed("cutting", self.mark.as_str(), self.why, || {
            attempt(mark.clone())
        })
        .await;
        *self.then
    }
}

impl Cutting<'_> {
    fn script_path(&self) -> String {
        format!("/tmp/vrcast-hls-{}.sh", self.base)
    }

    fn log_path(&self) -> String {
        format!("/tmp/vrcast-hls-{}.log", self.base)
    }

    /// Where the wrapper records its process group (T605).
    pub fn pgid_path(&self) -> String {
        format!("/tmp/vrcast-hls-{}.pgid", self.base)
    }

    /// Put the script on the server and start it, detached, in a process group of its own
    /// — and check that it really is (T605).
    ///
    /// `setsid` and `nohup` together: the first takes it out of our session so that the
    /// session ending does not reach it, the second detaches it from the terminal. One
    /// without the other has let work die on this project before.
    ///
    /// **The group is checked, not assumed.** The wrapper records its own pid, group and
    /// session from `/proc/self/stat` as its very first act (the launcher's `$!` would not
    /// do: util-linux `setsid` forks when it is itself a group leader), and the start only
    /// counts once that record says: this start's mark, pid == group == session, and a
    /// group other than the launching shell's. Anything else — no record, a stranger's, a
    /// wrapper without a group of its own — and whatever did start is stopped by its mark
    /// (which does not depend on the group), and the start is an error. A cutting whose
    /// group is not known cannot be stopped as a whole, and is never carried on with.
    ///
    /// **A live cutting of the same directory is refused, not stopped.** It may be one this
    /// application left behind after being closed, in which case it is finishing work that
    /// a new start would only redo (the script skips variants already cut whole) and it
    /// will end on its own; or it may be another instance's, on another machine, which is
    /// not ours to kill. Refusing costs a person a retry later; stopping could cost somebody
    /// else their build. Two cuttings into one directory at once is not an option at all.
    pub async fn start(&self) -> CuttingResult<Started> {
        guard_base(self.base)?;

        // Asked before the script is written, not only by the launcher after: the script
        // lives at a path of its own per directory, and writing over the file a live bash
        // is still reading from would change the rest of that cutting's instructions under
        // it. The launcher asks again, for the moment between the two.
        if anything_alive(self.conn, &format!("{}:", self.base)).await? {
            return Err(CuttingError::AlreadyRunning {
                base: self.base.to_owned(),
            });
        }

        let script = self.script_path();
        write_file(self.conn, &script, &hls_package::script_text()).await?;

        let mark = JobMark::for_base(self.base);
        let mut args = vec![
            self.base.to_owned(),
            mark.as_str().to_owned(),
            script,
            self.log_path(),
            self.pgid_path(),
            self.video_dir.to_owned(),
            self.owner.to_owned(),
            self.base.to_owned(),
        ];
        for variant in self.variants {
            args.push(format!("{}={}", variant.sub, variant.file));
        }

        let launched = self
            .conn
            .exec_with_timeout(
                &bash_with_args(&hls_package::launch_script(), &args),
                START_CEILING,
            )
            .await;
        let out = match launched {
            Ok(out) => out,
            // Whether it started is not known: the command may have run and only its answer
            // been lost. So it is stopped by its mark, which finds it if it is there.
            Err(e) => return Err(self.stop_after_failure(&mark, e.into()).await),
        };

        let said = out.stdout.trim();
        if said.lines().any(|l| l.trim() == "VRCAST_HLS_UNREADABLE") {
            // The launcher checks this before starting anything.
            return Err(SshError::Exec(String::from(
                "could not read the server's process table, so the cutting was not started",
            ))
            .into());
        }
        let report = hls_package::read_launch(said);
        if report == hls_package::LaunchReport::Busy {
            return Err(CuttingError::AlreadyRunning {
                base: self.base.to_owned(),
            });
        }
        match hls_package::check_launch(&report, mark.as_str()) {
            Ok(group) => Ok(Started { mark, group }),
            Err(problem) => {
                let failure = SshError::Exec(format!(
                    "could not start the cutting in a process group of its own: {problem} \
                     (the server said: {said}; stderr: {})",
                    out.stderr.trim()
                ));
                Err(self.stop_after_failure(&mark, failure.into()).await)
            }
        }
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

    /// Whether any process of a cutting of this directory is still alive (T605).
    ///
    /// **About the group, not about the wrapper.** This used to ask `pgrep -f` for the
    /// wrapper script's command line — and a wrapper that had died on its own while its
    /// `ffmpeg` went on writing read as "not running": `run` reported the build failed, the
    /// failure released the directory to `media_delete`/`media_rename`, and `ffmpeg` carried
    /// on writing into it. Now it asks for the mark every process of the cutting carries in
    /// its environment, `ffmpeg` included, and counts only live ones (a zombie writes
    /// nothing). Being about processes rather than names, it also has no need for the
    /// self-excluding `[v]rcast-…` pattern the `pgrep -f` version did: the question is not
    /// on anybody's command line.
    ///
    /// An `Err` when `/proc` could not be read: "could not look" is never "nothing there".
    pub async fn still_running(&self) -> Result<bool> {
        anything_alive(self.conn, &format!("{}:", self.base)).await
    }

    /// Stop this start's processes and confirm it, through this cutting's own connection.
    pub async fn stop(&self, started: &Started) -> std::result::Result<Stopped, StopProblem> {
        stop_confirmed(self.conn, &started.mark).await
    }

    /// The work failed or was cancelled: confirm the stop, then say how it ended — or, if
    /// the stop could not be confirmed, say that instead (T605).
    ///
    /// Every way out of [`Self::run`] after the launch goes through here, failures as well
    /// as cancellations: a failed build releases the media's directory exactly as a
    /// cancelled one does, so a failure reported while `ffmpeg` is still writing into it is
    /// the same defect under another name.
    async fn stop_after_failure(&self, mark: &JobMark, then: CuttingError) -> CuttingError {
        match stop_confirmed(self.conn, mark).await {
            Ok(_) => then,
            Err(problem) => CuttingError::StopUnconfirmed(PendingStop::new(
                mark.clone(),
                then,
                problem.to_string(),
            )),
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
    /// Before T597, a stop pressed during the cutting did nothing but wait out the sleep, and
    /// the detached server-side process went on regardless.
    ///
    /// ⚠ **T605 — `Cancelled`, and every failure, only once the stop is confirmed.** T597/T598
    /// signalled the group once, with TERM, and returned `Cancelled` straight after, whatever
    /// the server did with the signal. Now any way out other than success goes through
    /// [`Self::stop_after_failure`]; when the stop cannot be confirmed on this connection the
    /// answer is [`CuttingError::StopUnconfirmed`], which the caller settles with
    /// [`PendingStop::confirm`] before reporting anything.
    pub async fn run<F>(
        &self,
        ctx: &TaskContext,
        mut on_progress: F,
    ) -> CuttingResult<Vec<CutFacts>>
    where
        F: FnMut(&Progress),
    {
        let started = self.start().await?;
        match self.watch(ctx, &started, &mut on_progress).await {
            Ok(facts) => Ok(facts),
            Err(then) => Err(self.stop_after_failure(&started.mark, then).await),
        }
    }

    async fn watch<F>(
        &self,
        ctx: &TaskContext,
        started: &Started,
        on_progress: &mut F,
    ) -> CuttingResult<Vec<CutFacts>>
    where
        F: FnMut(&Progress),
    {
        let mut last_seen = 0usize;
        let cancel_token = ctx.cancel_token();
        loop {
            // Raced rather than checked only between iterations: waiting out the whole of
            // `ASK_EVERY` before noticing a cancellation would answer "stop" several seconds
            // late on every single poll, and this is the one thing T597 exists to shorten.
            tokio::select! {
                _ = tokio::time::sleep(ASK_EVERY) => {}
                _ = cancel_token.cancelled() => return Err(CuttingError::Cancelled),
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

            // No marker and nothing of this start alive — `ffmpeg` included, not only the
            // wrapper (T605): it was killed, ran out of room, or the machine was restarted
            // under it. Whatever it was, it is not going to finish on its own, and waiting
            // for a marker that will never come is the worst way to find out. A check that
            // could not be made is not an answer either way, and is asked again.
            if !anything_alive(self.conn, started.mark.as_str())
                .await
                .unwrap_or(true)
            {
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
    /// The segments stay; the script, its log and its group record do not. They live in
    /// `/tmp` and would go on their own eventually, but "eventually" on a server that is
    /// never restarted is a long time. Called once the cutting has ended.
    pub async fn tidy_up(&self) -> Result<()> {
        let pgid = self.pgid_path();
        self.conn
            .exec(&format!(
                "rm -f {} {} {} {}",
                super::shell_quote(&self.script_path()),
                super::shell_quote(&self.log_path()),
                super::shell_quote(&pgid),
                super::shell_quote(&format!("{pgid}.part")),
            ))
            .await?;
        Ok(())
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
