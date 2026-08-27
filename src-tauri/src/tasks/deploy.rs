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
use crate::domain::wording::DetailCode;
use crate::server::deploy::{self, Context, DeployError, Step};
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
    kind: Kind,
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
        match kind {
            Kind::Fresh => deploy::run(ctx, steps, &cancelled, &mut watch).await,
            Kind::Upgrade => upgrade::run(ctx, steps, &cancelled, &mut watch).await,
        }
    };

    outcome.map_err(|e| failed(e, &settled))
}

/// Turn a deployment's failure into what the interface branches on.
///
/// The step is named in every case (FR-123). "The deployment failed" and "the firewall step
/// failed" send a person to different places, and only one of them is somewhere to go.
fn failed(e: DeployError, settled: &[PlannedStep]) -> AppError {
    match e {
        DeployError::Cancelled => AppError::new(ErrorCode::TaskCancelled),
        DeployError::Ssh(inner) => inner.into(),
        DeployError::NotTaken { id } => AppError::new(ErrorCode::DeployStepFailed).with_cause(
            format!("{id:?}: it was applied and the check still says it was not"),
        ),
        DeployError::Step { id, detail, advice } => {
            // The step's own advice, when it has any. The domain check is the one that does:
            // which record to create, with what value, and where it leads now.
            let code = code_for(id);
            let error = AppError::new(code).with_cause(format!("{id:?}: {detail}"));
            match advice {
                Some(detail) => error.with_detail(detail),
                None => error,
            }
        }
    }
    .with_cause(format!("after {} steps", stopped_after(settled)))
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
