//! T605, generalised by T609 — asking a server to stop what carries a mark, and trying again
//! until it confirms.
//!
//! The scripts and the reading of their answers are [`crate::domain::marked`]'s; here is only
//! how they are run and retried. Two kinds of work stand on it: the cutting of a ladder
//! (`server::hls_package`) and every remote command of a deployment (`tasks::deploy`).

use std::time::Duration;

use crate::domain::marked::{self, StopReport, Stopped};
use crate::ssh::{Connection, Result, SshError};

/// How long the work is given to end on TERM before it is killed, in 100 ms ticks.
///
/// Measured in the test container (T605): `ffmpeg` in the middle of a `-c copy` remux, and
/// the wrapper around it, are gone ~0.1 s after TERM; apt and dpkg the same (T609 phase A,
/// 105–106 ms). Five seconds is fifty times that — room for a loaded server — and short
/// enough that a person pressing "stop" is not kept waiting long by something that ignores
/// TERM altogether.
pub const TERM_TICKS: u32 = 50;

/// How long the work is given to disappear after KILL, in 100 ms ticks.
///
/// KILL cannot be caught; what is waited for is the kernel tearing the processes down. On
/// the test container that is the very next scan. Five seconds leaves room for a process in
/// uninterruptible sleep on a slow disk, which KILL reaches only once the disk answers.
pub const KILL_TICKS: u32 = 50;

/// The ceiling on one stop command beyond its own waiting (T595's `exec_with_timeout`).
///
/// The TERM and KILL waits together are at most ten seconds, plus the scans between them.
/// A ceiling well above that — but far below `exec`'s own 600 s — means a connection that
/// died silently is found out in a minute rather than ten, and the next attempt goes
/// through a fresh one.
const STOP_CEILING: Duration = Duration::from_secs(60);

/// The ceiling on asking whether anything marked is alive — one scan of `/proc`.
const PROBE_CEILING: Duration = Duration::from_secs(30);

/// The longest one stop command waits for the work to end on its own (T609), in seconds.
///
/// A deployment's command that is still legitimately running when the stop is asked through
/// a fresh connection is waited for, not killed — but not inside one endless command: a
/// connection that dies during the wait is found out within this plus [`STOP_CEILING`], and
/// the next attempt carries on waiting through a fresh one.
pub const MAX_GRACE_S: u64 = 50;

/// The first pause before an unconfirmed stop is tried again, and the ceiling the pause
/// doubles up to.
///
/// Not measured, and there is nothing to measure: it is how long a person is kept waiting
/// once the server is back, against how hard a server that is not back is knocked on. Two
/// seconds answers a blip at once; a minute is how often an unreachable server is tried for
/// as long as it stays unreachable — no tight loop, and no giving up either, because giving
/// up would mean writing an end nobody has confirmed.
pub const RETRY_FIRST: Duration = Duration::from_secs(2);
pub const RETRY_CEILING: Duration = Duration::from_secs(60);

/// How a stop treats work that is still alive when it is asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Patience {
    /// Seconds to wait for the marked processes to end on their own before anything else.
    pub grace_s: u64,
    /// After the wait: signal what is left (TERM → KILL), or leave it alone and answer
    /// "still running" ([`StopProblem::StillRunning`]).
    pub then_signal: bool,
}

impl Patience {
    /// Signal at once — the cutting's stop (T605), and a deployment's once its command has
    /// had all the time it may have.
    pub const NONE: Self = Self {
        grace_s: 0,
        then_signal: true,
    };

    /// For work that may still legitimately run for `left`: wait for it, at most
    /// [`MAX_GRACE_S`] in this one command; signal only if `left` runs out inside it.
    pub fn for_remaining(left: Duration) -> Self {
        let left_s = left.as_secs() + u64::from(left.subsec_nanos() > 0);
        if left_s == 0 {
            Self::NONE
        } else if left_s <= MAX_GRACE_S {
            Self {
                grace_s: left_s,
                then_signal: true,
            }
        } else {
            Self {
                grace_s: MAX_GRACE_S,
                then_signal: false,
            }
        }
    }
}

/// Why a stop was not confirmed.
#[derive(Debug, thiserror::Error)]
pub enum StopProblem {
    /// The server could not be asked at all.
    #[error(transparent)]
    Ssh(#[from] SshError),
    /// Asked, and something of ours was still alive after KILL.
    #[error("still alive after KILL: {0}")]
    StillAlive(String),
    /// Asked, and the work is still running on its own time — deliberately not signalled
    /// (T609: a deployment's command is waited for, not torn in half).
    #[error("still running, waited for rather than stopped: {0}")]
    StillRunning(String),
    /// Asked, and the answer was not one — `/proc` could not be read, or nothing came back
    /// that the stop script says. Never read as "gone": silence is not a confirmation.
    #[error("no readable answer: {0}")]
    Unreadable(String),
}

/// Run one of the domain module's scripts with arguments, through `bash -c`.
///
/// `bash` by name rather than whatever the login shell is: the scripts use arrays and
/// `mapfile -d`, and a server whose login shell is `sh` would otherwise read them as
/// nonsense and answer with silence — which, for a question like "is it still running?",
/// is the one answer that must never be taken at its word. The sentinel in front is what
/// the scan checks itself against (see `SCAN` in the domain module). `name` is only `$0`.
pub fn bash_with_args(name: &str, script: &str, args: &[String]) -> String {
    let mut cmd = format!(
        "{} bash -c {} {name}",
        marked::SELFCHECK,
        super::shell_quote(script)
    );
    for arg in args {
        cmd.push(' ');
        cmd.push_str(&super::shell_quote(arg));
    }
    cmd
}

/// Stop every process whose environment carries `var=<mark…>`, and confirm it.
///
/// One command: (optionally) wait for the work to end on its own → TERM to the group(s) and
/// to each marked process → wait, asking `/proc` every 100 ms → KILL whoever is left → wait
/// → answer. A zombie counts as gone: it executes nothing and writes nothing.
///
/// The outcomes are kept apart on purpose — `Ok` is a confirmation (including "there was
/// nothing to stop"), [`StopProblem::StillAlive`], [`StopProblem::StillRunning`] and
/// [`StopProblem::Unreadable`] are "asked, and not confirmed", [`StopProblem::Ssh`] is
/// "could not ask". None of the last four may be reported upward as a stop.
pub async fn stop_confirmed(
    conn: &Connection,
    var: &str,
    mark: &str,
    patience: Patience,
) -> std::result::Result<Stopped, StopProblem> {
    let out = conn
        .exec_with_timeout(
            &bash_with_args(
                "vrcast-stop",
                &marked::stop_script(var),
                &[
                    mark.to_owned(),
                    TERM_TICKS.to_string(),
                    KILL_TICKS.to_string(),
                    patience.grace_s.to_string(),
                    String::from(if patience.then_signal {
                        "signal"
                    } else {
                        "wait"
                    }),
                ],
            ),
            STOP_CEILING + Duration::from_secs(patience.grace_s),
        )
        .await?;
    match marked::read_stop(&out.stdout) {
        StopReport::Confirmed { how, elapsed_ms } => {
            tracing::info!(
                var,
                mark,
                ?how,
                ?elapsed_ms,
                "the marked processes are gone"
            );
            Ok(how)
        }
        StopReport::StillAlive(who) => Err(StopProblem::StillAlive(who)),
        StopReport::StillRunning(who) => Err(StopProblem::StillRunning(who)),
        StopReport::Unreadable(said) => Err(StopProblem::Unreadable(format!(
            "{said} (exit {:?}, stderr: {})",
            out.exit_code,
            out.stderr.trim()
        ))),
    }
}

/// Whether any live process carries `var=<want…>` (a prefix).
///
/// An `Err` when `/proc` could not be read: "could not look" is never "nothing there".
pub async fn anything_alive(conn: &Connection, var: &str, want: &str) -> Result<bool> {
    let out = conn
        .exec_with_timeout(
            &bash_with_args(
                "vrcast-probe",
                &marked::probe_script(var),
                &[want.to_owned()],
            ),
            PROBE_CEILING,
        )
        .await?;
    match out.trimmed().lines().last().map(str::trim) {
        Some("VRCAST_RUNNING yes") => Ok(true),
        Some("VRCAST_RUNNING no") => Ok(false),
        _ => Err(SshError::Exec(format!(
            "could not tell whether anything marked {var} is running: {} {}",
            out.stdout.trim(),
            out.stderr.trim()
        ))),
    }
}

/// Try a stop again, with a growing pause, until one attempt confirms it.
///
/// **Does not return until then**, and that is the point: the caller writes nothing final —
/// no `Cancelled`, no `Failed` — before this returns, so the task stays running (and holds
/// whatever it holds: a media's directory, a server's deployment slot) for as long as it
/// takes. A refusal or a failure to connect is one more unconfirmed attempt, not an end.
///
/// `attempt` is one try — typically: connect afresh, stop, close. It is the caller's to make
/// because that is where the secrets and the profile are, and where the gate is passed.
pub async fn retry_until_confirmed<A, Fut>(
    what: &str,
    mark: &str,
    why: String,
    mut attempt: A,
) -> Stopped
where
    A: FnMut() -> Fut,
    Fut: std::future::Future<Output = std::result::Result<Stopped, String>>,
{
    let mut pause = RETRY_FIRST;
    let mut tries = 1u32;
    let mut why = why;
    loop {
        tracing::warn!(
            what,
            mark,
            tries,
            why = %why,
            retry_in_s = pause.as_secs(),
            "the stop on the server is not confirmed yet"
        );
        tokio::time::sleep(pause).await;
        pause = (pause * 2).min(RETRY_CEILING);
        tries += 1;
        match attempt().await {
            Ok(how) => {
                tracing::info!(
                    what,
                    mark,
                    tries,
                    ?how,
                    "the stop on the server is confirmed"
                );
                return how;
            }
            Err(e) => why = e,
        }
    }
}
