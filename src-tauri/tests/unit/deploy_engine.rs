//! T267 — the mechanism, checked without a server.
//!
//! Every one of the fifteen steps rests on this, and what it has to get right is not "the
//! steps ran" but the awkward middle: a repeat that skips what is done, an apply that
//! returns without having done anything, a failure that should stop the run and one that
//! should not. None of those need a server to ask about, and a mechanism that could only be
//! checked through one would count as unchecked.
//!
//! The stand-in below is a note-taking context: its checks answer from a script and its
//! applies write down that they were called. Nothing about a real server is claimed here —
//! that is `deploy_clean.rs`'s work.

use std::collections::HashMap;
use std::sync::Mutex;

use futures::future::BoxFuture;
use vrcast_studio_lib::domain::deploy_steps::{Change, Checked, SkipReason, Status, StepId, ORDER};
use vrcast_studio_lib::server::deploy::{run, DeployError, Step};

/// What a step is handed here: a script of answers and a place to write down what happened.
///
/// It keeps its own place in the deployment rather than working out which step is being
/// asked about, because working it out is guessing — the first attempt did guess, and it
/// answered for the step after the one being applied. The engine's protocol is fixed and
/// short (check, and if the answer is "not applied" then apply and check again), so the
/// stand simply follows it.
#[derive(Default)]
struct Stand {
    inner: Mutex<Inner>,
    /// What each step's check answers, one call at a time. The last answer repeats.
    says: Mutex<HashMap<StepId, Vec<Checked>>>,
    /// Steps whose apply fails.
    fails: Mutex<Vec<StepId>>,
}

#[derive(Default)]
struct Inner {
    /// How far through [`ORDER`] we are.
    at: usize,
    /// The step was applied and the engine owes it a second check.
    recheck_due: bool,
    /// Which steps had their check called, in order.
    asked: Vec<StepId>,
    /// Which steps had their apply called, in order.
    applied: Vec<StepId>,
}

impl Stand {
    fn saying(pairs: &[(StepId, &[Checked])]) -> Self {
        let stand = Self::default();
        {
            let mut says = stand.says.lock().unwrap();
            for (id, answers) in pairs {
                says.insert(*id, answers.to_vec());
            }
        }
        stand
    }

    fn scripted(&self, id: StepId) -> Checked {
        let mut says = self.says.lock().unwrap();
        let answers = says.entry(id).or_insert_with(|| vec![Checked::Applied]);
        if answers.len() > 1 {
            answers.remove(0)
        } else {
            answers[0].clone()
        }
    }

    fn applied(&self) -> Vec<StepId> {
        self.inner.lock().unwrap().applied.clone()
    }

    fn asked(&self) -> Vec<StepId> {
        self.inner.lock().unwrap().asked.clone()
    }

    fn fails_at(&self, id: StepId) {
        self.fails.lock().unwrap().push(id);
    }
}

fn check(ctx: &Stand) -> BoxFuture<'_, vrcast_studio_lib::server::deploy::Result<Checked>> {
    Box::pin(async move {
        let id = {
            let inner = ctx.inner.lock().unwrap();
            ORDER.get(inner.at).copied().unwrap_or(StepId::State)
        };
        let answer = ctx.scripted(id);
        let mut inner = ctx.inner.lock().unwrap();
        inner.asked.push(id);
        if inner.recheck_due {
            // The second look, after applying. Whatever it says, this step is settled.
            inner.recheck_due = false;
            inner.at += 1;
        } else if answer == Checked::NotApplied {
            // The engine will apply and come back.
            inner.recheck_due = true;
        } else {
            inner.at += 1;
        }
        Ok(answer)
    })
}

fn apply(ctx: &Stand) -> BoxFuture<'_, vrcast_studio_lib::server::deploy::Result<()>> {
    Box::pin(async move {
        let mut inner = ctx.inner.lock().unwrap();
        let id = ORDER.get(inner.at).copied().unwrap_or(StepId::State);
        inner.applied.push(id);
        if ctx.fails.lock().unwrap().contains(&id) {
            // A failed apply is not followed by a second check: the step is settled here.
            inner.recheck_due = false;
            inner.at += 1;
            return Err(DeployError::Step {
                id,
                detail: String::from("the stand was told to fail here"),
                advice: None,
            });
        }
        Ok(())
    })
}
fn no_changes(_: &Stand) -> Vec<Change> {
    Vec::new()
}

fn all_steps() -> Vec<Step<Stand>> {
    ORDER
        .iter()
        .map(|id| Step {
            id: *id,
            changes: no_changes,
            check,
            apply,
        })
        .collect()
}

async fn carry_out(stand: &Stand) -> vrcast_studio_lib::server::deploy::Result<Vec<Status>> {
    let steps = all_steps();
    let never = || false;
    let mut seen: Vec<Status> = Vec::new();
    let outcome = run(stand, &steps, &never, &mut |planned| {
        seen.push(planned.status.clone())
    })
    .await;
    outcome.map(|_| seen)
}

#[tokio::test]
async fn a_step_already_done_is_not_done_again() {
    // **The whole of safety on a repeat** (FR-124, SC-015). Nothing was remembered between
    // the two runs for this to hold: the check looks at the server.
    let stand = Stand::saying(&[]);
    let statuses = carry_out(&stand).await.expect("the run failed");

    assert!(
        stand.applied().is_empty(),
        "something was applied on a server where everything was already done"
    );
    assert!(statuses.iter().all(|s| *s == Status::Applied));
}

#[tokio::test]
async fn a_step_that_is_not_done_is_applied_and_then_asked_about_again() {
    let stand = Stand::saying(&[(StepId::DnsCheck, &[Checked::NotApplied, Checked::Applied])]);
    let statuses = carry_out(&stand).await.expect("the run failed");

    assert_eq!(
        stand.applied(),
        vec![StepId::DnsCheck],
        "the wrong steps were applied"
    );
    assert_eq!(statuses.first(), Some(&Status::Applied));
}

#[tokio::test]
async fn an_apply_that_did_nothing_is_a_failure_however_quietly_it_returned() {
    // **The six-month mistake.** On the live server the hardening step was written, ran
    // without complaint, and `sshd -T` went on saying password logins were allowed while
    // twenty-two thousand attempts a day went at it. An apply that returns proves nothing;
    // the check is what says the thing is so.
    let stand = Stand::saying(&[(
        StepId::DnsCheck,
        &[Checked::NotApplied, Checked::NotApplied],
    )]);
    let outcome = carry_out(&stand).await;

    match outcome {
        Err(DeployError::NotTaken { id }) => assert_eq!(id, StepId::DnsCheck),
        other => panic!("an apply that did nothing was accepted: {other:?}"),
    }
    assert_eq!(
        stand.applied(),
        vec![StepId::DnsCheck],
        "it was not even applied once"
    );
}

#[tokio::test]
async fn a_blocking_failure_stops_the_run() {
    // Going on would apply the rest to a server missing what they need, and every failure
    // after it would say "missing" — a page of consequences with the cause five screens up.
    let stand = Stand::saying(&[(StepId::DnsCheck, &[Checked::NotApplied])]);
    stand.fails_at(StepId::DnsCheck);

    match carry_out(&stand).await {
        Err(DeployError::Step { id, .. }) => assert_eq!(id, StepId::DnsCheck),
        other => panic!("a blocking failure did not stop the run: {other:?}"),
    }
    assert_eq!(
        stand.applied().len(),
        1,
        "the run went on after a blocking step failed"
    );
}

#[tokio::test]
async fn what_cannot_be_established_here_is_neither_applied_nor_called_done() {
    // T246 measured that a container cannot do swap or the kernel settings. Folded into
    // "applied", a run there would report a fully deployed server that has neither — and that
    // report is worse than a failure, because it is believed.
    let stand = Stand::saying(&[(
        StepId::Swap,
        &[Checked::NotPossibleHere {
            detail: String::from("swapon is refused in a container"),
        }],
    )]);
    let statuses = carry_out(&stand).await.expect("the run failed");

    assert!(
        !stand.applied().contains(&StepId::Swap),
        "a step that cannot be carried out here was attempted anyway"
    );
    let at = ORDER.iter().position(|id| *id == StepId::Swap).unwrap();
    assert!(
        matches!(
            &statuses[at],
            Status::Skipped {
                why: SkipReason::NotPossibleHere { .. }
            }
        ),
        "it was recorded as {:?}",
        statuses[at]
    );
}

#[tokio::test]
async fn the_run_keeps_the_deployment_s_order_whatever_order_it_was_handed() {
    // The order is not a preference in four places (R-12), and a caller building the list by
    // hand is exactly where it would be got wrong.
    let stand = Stand::default();
    let mut steps = all_steps();
    steps.reverse();
    let never = || false;
    run(&stand, &steps, &never, &mut |_| {})
        .await
        .expect("the run failed");

    let asked = stand.asked();
    let mut order: Vec<StepId> = Vec::new();
    for id in asked {
        if !order.contains(&id) {
            order.push(id);
        }
    }
    assert_eq!(
        order,
        ORDER.to_vec(),
        "the steps were carried out in the order they were handed in, not the deployment's own"
    );
}

#[tokio::test]
async fn a_cancelled_run_stops_where_it_is() {
    let stand = Stand::default();
    let steps = all_steps();
    let always = || true;
    match run(&stand, &steps, &always, &mut |_| {}).await {
        Err(DeployError::Cancelled) => {}
        other => panic!("cancelling did not stop the run: {other:?}"),
    }
    assert!(
        stand.asked().is_empty(),
        "a cancelled run still asked the server about things"
    );
}

#[test]
fn the_assembled_deployment_is_every_step_exactly_once() {
    // The list in `all()` is written by hand and the engine imposes the order anyway — so what
    // is left to get wrong is forgetting one, and a deployment short of a step reports success
    // and leaves the server without it. `ordering_holds` refuses a list that is not the whole
    // deployment, which is what makes this a check rather than a comment.
    use vrcast_studio_lib::domain::deploy_steps::ordering_holds;

    let ids: Vec<StepId> = vrcast_studio_lib::server::deploy::all()
        .iter()
        .map(|step| step.id)
        .collect();
    ordering_holds(&ids).expect("the assembled deployment is not the deployment");
    assert_eq!(ids, ORDER.to_vec());
}
