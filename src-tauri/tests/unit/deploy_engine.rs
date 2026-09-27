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
use vrcast_studio_lib::domain::deploy_steps::{
    Change, Checked, PlannedStep, SkipReason, Status, StepId, ORDER,
};
use vrcast_studio_lib::server::deploy::{keeps_key_before, run, BeforeStep, DeployError, Step};
use vrcast_studio_lib::ssh::SshError;

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
    /// T616: the run made a key, and keeping it answers this (`None` — no key was made).
    keeping: Mutex<Option<Result<(), String>>>,
    /// T625: where a command of the run whose end was not heard fails to be settled —
    /// `("check" | "apply" | "recheck", step)`.
    unsettles: Mutex<Option<(&'static str, StepId)>>,
}

/// What `settle_unheard` hands back when the stop of an unheard command is not confirmed.
fn unsettled() -> DeployError {
    DeployError::Unsettled {
        id: None,
        error: SshError::Exec(String::from(
            "a command of this run whose end was not heard may still be running, and its stop \
             could not be confirmed: dpkg 77",
        )),
    }
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
    /// Everything that happened, in order: `check`, `apply` and (T616) `keep` of a step.
    timeline: Vec<(&'static str, StepId)>,
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
        inner.timeline.push(("check", id));
        let at = if inner.recheck_due {
            "recheck"
        } else {
            "check"
        };
        if *ctx.unsettles.lock().unwrap() == Some((at, id)) {
            return Err(unsettled());
        }
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
        inner.timeline.push(("apply", id));
        if *ctx.unsettles.lock().unwrap() == Some(("apply", id)) {
            return Err(unsettled());
        }
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
/// The real context's rule (`server::deploy::keeps_key_before`), with the keeping written down
/// instead of done. A refusal settles the step as the real one does, so the stand moves past
/// it: neither its check nor its apply will be asked.
impl BeforeStep for Stand {
    fn before_step<'a>(
        &'a self,
        id: StepId,
        done: &'a [PlannedStep],
    ) -> BoxFuture<'a, vrcast_studio_lib::server::deploy::Result<()>> {
        Box::pin(async move {
            let Some(answer) = self.keeping.lock().unwrap().clone() else {
                return Ok(());
            };
            if !keeps_key_before(id, done) {
                return Ok(());
            }
            let mut inner = self.inner.lock().unwrap();
            inner.timeline.push(("keep", id));
            answer.map_err(|why| {
                inner.at += 1;
                DeployError::Step {
                    id,
                    detail: why,
                    advice: None,
                }
            })
        })
    }
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

// ---------- T616: the made key is kept before password logins go ----------

fn with_key_kept(answer: Result<(), String>) -> Stand {
    let stand = Stand::saying(&[
        (StepId::SshKey, &[Checked::NotApplied, Checked::Applied]),
        (
            StepId::SshHardening,
            &[Checked::NotApplied, Checked::Applied],
        ),
    ]);
    *stand.keeping.lock().unwrap() = Some(answer);
    stand
}

fn position(timeline: &[(&'static str, StepId)], what: &str, id: StepId) -> Option<usize> {
    timeline.iter().position(|(w, i)| *w == what && *i == id)
}

#[tokio::test]
async fn the_made_key_is_kept_after_the_key_step_and_before_the_hardening_step_touches_anything() {
    // QA-19 №1: the key used to be kept once the whole run had returned — long after
    // `SshHardening` turned passwords off, so an application closed in between took the only
    // copy of the key with it.
    let stand = with_key_kept(Ok(()));
    carry_out(&stand).await.expect("the run failed");
    let timeline = stand.inner.lock().unwrap().timeline.clone();

    let key_proved = position(&timeline, "apply", StepId::SshKey).expect("SshKey not applied");
    let kept = position(&timeline, "keep", StepId::SshHardening).expect("the key was not kept");
    let hardening_asked =
        position(&timeline, "check", StepId::SshHardening).expect("SshHardening not checked");
    let hardening_applied =
        position(&timeline, "apply", StepId::SshHardening).expect("SshHardening not applied");
    assert!(
        key_proved < kept && kept < hardening_asked && hardening_asked < hardening_applied,
        "the key was not kept between the key step and the hardening step: {timeline:?}"
    );
    assert_eq!(
        timeline.iter().filter(|(w, _)| *w == "keep").count(),
        1,
        "the key was kept more than once: {timeline:?}"
    );
}

#[tokio::test]
async fn a_key_that_could_not_be_kept_stops_the_run_before_the_hardening_step() {
    let stand = with_key_kept(Err(String::from("the store said no")));
    let outcome = carry_out(&stand).await;
    match outcome {
        Err(DeployError::Step { id, ref detail, .. }) => {
            assert_eq!(id, StepId::SshHardening);
            assert!(detail.contains("the store said no"), "{detail}");
        }
        other => panic!("a key that could not be kept did not stop the run: {other:?}"),
    }
    let timeline = stand.inner.lock().unwrap().timeline.clone();
    assert!(
        position(&timeline, "check", StepId::SshHardening).is_none()
            && position(&timeline, "apply", StepId::SshHardening).is_none(),
        "password logins were touched although the key could not be kept: {timeline:?}"
    );
    assert!(
        !stand.applied().iter().any(|id| {
            ORDER.iter().position(|s| s == id) > ORDER.iter().position(|s| *s == StepId::SshKey)
        }),
        "the run went on past the key step: {:?}",
        stand.applied()
    );
}

#[tokio::test]
async fn no_key_is_kept_when_the_key_step_did_not_succeed_or_none_was_made() {
    // The key step failed (blocking): the run ends there, nothing is kept.
    let stand = with_key_kept(Ok(()));
    stand.fails_at(StepId::SshKey);
    assert!(carry_out(&stand).await.is_err());
    let timeline = stand.inner.lock().unwrap().timeline.clone();
    assert!(position(&timeline, "keep", StepId::SshHardening).is_none());

    // No key was made (a profile already on a key): nothing to keep.
    let stand = Stand::saying(&[]);
    carry_out(&stand).await.expect("the run failed");
    assert!(stand
        .inner
        .lock()
        .unwrap()
        .timeline
        .iter()
        .all(|(w, _)| *w != "keep"));
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

// ---------- what the failure says about where it stopped (T506, FR-123) ----------

/// ⚠ **The check the function's own doc comment claimed, and nothing made true.**
///
/// `failed`'s comment reads "The step is named in every case (FR-123)". It was not: the
/// arms set `{id:?}: {detail}` as the cause, and the last line of the function hung
/// `after N steps` on the whole `match` — and `with_cause` **replaces**. The step name was
/// destroyed on the way out, in the one function that exists to carry it. Nothing checked
/// this function at all, which is how a doc comment came to be the only thing asserting it.
///
/// Over **every** step rather than one: a rule stated about the one somebody picked is how
/// T517 stayed hidden for months.
#[test]
fn a_failure_always_says_which_step_it_stopped_at() {
    use vrcast_studio_lib::domain::wording::DetailCode;
    use vrcast_studio_lib::server::deploy::DeployError;
    use vrcast_studio_lib::tasks::deploy::failed;

    for id in ORDER {
        let broke = DeployError::Step {
            id,
            detail: String::from("the machine said no"),
            advice: None,
        };
        let error = failed(broke, &[]);
        let named = error
            .details
            .iter()
            .find(|d| d.key == DetailCode::DeployStoppedAtStep)
            .unwrap_or_else(|| panic!("{id:?} failed and the error does not say which step"));
        assert_eq!(
            named.params.get("step"),
            Some(&serde_json::json!(format!("{id:?}"))),
            "the detail names a different step from the one that failed"
        );
        // The particulars survive too: they are what a person pastes into a search.
        assert_eq!(error.cause.as_deref(), Some("the machine said no"));

        // And the same for the other failure that knows its step.
        let untaken = failed(DeployError::NotTaken { id }, &[]);
        assert!(
            untaken.says(DetailCode::DeployStoppedAtStep),
            "{id:?} was applied and did not take, and the error does not say which step"
        );
    }
}

/// A failure with no step still says how far it got — and does not claim a step it has not.
#[test]
fn a_failure_with_no_step_says_so_rather_than_naming_one() {
    use vrcast_studio_lib::domain::wording::DetailCode;
    use vrcast_studio_lib::server::deploy::DeployError;
    use vrcast_studio_lib::tasks::deploy::failed;

    let cancelled = failed(DeployError::Cancelled, &[]);
    assert!(cancelled.says(DetailCode::DeployStoppedAfter));
    assert!(
        !cancelled.says(DetailCode::DeployStoppedAtStep),
        "a cancellation blamed a step, and a person would go and look at it"
    );
}

// ---------- T625: an unheard command whose stop was not confirmed names its step ----------

/// Run the whole deployment with the stop of an unheard command failing at `at` of `id`;
/// hand back what the run ended with and every step it reported, in order.
async fn unsettled_run(
    at: &'static str,
    id: StepId,
) -> (
    vrcast_studio_lib::server::deploy::Result<Vec<PlannedStep>>,
    Vec<PlannedStep>,
    Stand,
) {
    let stand = match at {
        "check" => Stand::saying(&[]),
        "apply" => Stand::saying(&[(id, &[Checked::NotApplied])]),
        _ => Stand::saying(&[(id, &[Checked::NotApplied, Checked::Applied])]),
    };
    *stand.unsettles.lock().unwrap() = Some((at, id));
    let steps = all_steps();
    let never = || false;
    let mut seen: Vec<PlannedStep> = Vec::new();
    let outcome = run(&stand, &steps, &never, &mut |p| seen.push(p.clone())).await;
    (outcome, seen, stand)
}

/// FR-123 over every step and every place in a step a command is sent from — the check, the
/// apply, the check after the apply — and so over the non-blocking steps (`Fail2ban`,
/// `UnattendedUpgrades`, `Tuning`) as much as the blocking ones: the run ends at that step,
/// the step is reported `Failed` with the stop's words, and the failure is `DEPLOY_STEP_FAILED`
/// naming it, with the stop's words as `cause` (they used to arrive as `INTERNAL`, no step).
#[tokio::test]
async fn an_unsettled_unheard_command_fails_the_step_it_happened_at_and_names_it() {
    use vrcast_studio_lib::commands::error::ErrorCode;
    use vrcast_studio_lib::domain::wording::DetailCode;
    use vrcast_studio_lib::tasks::deploy::failed;

    for at in ["check", "apply", "recheck"] {
        for id in ORDER {
            let (outcome, seen, stand) = unsettled_run(at, id).await;
            let what = format!("{id:?} at its {at}");

            match &outcome {
                Err(DeployError::Unsettled {
                    id: Some(named), ..
                }) => {
                    assert_eq!(*named, id, "{what}: the error names another step")
                }
                other => panic!("{what}: the run did not end unsettled at its step: {other:?}"),
            }

            // Reported, and last: nothing of any later step was reported, asked or applied.
            let last = seen
                .last()
                .unwrap_or_else(|| panic!("{what}: nothing was reported"));
            assert_eq!(last.id, id, "{what}: {seen:?}");
            match &last.status {
                Status::Failed { detail } => assert!(
                    detail.contains("its stop could not be confirmed"),
                    "{what}: the step's failure lost the stop's words: {detail}"
                ),
                other => panic!("{what}: the step was reported {other:?}, not failed"),
            }
            let place = ORDER.iter().position(|s| *s == id).unwrap();
            assert!(
                stand
                    .asked()
                    .iter()
                    .chain(stand.applied().iter())
                    .all(|s| ORDER.iter().position(|o| o == s).unwrap() <= place),
                "{what}: the run went on past the step: asked {:?}, applied {:?}",
                stand.asked(),
                stand.applied()
            );

            let error = failed(outcome.unwrap_err(), &seen);
            assert_eq!(error.code, ErrorCode::DeployStepFailed, "{what}");
            let named = error
                .details
                .iter()
                .find(|d| d.key == DetailCode::DeployStoppedAtStep)
                .unwrap_or_else(|| panic!("{what}: the error does not say which step"));
            assert_eq!(
                named.params.get("step"),
                Some(&serde_json::json!(format!("{id:?}"))),
                "{what}"
            );
            assert_eq!(
                named.params.get("done"),
                Some(&serde_json::json!(place as u64)),
                "{what}: how far it got is not the steps before it"
            );
            assert!(
                error
                    .cause
                    .as_deref()
                    .is_some_and(|c| c.contains("its stop could not be confirmed")),
                "{what}: the cause is not the stop's own words: {:?}",
                error.cause
            );
        }
    }
}

/// Before any step — the mark of ours, the tidy-up, an upgrade's copy — there is no step to
/// name, and none is claimed: the connection's own error, as before T625.
#[test]
fn an_unsettled_unheard_command_before_any_step_names_none() {
    use vrcast_studio_lib::domain::wording::DetailCode;
    use vrcast_studio_lib::tasks::deploy::failed;

    let error = failed(unsettled(), &[]);
    assert!(error.says(DetailCode::DeployStoppedAfter));
    assert!(!error.says(DetailCode::DeployStoppedAtStep));
    assert!(error
        .cause
        .as_deref()
        .is_some_and(|c| c.contains("its stop could not be confirmed")));
}
