//! T267 — the mechanism a deployment is made of.
//!
//! Every step is a pair: **check**, which says whether the thing is already so, and
//! **apply**, which makes it so. The check looks at the server rather than at a record of
//! what we did, and that one rule is where safety on a repeat comes from (FR-124, R-12) —
//! not from care taken inside each step.
//!
//! It buys three things at once:
//!
//! - a plan that can be shown before anything changes, because running every check and
//!   applying nothing is a plan (FR-122);
//! - a repeat after a failure that skips what is done (FR-124, SC-015);
//! - a way to ask whether an already deployed server still matches the reference — the same
//!   checks, again with nothing applied.
//!
//! **A step is done when its check says so, not when its apply returned.** The engine
//! re-runs the check after applying and calls the step failed if it still says no. That is
//! not belt and braces: on the live server the hardening step was written, ran without
//! complaint, and for six months `sshd -T` said password logins were on, while twenty-two
//! thousand attempts a day went at it. An apply that returns quietly proves nothing.

use futures::future::BoxFuture;

use crate::domain::deploy_steps::{self, Change, Checked, PlannedStep, Status, StepId, ORDER};
use crate::domain::dns_verdict::{Ipv6Choice, ServerAddresses};
use crate::ssh::{Connection, SshError};

/// What every step is given.
///
/// Everything a step needs to know about *this* server and *this* person's choices. Nothing
/// here is global: two servers are deployed with two contexts, and a step that reached for a
/// setting instead of its context would work until somebody had a second server.
pub struct Context<'a> {
    pub conn: &'a Connection,
    /// The domain the serving will answer on.
    pub domain: &'a str,
    /// Where the videos live. From the profile — a server set up by hand keeps them where
    /// its owner put them (FR-004).
    pub video_dir: &'a str,
    /// What the person chose about IPv6 (FR-135).
    pub ipv6: Ipv6Choice,
    /// Where this machine is, as far as we know.
    pub server: ServerAddresses,
    /// The public half of the key to put on the server, in `authorized_keys` form.
    pub public_key: String,
}

/// Function pointers rather than a trait: the trait would have to be dyn-compatible to live
/// in a list, which means either an extra crate or boxing every future by hand. This is the
/// same thing with less ceremony, and it keeps a step's two halves visibly one pair.
///
///
/// Generic over what a step is handed. The real steps take a [`Context`] with a live
/// connection in it; the checks of the mechanism itself hand over a note-taking stand-in,
/// and so can ask what happens when a step fails, or when its apply returns without
/// having done anything — **without a server**. A mechanism that could only be checked
/// through a server would count as unchecked (constitution, limits on how work is done),
/// and this is the piece every one of the fifteen steps rests on.
pub struct Step<C> {
    pub id: StepId,
    /// What this step will change on the server, for the plan a person agrees to (FR-122).
    pub changes: fn(&C) -> Vec<Change>,
    /// Is it already so? Looks at the server, never at a record of what we did.
    pub check: for<'a> fn(&'a C) -> BoxFuture<'a, Result<Checked>>,
    /// Make it so.
    pub apply: for<'a> fn(&'a C) -> BoxFuture<'a, Result<()>>,
}

pub type Result<T> = std::result::Result<T, DeployError>;

/// What went wrong, and where.
#[derive(Debug)]
pub enum DeployError {
    /// A step failed. Names it, because "the deployment failed" and "the firewall step
    /// failed" are different things to act on (FR-123).
    Step {
        id: StepId,
        detail: String,
    },
    /// A step's apply returned without complaint and its check still says the thing is not
    /// so. Kept apart from an ordinary failure on purpose: this is the shape of the mistake
    /// that hid on the live server for six months, and a report that reads like any other
    /// failure would let it hide again.
    NotTaken {
        id: StepId,
    },
    Ssh(SshError),
    Cancelled,
}

impl From<SshError> for DeployError {
    fn from(e: SshError) -> Self {
        Self::Ssh(e)
    }
}

impl std::fmt::Display for DeployError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Step { id, detail } => write!(f, "step {id:?} failed: {detail}"),
            Self::NotTaken { id } => write!(
                f,
                "step {id:?} reported success and its check still says it was not applied"
            ),
            Self::Ssh(e) => write!(f, "{e}"),
            Self::Cancelled => f.write_str("cancelled"),
        }
    }
}

impl std::error::Error for DeployError {}

/// Run every check and change nothing (FR-122).
///
/// This is what a person is shown before they agree, and it is also how an already deployed
/// server is compared against the reference — the same checks, and the difference is only
/// what is done with the answers.
pub async fn plan<C>(ctx: &C, steps: &[Step<C>]) -> Result<Vec<PlannedStep>> {
    let mut found = Vec::new();
    for step in in_order(steps) {
        found.push((step.id, (step.check)(ctx).await?));
    }
    Ok(deploy_steps::plan(&found, |id| changes_of(steps, id, ctx)))
}

/// Carry the deployment out, reporting each step as it goes (FR-123).
///
/// `watch` is called when a step's outcome is settled — before the next one starts, so a
/// screen showing progress is never a step behind.
pub async fn run<C>(
    ctx: &C,
    steps: &[Step<C>],
    cancelled: &(dyn Fn() -> bool + Sync),
    watch: &mut (dyn FnMut(&PlannedStep) + Send),
) -> Result<Vec<PlannedStep>> {
    let mut done: Vec<PlannedStep> = Vec::new();

    for step in in_order(steps) {
        if cancelled() {
            return Err(DeployError::Cancelled);
        }

        let found = (step.check)(ctx).await?;
        let status = match found {
            // Already so. **This is the whole of safety on a repeat**: a run after a failure
            // does not undo the half that succeeded, and nothing had to be remembered between
            // the two runs for that to hold.
            Checked::Applied => Status::Applied,
            Checked::NotNeeded => Status::Skipped {
                why: deploy_steps::SkipReason::NotNeeded,
            },
            Checked::NotPossibleHere { detail } => Status::Skipped {
                why: deploy_steps::SkipReason::NotPossibleHere { detail },
            },
            Checked::NotApplied => match (step.apply)(ctx).await {
                Ok(()) => {
                    // **Asked again, on purpose.** A step is done when the server says so,
                    // not when our own code returned. The one time this rule was missing, the
                    // hardening step ran without complaint and password logins stayed on for
                    // half a year.
                    match (step.check)(ctx).await? {
                        Checked::Applied => Status::Applied,
                        Checked::NotNeeded => Status::Skipped {
                            why: deploy_steps::SkipReason::NotNeeded,
                        },
                        Checked::NotPossibleHere { detail } => Status::Skipped {
                            why: deploy_steps::SkipReason::NotPossibleHere { detail },
                        },
                        Checked::NotApplied => {
                            let planned = settled(
                                steps,
                                step.id,
                                ctx,
                                &Status::Failed {
                                    detail: String::from("applied, and the check still says no"),
                                },
                            );
                            watch(&planned);
                            done.push(planned);
                            return Err(DeployError::NotTaken { id: step.id });
                        }
                    }
                }
                Err(e) => {
                    let detail = e.to_string();
                    let planned = settled(
                        steps,
                        step.id,
                        ctx,
                        &Status::Failed {
                            detail: detail.clone(),
                        },
                    );
                    watch(&planned);
                    done.push(planned);
                    // A blocking step that failed ends the run. Going on would apply the rest
                    // to a server missing what they need, and every failure after it would say
                    // "missing" — a page of consequences with the cause five screens up.
                    if deploy_steps::stops_the_run(step.id) {
                        return Err(DeployError::Step {
                            id: step.id,
                            detail,
                        });
                    }
                    continue;
                }
            },
        };

        let planned = settled(steps, step.id, ctx, &status);
        watch(&planned);
        done.push(planned);
    }

    Ok(done)
}

/// The steps in the deployment's own order, whatever order they were handed in.
///
/// The order is not a preference in three places (R-12), and a caller building the list by
/// hand is exactly where it would be got wrong — so it is imposed here rather than trusted.
fn in_order<C>(steps: &[Step<C>]) -> Vec<&Step<C>> {
    ORDER
        .iter()
        .filter_map(|id| steps.iter().find(|s| s.id == *id))
        .collect()
}

fn changes_of<C>(steps: &[Step<C>], id: StepId, ctx: &C) -> Vec<Change> {
    steps
        .iter()
        .find(|s| s.id == id)
        .map(|s| (s.changes)(ctx))
        .unwrap_or_default()
}

fn settled<C>(steps: &[Step<C>], id: StepId, ctx: &C, status: &Status) -> PlannedStep {
    PlannedStep {
        id,
        changes: changes_of(steps, id, ctx),
        blocking: deploy_steps::blocking(id),
        status: status.clone(),
    }
}
