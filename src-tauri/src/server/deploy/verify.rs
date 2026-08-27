//! T280 — second to last: does the serving actually answer (FR-125)?
//!
//! **From this machine, over the domain, and read** (R-20). An established connection is not
//! proof: at some providers the connection is accepted by a protective layer in front of the
//! server and not by the server, so a port that opens says only that something opened it.
//! Asking from inside the server is no better — from there "it works" is true of things that
//! cannot be reached from outside at all.
//!
//! It comes before the state file so that the file, which says "all of this was done here",
//! is only written about a server that serves.

use futures::future::BoxFuture;
use std::time::Duration;

use crate::domain::deploy_steps::{Change, Checked, StepId};

use super::{Context, DeployError, Result, Step};

/// How long to wait for the first answer.
///
/// Generous on purpose: this runs right after the serving was started for the first time, and
/// a certificate is being obtained while we ask.
const PATIENCE: Duration = Duration::from_secs(30);

pub fn step<'a>() -> Step<Context<'a>> {
    Step {
        id: StepId::Verify,
        changes,
        check,
        apply,
    }
}

fn changes(_: &Context<'_>) -> Vec<Change> {
    vec![Change::LooksOnly]
}

/// Ask the domain and read what comes back.
async fn answers(domain: &str) -> bool {
    let Ok(client) = reqwest::Client::builder().timeout(PATIENCE).build() else {
        return false;
    };
    let Ok(answer) = client.get(format!("https://{domain}/")).send().await else {
        return false;
    };
    if !answer.status().is_success() {
        return false;
    }
    // The body, not the status. A protective layer in front of the server answers 200 with a
    // page of its own, and a check that stopped at the status would call that a working
    // deployment.
    matches!(answer.text().await, Ok(body) if body.contains("VRCast"))
}

fn check<'x, 'a>(ctx: &'x Context<'a>) -> BoxFuture<'x, Result<Checked>> {
    Box::pin(async move {
        if ctx.machine.is_container() {
            // No domain of its own and no certificate. Said plainly rather than passed: a
            // container run that reported the serving verified would be reporting nothing.
            return Ok(ctx.not_here("serving over the domain"));
        }
        Ok(if answers(ctx.domain).await {
            Checked::Applied
        } else {
            Checked::NotApplied
        })
    })
}

fn apply<'x, 'a>(ctx: &'x Context<'a>) -> BoxFuture<'x, Result<()>> {
    Box::pin(async move {
        // Nothing to apply — the serving either answers or it does not. Asked once more
        // because a certificate can arrive in the seconds between the check and here.
        if answers(ctx.domain).await {
            return Ok(());
        }
        Err(DeployError::Step {
            id: StepId::Verify,
            detail: format!("https://{} did not answer as the serving", ctx.domain),
            advice: None,
        })
    })
}
