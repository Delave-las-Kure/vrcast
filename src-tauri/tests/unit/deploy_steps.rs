//! T263 — the order of a deployment, and what a repeat should do.
//!
//! The three mandatory pairs were each bought. The third was bought with a server nobody
//! could get back into, and none of the three fails when it is broken — the deployment runs
//! to the end and reports success, having installed packages on a machine with no swap, or
//! having turned off the way in before putting the key there.

use vrcast_studio_lib::domain::deploy_steps::{
    blocking, ordering_holds, plan, stops_the_run, to_apply, Change, Checked, OrderProblem,
    SkipReason, Status, StepId, MUST_PRECEDE, ORDER,
};

fn no_changes(_: StepId) -> Vec<Change> {
    Vec::new()
}

#[test]
fn the_order_we_ship_holds() {
    ordering_holds(&ORDER).expect("the deployment's own order breaks its own rules");
}

#[test]
fn checking_the_domain_has_to_come_first() {
    // A wrong record found by a check costs nothing. Found at the verifying step it costs a
    // half-configured server, and the person is left to work out which half.
    let mut order = ORDER;
    order.swap(0, 1);
    assert_eq!(
        ordering_holds(&order),
        Err(OrderProblem::DnsNotFirst {
            first: StepId::Swap
        })
    );
}

#[test]
fn every_mandatory_pair_is_caught_when_it_is_reversed() {
    // Written against the list rather than against three hand-made cases, so that a fourth
    // pair added later is checked by this without anybody remembering to come back here.
    for (earlier, later) in MUST_PRECEDE {
        let mut order = ORDER;
        let a = order.iter().position(|s| *s == earlier).expect("no step");
        let b = order.iter().position(|s| *s == later).expect("no step");
        order.swap(a, b);

        let verdict = ordering_holds(&order);
        assert!(
            verdict.is_err(),
            "{earlier:?} after {later:?} was allowed through"
        );
        // The first rule can fire instead when the swap moves the domain check: either
        // refusal is correct, and what matters is that the order does not pass.
        assert!(
            matches!(
                verdict,
                Err(OrderProblem::OutOfOrder { .. }) | Err(OrderProblem::DnsNotFirst { .. })
            ),
            "the refusal was about something else: {verdict:?}"
        );
    }
}

#[test]
fn an_order_that_is_not_the_whole_deployment_is_refused() {
    // A step named twice or one missing is not an order at all. Left unchecked it reads as a
    // deployment and is one step short of being one — and the missing step is found on the
    // server, later, by the person.
    let short: Vec<StepId> = ORDER.iter().copied().skip(1).collect();
    assert_eq!(ordering_holds(&short), Err(OrderProblem::NotEveryStepOnce));

    let mut doubled: Vec<StepId> = ORDER.to_vec();
    doubled[3] = StepId::DnsCheck;
    assert_eq!(
        ordering_holds(&doubled),
        Err(OrderProblem::NotEveryStepOnce)
    );
}

#[test]
fn a_repeat_does_not_redo_what_is_already_done() {
    // **What FR-124 is** (SC-015). The check has to be independent of the application — it
    // looks at the server, not at a record of what we did — or a repeat after a crash starts
    // from the beginning and undoes the half that succeeded.
    let found = vec![
        (StepId::DnsCheck, Checked::Applied),
        (StepId::Packages, Checked::Applied),
        (StepId::UserDirs, Checked::Applied),
    ];
    let todo = to_apply(&found);
    assert!(
        !todo.contains(&StepId::Packages),
        "packages were reinstalled"
    );
    assert!(
        todo.contains(&StepId::Configs),
        "the rest was not carried on with"
    );
    assert_eq!(
        todo.first(),
        Some(&StepId::Swap),
        "what remains has to keep the deployment's order, not the order the findings arrived in"
    );
}

#[test]
fn what_cannot_be_established_here_is_not_the_same_as_done() {
    // **The distinction T246 was measured for.** In a container `swapon` is refused whatever
    // the privileges — and `free` inside reports the host's swap, so a check that merely
    // looked would pass on a machine that has none. Folded into "applied", a run in a
    // container reports a fully deployed server with neither swap nor tuning, and that report
    // is worse than a failure because it is believed.
    let found = vec![
        (
            StepId::Swap,
            Checked::NotPossibleHere {
                detail: String::from("swapon is refused in a container"),
            },
        ),
        (StepId::Tuning, Checked::NotNeeded),
    ];

    // Neither is attempted...
    let todo = to_apply(&found);
    assert!(!todo.contains(&StepId::Swap));
    assert!(!todo.contains(&StepId::Tuning));

    // ...but the plan says which was which, and does not call either of them done.
    let shown = plan(&ORDER, &found, no_changes);
    let swap = shown
        .iter()
        .find(|s| s.id == StepId::Swap)
        .expect("no swap step");
    assert!(
        matches!(
            &swap.status,
            Status::Skipped {
                why: SkipReason::NotPossibleHere { .. }
            }
        ),
        "the swap step came back as {:?}",
        swap.status
    );
    let tuning = shown
        .iter()
        .find(|s| s.id == StepId::Tuning)
        .expect("no tuning step");
    assert_eq!(
        tuning.status,
        Status::Skipped {
            why: SkipReason::NotNeeded
        },
        "\"not needed on this server\" and \"cannot be established here\" are different answers"
    );
    assert_ne!(swap.status, Status::Applied);
}

#[test]
fn the_plan_shows_the_whole_deployment_and_in_its_own_order() {
    // Steps already done are shown, marked done, rather than left out. A plan that listed
    // only the remaining work would read differently on a repeat than on a first run, and the
    // person would have no way to tell "this was done earlier" from "this will not be done".
    let found = vec![(StepId::Packages, Checked::Applied)];
    let shown = plan(&ORDER, &found, no_changes);

    assert_eq!(
        shown.len(),
        ORDER.len(),
        "the plan is not the whole deployment"
    );
    let order: Vec<StepId> = shown.iter().map(|s| s.id).collect();
    assert_eq!(order, ORDER.to_vec());
    assert_eq!(
        shown
            .iter()
            .find(|s| s.id == StepId::Packages)
            .map(|s| s.status.clone()),
        Some(Status::Applied)
    );
}

#[test]
fn a_failure_stops_the_run_where_going_on_would_build_on_nothing() {
    // Without packages there is nothing to configure and nothing to start, and the failures
    // after such a step would say nothing about their own causes — they would all say
    // "missing", and the person would be reading a page of consequences with the cause five
    // screens up.
    assert!(stops_the_run(StepId::Packages));
    assert!(stops_the_run(StepId::SshKey));
    assert!(stops_the_run(StepId::Verify));

    // And where it would not. A kernel that will not take a setting is a reason to say so,
    // not to leave the person without a server: the tuning makes the serving faster, it does
    // not make it work.
    assert!(!blocking(StepId::Tuning));
    assert!(!blocking(StepId::Fail2ban));
    assert!(!blocking(StepId::UnattendedUpgrades));
}

#[test]
fn nothing_found_means_everything_is_to_be_done() {
    // A bare machine: the case the whole of phase 7 exists for, and the one where a filter
    // written the wrong way round would quietly do nothing at all and report success.
    assert_eq!(to_apply(&[]), ORDER.to_vec());
}
