//! T286 — deploying and upgrading as tasks of the engine.
//!
//! Both are long — packages come down over somebody's line, a certificate is obtained, a
//! service settles — so neither may hold the interface still (FR-080). And both report step by
//! step, because "deploying…" for four minutes tells a person nothing about whether to wait or
//! to go and look at their DNS (FR-123).
//!
//! **The kinds are already in the database.** `deploy` and `upgrade_server` were listed in the
//! task table's constraint from the start, unlike `measure_quality` — which was added to the
//! code and not to the list, so the very first attempt to measure anything failed at the
//! database after the interface had already said the task was starting. Worth saying out loud:
//! that is the shape of mistake this file could repeat and does not.

use crate::commands::error::{AppError, ErrorCode, Result};
use crate::domain::deploy_steps::{PlannedStep, Status, StepId};
use crate::domain::wording::{Detail, DetailCode};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use futures::future::BoxFuture;

use crate::domain::marked::Stopped;
use crate::server::deploy::{Context, DeployError, Step, RUN_VAR};
use crate::server::marked::Patience;
use crate::server::upgrade;
use crate::tasks::engine::TaskContext;

/// What kind of run this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A bare machine.
    Fresh,
    /// A server already ours, being brought up to date — copies are taken first.
    Upgrade,
}

/// One more try at confirming a cancelled or failed run's stop, through a **fresh**
/// connection (T609, T615).
///
/// Handed the run's mark and how patient to be; answers how the stop went, or why it could
/// not be confirmed. The caller's to make because that is where the secrets, the profile and
/// the gate are — production passes the gate with the same intent the run itself used
/// (`Intent::Setup`): stopping a process on the server is an action on the server.
pub type StopAgain<'s> = dyn Fn(String, Patience) -> BoxFuture<'s, std::result::Result<Stopped, String>>
    + Send
    + Sync
    + 's;

/// Carry a deployment out, reporting as it goes.
///
/// `report` is handed each step as its outcome settles, so a screen is never a step behind.
///
/// ⚠ **T609 — a cancellation waits for the command on the server, then confirms nothing of
/// the run is left, and only then answers `Cancelled`.** T595 raced the run against the
/// cancel token and dropped it the moment the token fired: `Cancelled` came back in 2.4 s
/// while `apt-get` and `dpkg` went on for another 17.8 s on the server holding dpkg's lock,
/// and a person pressing "deploy" again at once got `Could not get lock` (measured, T609
/// phase A). Killing them instead was measured too, and is worse: `dpkg` killed halfway
/// leaves the package database interrupted and every later install refused.
///
/// So now, when the token fires:
///
/// 1. the run is asked to stop ([`crate::server::deploy::RunMark::ask_to_stop`]): no further
///    command of it is sent, and the one in flight is **not** interrupted;
/// 2. the stage becomes `STAGE_STOPPING_AFTER_STEP`; the task stays `running`, so
///    `running_deploy_for` goes on refusing a second run of the same server;
/// 3. the run is awaited to its natural end — at most one command's `EXEC_CEILING`;
/// 4. the server is asked to confirm that no process carrying the run's mark is alive —
///    TERM → KILL only for what is still marked after the command itself ended (a descendant
///    that slipped away, say). A command whose answer was lost with its connection is given
///    the rest of its `EXEC_CEILING` first, and only then signalled;
/// 5. unconfirmed — [`confirm_stopped`].
///
/// ⚠ **T615 — a run that fails answers `Failed` only after the same confirmation.** A broken
/// connection in the middle of a command used to be written down as `Failed` at once, while
/// `apt-get`/`dpkg` of that command could still be running on the server — and a person
/// pressing "deploy" again met them at dpkg's lock. Now any failure goes through steps 4–5
/// above before it is handed back: the command whose answer was lost with its connection is
/// waited for until its `EXEC_CEILING`, anything of the run still alive after that is
/// stopped, and only then is the **step's own error** (not the stop's) returned. A step that
/// failed on a healthy connection costs one scan of `/proc` more.
///
/// A run that ends well is reported as before: every command of it has said how it ended.
pub async fn run<'a>(
    ctx: &Context<'a>,
    steps: &[Step<Context<'a>>],
    task: &TaskContext,
    report: &mut (dyn FnMut(&[PlannedStep]) + Send),
    stop_again: &StopAgain<'_>,
) -> Result<Vec<PlannedStep>> {
    let total = steps.len().max(1) as f64;
    let mut settled: Vec<PlannedStep> = Vec::new();
    // How many steps have settled, readable while the run holds `settled` itself.
    let done = AtomicUsize::new(0);

    let cancelled = || task.is_cancelled();
    let (outcome, stop_asked) = {
        let mut watch = |step: &PlannedStep| {
            settled.push(step.clone());
            done.store(settled.len(), Ordering::SeqCst);
            // Not overwritten once a stop is asked: "stopping" is the one thing to say then.
            if !ctx.run.is_stopping() {
                task.report(settled.len() as f64 / total, DetailCode::StageDeploying);
            }
            report(&settled);
        };
        // ⚠ **Both kinds copy aside first** (T513, FR-095). Only the upgrade did, and the
        // requirement is not about upgrades: it asks for a copy of the server's settings files
        // **that are changed**, and for the previous state to be recoverable — any file a
        // deployment changes. A first run edits `/etc/default/ufw` in place, appends to
        // `/etc/fstab` and writes `/etc/fail2ban/jail.local` whole over whatever was there,
        // and none of it was recoverable, because the only backup there is was on the other
        // branch. FR-133 is the narrower one and was met; FR-095 was met nowhere.
        //
        // On a bare server this copies nothing and costs a directory: `back_up` skips a file
        // that is not there. That is the right outcome — there was nothing to lose — and it
        // still leaves a `latest` to roll back to, so the answer to "put it back" stops being
        // an internal error about a missing directory.
        let run_fut = async {
            // T615: what an earlier run interrupted between writing a file and moving it into
            // place left beside the real one — see `leftovers_script` for why here.
            let tidied = ctx.ran(&crate::server::deploy::leftovers_script()).await?;
            if !tidied.trim().is_empty() {
                tracing::info!(files = %tidied.trim(), "removed what an interrupted run left half-written");
            }
            upgrade::run(ctx, steps, &cancelled, &mut watch).await
        };
        tokio::pin!(run_fut);
        let cancel_token = task.cancel_token();

        // Raced against the token only to learn **when** the stop was asked — never to drop
        // the run. Dropping it is what T595 did, and what left `dpkg` running behind a
        // `Cancelled` (see above).
        tokio::select! {
            result = &mut run_fut => (result, false),
            _ = cancel_token.cancelled() => {
                ctx.run.ask_to_stop();
                task.report_important(
                    done.load(Ordering::SeqCst) as f64 / total,
                    DetailCode::StageStoppingAfterStep,
                );
                (run_fut.await, true)
            }
        }
    };

    let cancelled = stop_asked || task.is_cancelled();
    let progress = settled.len() as f64 / total;
    let mark = ctx.run.as_str().to_owned();
    let first = async {
        // Nothing further of this run is to be sent, whatever it ended with.
        ctx.run.ask_to_stop();
        crate::server::marked::stop_confirmed(
            ctx.conn,
            RUN_VAR,
            &mark,
            patience_until(ctx.run.may_run_until()),
        )
        .await
        .map_err(|problem| problem.to_string())
    };
    settle(
        task,
        progress,
        &mark,
        outcome,
        cancelled,
        first,
        &|| ctx.run.may_run_until(),
        stop_again,
    )
    .await
    .map_err(|e| failed(e, &settled))
}

/// How a finished run is handed back — the decision T609 and T615 rest on, apart from the
/// connection so it can be checked without a server (`tests/unit/deploy_stop.rs`).
///
/// - Ended well, not cancelled: handed back at once. `first` is not asked.
/// - Cancelled — whatever the run itself ended with (`Cancelled` from the next command it did
///   not send, success because the last step had just finished, a broken connection):
///   `DeployError::Cancelled`, once the stop is confirmed.
/// - Failed (T615): **the run's own error, untouched**, once the stop is confirmed — never
///   the stop's error, which says nothing about why the deployment failed.
///
/// "Confirmed" is [`confirm_stopped`]: `first` (through the run's own connection), then
/// `stop_again` through fresh ones for as long as it takes. Until this returns, the task is
/// `running` and `running_deploy_for` holds the server.
#[allow(clippy::too_many_arguments)]
pub async fn settle<T>(
    task: &TaskContext,
    progress: f64,
    mark: &str,
    outcome: std::result::Result<T, DeployError>,
    cancelled: bool,
    first: impl std::future::Future<Output = std::result::Result<Stopped, String>>,
    may_run_until: &(dyn Fn() -> Option<Instant> + Sync),
    stop_again: &StopAgain<'_>,
) -> std::result::Result<T, DeployError> {
    let outcome = match outcome {
        // A future does nothing until awaited: `first` is never sent to the server here.
        Ok(done) if !cancelled => return Ok(done),
        other => other,
    };
    let first = first.await;
    let how = confirm_stopped(task, progress, mark, first, may_run_until, stop_again).await;
    match outcome {
        _ if cancelled => Err(DeployError::Cancelled),
        Err(e) => {
            tracing::info!(mark, ?how, error = %e, "the failed run's processes are gone");
            Err(e)
        }
        // Not reachable — an unasked success returned above — but written out rather than
        // `unreachable!`: a success is a success.
        Ok(done) => Ok(done),
    }
}

/// How patient a stop may be with a command that may still be running until `until`.
fn patience_until(until: Option<Instant>) -> Patience {
    match until {
        None => Patience::NONE,
        Some(until) => Patience::for_remaining(until.saturating_duration_since(Instant::now())),
    }
}

/// Settle a cancelled or failed run's stop: return only once the server has confirmed that
/// nothing carrying the run's mark is alive (T609, T615, by the pattern of T605's `settle_stop`).
///
/// `first` is how the first attempt went (through the run's own connection). Confirmed —
/// return at once. Not — the stage becomes `STAGE_STOP_UNCONFIRMED` and the stop is tried
/// again through `stop_again`, a fresh connection each time, pausing 2 s doubling to 60 s,
/// **for as long as it takes**: the engine writes `Cancelled` or `Failed` only once the work
/// returns, and this is the only way out of it. Meanwhile the task is `running`, and
/// `running_deploy_for` refuses a second deployment or upgrade of the same server.
///
/// `may_run_until` says, at each attempt, until when the command last sent may still
/// legitimately be running (its `EXEC_CEILING`); until then it is waited for, not signalled.
///
/// Public so it can be checked without a server, on the real task engine
/// (`tests/unit/deploy_stop.rs`).
pub async fn confirm_stopped(
    task: &TaskContext,
    progress: f64,
    mark: &str,
    first: std::result::Result<Stopped, String>,
    may_run_until: &(dyn Fn() -> Option<Instant> + Sync),
    stop_again: &StopAgain<'_>,
) -> Stopped {
    let why = match first {
        Ok(how) => {
            tracing::info!(mark, ?how, "the stopped run's processes are gone");
            return how;
        }
        Err(why) => why,
    };
    // Said out loud while it lasts: a person who pressed "stop" — or watches a deployment
    // whose connection broke — and sees the task still running is owed the reason, not a
    // frozen bar.
    task.report_important(progress, DetailCode::StageStopUnconfirmed);
    crate::server::marked::retry_until_confirmed("deployment", mark, why, || {
        stop_again(mark.to_owned(), patience_until(may_run_until()))
    })
    .await
}

/// Turn a deployment's failure into what the interface branches on.
///
/// The step is named in every case (FR-123). "The deployment failed" and "the firewall step
/// failed" send a person to different places, and only one of them is somewhere to go.
/// Public so it can be checked without a server (constitution: logic reachable only through
/// a server counts as unchecked). The whole of FR-123 lives in this function, and until
/// 2026-09-06 nothing checked it — which is how the last line came to destroy what the arms
/// above had set.
pub fn failed(e: DeployError, settled: &[PlannedStep]) -> AppError {
    let done = stopped_after(settled);

    // Said first, so it is the first thing read: which step, and how far the deployment got.
    //
    // ⚠ **This used to be `.with_cause(format!("after {done} steps"))` on the whole `match`,
    // and `with_cause` replaces** — so the step name set inside the arms was destroyed on the
    // way out, in the one function whose doc comment promises to name it (T506, FR-123).
    // "The deployment failed" and "the firewall step failed" send a person to different
    // places, and only one of them is somewhere to go.
    //
    // A code with the step as a value, rather than English in `cause`: `deploySteps` holds all
    // fifteen names in both languages already, and the screen shows the plan from that same
    // set — so a failure names the step in the same words the plan did.
    let where_it_stopped = |id: Option<StepId>| match id {
        Some(id) => Detail::new(DetailCode::DeployStoppedAtStep)
            .with("step", format!("{id:?}"))
            .with("done", done as u64),
        None => Detail::new(DetailCode::DeployStoppedAfter).with("done", done as u64),
    };

    match e {
        DeployError::Cancelled => {
            AppError::new(ErrorCode::TaskCancelled).with_detail(where_it_stopped(None))
        }
        // The link broke rather than a step refusing: the connection's own error says what
        // happened, and no step owns the failure.
        DeployError::Ssh(inner) => AppError::from(inner).with_detail(where_it_stopped(None)),
        DeployError::NotTaken { id } => AppError::new(ErrorCode::DeployStepFailed)
            .with_detail(where_it_stopped(Some(id)))
            .with_cause("it was applied and the check still says it was not"),
        DeployError::Step { id, detail, advice } => {
            let code = code_for(id);
            let error = AppError::new(code)
                .with_detail(where_it_stopped(Some(id)))
                .with_cause(detail);
            // The step's own advice, when it has any. The domain check is the one that does:
            // which record to create, with what value, and where it leads now.
            match advice {
                Some(detail) => error.with_detail(detail),
                None => error,
            }
        }
    }
}

/// Which contract code a failing step answers with.
///
/// Most are simply "a step failed", and the step is named. Three have codes of their own
/// because the interface does something different with them: a domain that is not attached
/// opens the domain screen, a swap that would not be made says how much room was wanted, and
/// a serving that will not answer is a different afternoon from a package that would not
/// install.
fn code_for(id: StepId) -> ErrorCode {
    match id {
        StepId::DnsCheck => ErrorCode::DomainNotPointed,
        StepId::Swap => ErrorCode::SwapFailed,
        StepId::Verify => ErrorCode::DomainNotServing,
        _ => ErrorCode::DeployStepFailed,
    }
}

fn stopped_after(settled: &[PlannedStep]) -> usize {
    settled
        .iter()
        .filter(|s| !matches!(s.status, Status::Failed { .. }))
        .count()
}
