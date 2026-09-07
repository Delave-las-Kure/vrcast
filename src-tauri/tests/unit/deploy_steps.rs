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

// ---------- what is copied aside, and where it goes back to (T513) ----------

/// ⚠ **The backup is flat, so two files with one name would be one file.**
///
/// `back_up` copies every owned path into a timestamped directory by `cp -a "$f" {dir}/` —
/// the basename and nothing else. Two entries sharing a basename would clobber each other
/// there, and the restore would then put whichever survived into both places, quietly, on a
/// server somebody was rolling back precisely because something had gone wrong.
///
/// Nothing checked it, and nothing about the list makes it obvious: eight paths in eight
/// different directories, and the collision arrives on the day a ninth is added.
#[test]
fn no_two_backed_up_files_share_a_name() {
    let owned = vrcast_studio_lib::server::upgrade::owned_files();
    assert!(
        owned.len() >= 8,
        "the list of files copied aside has shrunk to {} — if that is deliberate, say so here",
        owned.len()
    );

    let mut names: Vec<&str> = owned
        .iter()
        .map(|p| p.rsplit('/').next().unwrap_or(p))
        .collect();
    names.sort_unstable();
    let before = names.len();
    names.dedup();
    assert_eq!(
        names.len(),
        before,
        "two files copied aside share a name, so one overwrites the other in the backup \
         directory and the restore puts the survivor in both places"
    );
}

/// Everything copied aside has somewhere to go back to.
///
/// ⚠ **This was two lists until 2026-09-06**: eight paths in `OWNED` and eight arms of a
/// hand-written `case` in the restore, kept in step by nobody. Add a file to the first and
/// forget the second, and it is saved faithfully and never put back — a rollback that reports
/// success having restored less than it saved. The arms are made from the list now, so the
/// two cannot drift; this is what says the making still covers everything.
#[test]
fn every_backed_up_file_has_a_way_back() {
    let owned = vrcast_studio_lib::server::upgrade::owned_files();
    let arms = vrcast_studio_lib::server::upgrade::restore_arms();

    for path in &owned {
        let name = path.rsplit('/').next().unwrap_or(path);
        assert!(
            arms.contains(&format!("    {name})")),
            "{path} is copied aside and the restore has no arm for it, so a rollback would \
             leave it as the failed upgrade left it"
        );
        assert!(
            arms.contains(&format!("cp -a \"$f\" {path} ;;")),
            "{path} has an arm that does not put it back where it came from"
        );
    }
    assert_eq!(
        arms.lines().count(),
        owned.len(),
        "the restore has arms for something that is not copied aside, or two for one file"
    );
}

// ---------- what a step says it will change (T507, FR-122) ----------

/// Every change says which it is, and the ones that carry values carry them.
///
/// ⚠ **The values were the whole point and the easiest thing to lose.** A step that says
/// "installs packages" and not *which* packages has told a person nothing they could not have
/// guessed from its name — and its name is what the screen showed for years while these values
/// were computed and thrown away. Breaking the packages arm to drop its values on purpose was
/// what showed this test was missing: everything else passed.
#[test]
fn every_change_says_which_it_is_and_carries_its_values() {
    use vrcast_studio_lib::domain::deploy_steps::Change;
    use vrcast_studio_lib::domain::wording::DetailCode;

    let every: Vec<Change> = vec![
        Change::LooksOnly,
        Change::InstallsPackages {
            names: vec![String::from("ffmpeg"), String::from("caddy")],
        },
        Change::CreatesSwapFile { megabytes: 2048 },
        Change::CreatesSystemUser {
            name: String::from("vrcast"),
        },
        Change::CreatesDirectory {
            path: String::from("/opt/vrcast"),
        },
        Change::WritesFile {
            path: String::from("/etc/caddy/Caddyfile"),
        },
        Change::EnablesService {
            name: String::from("caddy"),
        },
        Change::OpensPorts {
            ports: vec![String::from("80/tcp"), String::from("443/tcp")],
        },
        Change::ClosesEverythingElse,
        Change::AddsSshKey,
        Change::TurnsPasswordLoginOff,
        Change::TurnsIpv6Off,
        Change::SetsKernelSettings,
    ];

    // Distinct codes, or two different changes read as one thing on screen.
    let mut codes: Vec<DetailCode> = every.iter().map(|c| c.detail().key).collect();
    let before = codes.len();
    codes.sort_by_key(|c| c.as_str());
    codes.dedup();
    assert_eq!(
        codes.len(),
        before,
        "two changes share a code, so a person is shown the same sentence for different work"
    );

    let packages = every[1].detail();
    assert_eq!(
        packages.params.get("names"),
        Some(&serde_json::json!("ffmpeg, caddy")),
        "a step that installs packages does not name them, which is the whole of what a \
         person could not have guessed from the step's own title"
    );
    assert_eq!(packages.params.get("count"), Some(&serde_json::json!(2)));

    let ports = every[7].detail();
    assert_eq!(
        ports.params.get("ports"),
        Some(&serde_json::json!("80/tcp, 443/tcp")),
        "the firewall step does not say which ports it opens"
    );
    assert_eq!(ports.params.get("count"), Some(&serde_json::json!(2)));

    assert_eq!(
        every[2].detail().params.get("megabytes"),
        Some(&serde_json::json!(2048)),
        "the swap step does not say how large a file it will make"
    );
    for (which, key) in [(3usize, "name"), (4, "path"), (5, "path"), (6, "name")] {
        assert!(
            every[which].detail().params.contains_key(key),
            "a change carrying a {key} lost it on the way to being said"
        );
    }
}
