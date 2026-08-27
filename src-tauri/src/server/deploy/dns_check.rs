//! The first step: does the domain point here (FR-137)?
//!
//! **First, and it changes nothing.** A record that leads somewhere else costs one lookup to
//! find here; found at the verifying step it costs a half-configured server, and the person
//! is left to work out which half. That is the whole reason this comes before everything.
//!
//! Its "apply" cannot apply anything — nobody but the domain's owner can create a record —
//! so it fails, and the failure carries what to go and do: the record's type, its exact name
//! and the exact value (FR-140).

use futures::future::BoxFuture;

use crate::domain::deploy_steps::{Change, Checked, StepId};
use crate::domain::dns_verdict::{self, Records, Verdict};
use crate::net::dns;

use super::{Context, DeployError, Result, Step};

pub fn step<'a>() -> Step<Context<'a>> {
    Step {
        id: StepId::DnsCheck,
        changes,
        check,
        apply,
    }
}

fn changes(_: &Context<'_>) -> Vec<Change> {
    vec![Change::LooksOnly]
}

/// Ask, and judge.
async fn verdict_of(ctx: &Context<'_>) -> Verdict {
    // A lookup that could not be made at all is not the same as a domain that is not
    // attached, and must not become one: the person would be sent to edit a record that was
    // never wrong. An empty answer is the honest "nothing points here".
    let records = dns::look_up(ctx.domain, dns::DEFAULT_PATIENCE)
        .await
        .unwrap_or_else(|_| Records::default());
    dns_verdict::judge(&records, &ctx.server, ctx.ipv6)
}

fn check<'x, 'a>(ctx: &'x Context<'a>) -> BoxFuture<'x, Result<Checked>> {
    Box::pin(async move {
        Ok(if verdict_of(ctx).await.may_begin() {
            Checked::Applied
        } else {
            Checked::NotApplied
        })
    })
}

fn apply<'x, 'a>(ctx: &'x Context<'a>) -> BoxFuture<'x, Result<()>> {
    Box::pin(async move {
        let verdict = verdict_of(ctx).await;
        if verdict.may_begin() {
            return Ok(());
        }
        Err(DeployError::Step {
            id: StepId::DnsCheck,
            detail: format!("{verdict:?}"),
            advice: verdict.what_to_do(ctx.domain, &ctx.server),
        })
    })
}
