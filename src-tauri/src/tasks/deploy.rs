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
use crate::server::deploy::{Context, DeployError, Step};
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

/// Carry a deployment out, reporting as it goes.
///
/// `report` is handed each step as its outcome settles, so a screen is never a step behind.
pub async fn run<'a>(
    ctx: &Context<'a>,
    steps: &[Step<Context<'a>>],
    task: &TaskContext,
    report: &mut (dyn FnMut(&[PlannedStep]) + Send),
) -> Result<Vec<PlannedStep>> {
    let total = steps.len().max(1) as f64;
    let mut settled: Vec<PlannedStep> = Vec::new();

    let cancelled = || task.is_cancelled();
    let outcome = {
        let mut watch = |step: &PlannedStep| {
            settled.push(step.clone());
            task.report(settled.len() as f64 / total, DetailCode::StageDeploying);
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
        let run_fut = upgrade::run(ctx, steps, &cancelled, &mut watch);
        let cancel_token = task.cancel_token();

        // ⚠ **T595 — cancellation raced against the run itself, not only checked between
        // steps.** `upgrade::run` (`server/deploy/mod.rs`) only calls `cancelled()` between
        // steps: a cancel arriving while `(step.apply)(ctx).await` is already in flight is
        // not seen until that await itself returns — which, on a step stuck behind a
        // `Connection::exec` that has not hit its own `EXEC_CEILING` yet, could be minutes.
        // This is the one production call site with a live `TaskContext`
        // (`tasks/deploy.rs::run`, unlike the test fixtures that call `server::deploy::run`
        // directly with `cancelled = || false`), so it is the one place this fix can land
        // without touching `Step`'s signature — see the doc comment on `upgrade::run` for
        // why that wider rework was deliberately not done here.
        //
        // **This is best-effort, not a hard guarantee.** When `task.cancel_token()` fires
        // first, `run_fut` is dropped mid-flight — inside it, `Connection::exec`'s `Channel`
        // is dropped along with it, which usually (not always: the server decides whether to
        // signal the child at all) tears down the SSH channel the stuck command was running
        // on. That is the same best-effort standing `upload::cleanup` already accepts for a
        // remote process, and it is NOT the constitution's principle III guarantee — that
        // one is about the LOCAL process tree via `kill_tree()`, not a command running on
        // somebody else's machine. What this buys is real: the task itself answers
        // `Cancelled` at once instead of waiting out `EXEC_CEILING`, so the "cancel" button
        // stops lying about working. `opened.conn.close()` in `commands/deploy.rs` runs
        // right after this returns, outside `tasks::deploy::run` entirely, so the connection
        // itself is always closed regardless of which side of this race won.
        tokio::select! {
            result = run_fut => result,
            _ = cancel_token.cancelled() => Err(DeployError::Cancelled),
        }
    };

    outcome.map_err(|e| failed(e, &settled))
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
