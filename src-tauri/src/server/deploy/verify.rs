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

/// How long one request may take.
const ONE_TRY: Duration = Duration::from_secs(10);

/// How long to keep asking altogether.
///
/// **Asked again rather than asked once** — found on the real stand (2026-08-27). This
/// step runs seconds after the serving started for the very first time, and at that
/// moment the certificate is being obtained: the authority is asked, a challenge is
/// answered, three chains are downloaded. A single request lands in the middle of that
/// and comes back with nothing, and the deployment is declared failed one step from its
/// end — on a server that was about to work.
///
/// Two minutes: the certificate on that run took about forty seconds from the service
/// starting, and a slow authority is a thing that happens.
const PATIENCE: Duration = Duration::from_secs(120);

/// The pause between attempts, doubling.
const FIRST_PAUSE: Duration = Duration::from_secs(2);
const LONGEST_PAUSE: Duration = Duration::from_secs(15);

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

/// Ask the domain once and read what comes back.
async fn answers_now(domain: &str) -> bool {
    let Ok(client) = reqwest::Client::builder().timeout(ONE_TRY).build() else {
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

/// Ask, and go on asking while there is time.
///
/// The waiting is not politeness: right after a first deployment the serving is up and the
/// certificate is not yet, and those few tens of seconds are a state, not a failure.
async fn answers(domain: &str) -> bool {
    let deadline = std::time::Instant::now() + PATIENCE;
    let mut pause = FIRST_PAUSE;
    loop {
        if answers_now(domain).await {
            return true;
        }
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return false;
        }
        tokio::time::sleep(pause.min(left)).await;
        pause = (pause * 2).min(LONGEST_PAUSE);
    }
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
        // Nothing to apply — the serving either answers or it does not. The check above
        // has already waited out its patience, so this is one last look rather than a
        // second wait: doubling the wait would only double how long a person stares at a
        // deployment that is not going to finish.
        if answers_now(ctx.domain).await {
            return Ok(());
        }
        Err(DeployError::Step {
            id: StepId::Verify,
            detail: format!("https://{} did not answer as the serving", ctx.domain),
            advice: None,
        })
    })
}
