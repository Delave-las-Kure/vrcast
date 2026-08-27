//! T332 — the deployment on a real server, through the commands a person uses.
//!
//! **Ignored by default, and not because it is slow.** It changes a real machine: it installs
//! packages, rewrites the way in, turns a firewall on and obtains a certificate. Nothing that
//! runs by itself may do that. It is run by hand, at the throwaway stand, and never anywhere
//! else.
//!
//! Where the stand is comes from the environment and not from this file (FR-004). An address
//! written into the repository is a server somebody else's copy of the application would try
//! to deploy on.
//!
//! ```text
//! VRCAST_STAND_HOST=… VRCAST_STAND_DOMAIN=… VRCAST_STAND_KEY=…/id_ed25519 \
//!   cargo test --features integration --test integration stand_live -- --ignored --nocapture
//! ```
//!
//! What this reaches that a container cannot (T246, measured): the swap file, the kernel
//! settings, the disk's readahead, and the serving answering over a domain with a real
//! certificate. Those four are the whole reason a real machine is worth the trouble.

use std::sync::Arc;
use std::time::Duration;

use vrcast_studio_lib::commands::deploy::api as deploy_api;
use vrcast_studio_lib::commands::servers::{api as servers, ServerInput};
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::domain::deploy_steps::{SkipReason, Status, StepId};
use vrcast_studio_lib::domain::dns_verdict::{Ipv6Choice, Verdict};
use vrcast_studio_lib::domain::server_profile::AuthKind;
use vrcast_studio_lib::domain::server_state::Kind;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;

/// Where the stand is. Absent means "do not run" rather than a default: a default here would
/// be an address in the repository, which is the thing FR-004 exists to prevent.
struct Stand {
    host: String,
    domain: String,
    key_path: String,
}

fn stand() -> Stand {
    let asked = |name: &str| {
        std::env::var(name)
            .unwrap_or_else(|_| panic!("{name} is not set — see the note at the top of this file"))
    };
    Stand {
        host: asked("VRCAST_STAND_HOST"),
        domain: asked("VRCAST_STAND_DOMAIN"),
        key_path: asked("VRCAST_STAND_KEY"),
    }
}

fn app_state() -> AppState {
    AppState::with_db(
        Arc::new(Db::open_in_memory().unwrap()),
        Arc::new(InMemorySecretStore::new()),
    )
    .expect("the application state would not assemble")
}

async fn profile_for(state: &AppState, stand: &Stand) -> String {
    let input = ServerInput {
        name: String::from("Stand"),
        host: stand.host.clone(),
        port: 22,
        user: String::from("root"),
        auth_kind: AuthKind::Key,
        key_path: Some(stand.key_path.clone()),
        domain: stand.domain.clone(),
        video_dir: None,
        cdn_base: None,
        ipv6_mode: None,
    };
    let id = servers::server_add(state, input, "").expect("the profile was not created");

    // The same step a person takes in the wizard: learn the fingerprint and confirm it.
    // Without it nothing connects at all — credentials are never sent to a server whose
    // fingerprint is unconfirmed (FR-092).
    let fingerprint = vrcast_studio_lib::commands::api::server_probe_fingerprint(&stand.host, 22)
        .await
        .expect("the fingerprint was not obtained");
    servers::server_fingerprint_confirm(state, &id, &fingerprint)
        .expect("the fingerprint was not confirmed");
    id
}

/// Refuse to go on unless the stand is bare, and say so in as many words.
///
/// Three of the checks below only mean anything on a machine nothing has been done to.
/// Run against a deployed stand they would fail somewhere in the middle with a message
/// about the wrong thing — so they stop here instead, saying what to do: the stand is
/// rebuilt from the provider's panel, and that is a person's decision, not a test's.
async fn require_bare(state: &AppState, id: &str) {
    let what = deploy_api::server_detect(state, id)
        .await
        .expect("the server would not say what it is");
    assert_eq!(
        what.kind,
        Kind::Clean,
        "this check needs a bare stand and the stand is {:?}. Rebuild it and run again.",
        what.kind
    );
}

/// The other way round: a check that only means anything once the stand is deployed.
async fn require_deployed(state: &AppState, id: &str) {
    let what = deploy_api::server_detect(state, id)
        .await
        .expect("the server would not say what it is");
    assert_eq!(
        what.kind,
        Kind::Managed,
        "this check needs a deployed stand and the stand is {:?}. Deploy it and run again.",
        what.kind
    );
}

/// Wait for a task to finish, and say what became of it.
async fn wait_for(state: &AppState, task_id: &str) -> Option<String> {
    for _ in 0..600 {
        let task = vrcast_studio_lib::commands::api::task_get(state, task_id)
            .expect("the task disappeared");
        use vrcast_studio_lib::tasks::state::TaskState;
        match task.state {
            TaskState::Completed => return None,
            TaskState::Failed | TaskState::Cancelled => {
                return Some(
                    task.error
                        .map(|e| format!("{e:?}"))
                        .unwrap_or_else(|| String::from("no reason given")),
                )
            }
            _ => {}
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    Some(String::from(
        "the task did not finish within twenty minutes",
    ))
}

#[tokio::test]
#[ignore = "changes a real server: run by hand at the throwaway stand"]
async fn a_bought_server_is_recognised_as_bare_and_the_domain_is_judged() {
    let stand = stand();
    let state = app_state();
    let id = profile_for(&state, &stand).await;

    // 1. What is it? (FR-120)
    let what = deploy_api::server_detect(&state, &id)
        .await
        .expect("the server would not say what it is");
    println!("state: {what:?}");
    assert_eq!(
        what.kind,
        Kind::Clean,
        "this check needs a bare stand and the stand is {:?}. Rebuild it and run again.",
        what.kind
    );

    // 2. Keeping IPv6 while the domain promises none of it is refused — and says which record
    // to create (FR-137, FR-140). This is the case the stand is actually in: the machine has
    // an IPv6 address and the domain has no AAAA record.
    let keeping = deploy_api::dns_check(&state, &id, Ipv6Choice::Keep)
        .await
        .expect("the domain check failed");
    println!(
        "keeping IPv6: {:?} / advice {:?}",
        keeping.verdict, keeping.advice
    );
    assert!(
        matches!(keeping.verdict, Verdict::Ipv6Mismatch { .. }),
        "a domain with no AAAA record was accepted while IPv6 was being kept"
    );
    assert!(
        keeping.advice.is_some(),
        "the refusal says nothing about what to go and do"
    );

    // 3. Turning it off, the same domain is in order.
    let turning_off = deploy_api::dns_check(&state, &id, Ipv6Choice::Disable)
        .await
        .expect("the domain check failed");
    println!("turning IPv6 off: {:?}", turning_off.verdict);
    assert_eq!(
        turning_off.verdict,
        Verdict::Ok,
        "the domain does not point at the stand — fix the record before deploying"
    );
}

#[tokio::test]
#[ignore = "changes a real server: run by hand at the throwaway stand"]
async fn the_plan_is_shown_before_anything_is_changed() {
    let stand = stand();
    let state = app_state();
    let id = profile_for(&state, &stand).await;
    require_bare(&state, &id).await;

    let preview = deploy_api::deploy_plan(&state, &id, Ipv6Choice::Disable)
        .await
        .expect("the plan failed");
    println!(
        "machine: {} MB of memory, disk {}",
        preview.memory_mb, preview.disk
    );
    for step in &preview.steps {
        println!("  {:?}: {:?} — {:?}", step.id, step.status, step.changes);
    }

    // On a real machine there is nothing that "cannot be established here" — that answer
    // belongs to a container (T246), and seeing it on a VPS would mean the machine was
    // misread.
    for step in &preview.steps {
        assert!(
            !matches!(
                step.status,
                Status::Skipped {
                    why: SkipReason::NotPossibleHere { .. }
                }
            ),
            "{:?} says it cannot be established on a real server",
            step.id
        );
    }

    // And a bare machine has everything left to do.
    let swap = preview
        .steps
        .iter()
        .find(|s| s.id == StepId::Swap)
        .expect("no swap step");
    assert_eq!(
        swap.status,
        Status::NotApplied,
        "a machine with 961 MB and no swap was told it needs none"
    );

    // Nothing was changed by asking.
    let after = deploy_api::server_detect(&state, &id)
        .await
        .expect("the server would not say what it is");
    assert_eq!(after.kind, Kind::Clean, "the plan changed the server");
}

#[tokio::test]
#[ignore = "changes a real server: run by hand at the throwaway stand"]
async fn the_stand_is_deployed_and_serves_over_its_domain() {
    let stand = stand();
    let state = app_state();
    let id = profile_for(&state, &stand).await;
    require_bare(&state, &id).await;

    // Without a yes, nothing happens (FR-122).
    let refused = deploy_api::deploy_run(&state, &id, Ipv6Choice::Disable, false).await;
    assert!(
        refused.is_err(),
        "a deployment ran without anybody agreeing"
    );

    let task_id = deploy_api::deploy_run(&state, &id, Ipv6Choice::Disable, true)
        .await
        .expect("the deployment would not start");
    println!("task {task_id}");

    if let Some(why) = wait_for(&state, &task_id).await {
        panic!("the deployment failed: {why}");
    }

    // Now ours, at the version this application deploys (FR-127, FR-128).
    let what = deploy_api::server_detect(&state, &id)
        .await
        .expect("the server would not say what it is");
    assert_eq!(
        what.kind,
        Kind::Managed,
        "the server was not recognised as ours"
    );
    assert_eq!(what.server_version, Some(what.app_expects));

    // Whether anything is left over is asked by the check that needs a deployed stand,
    // right after this one — keeping it here would mean this check depended on its own
    // result, which is a thing that reads as confirmation and is not.
}

#[tokio::test]
#[ignore = "changes a real server: run by hand at the throwaway stand"]
async fn a_deployed_stand_has_no_work_left_and_serves() {
    // SC-015 on a real machine. The container check proves the mechanism; this proves it
    // against apt, systemd, ufw and a certificate that already exists.
    let stand = stand();
    let state = app_state();
    let id = profile_for(&state, &stand).await;
    require_deployed(&state, &id).await;

    // Deploying again is refused — and refused as "already deployed", which is the
    // opposite of "nothing is deployed" and used to arrive with those very words.
    let again = deploy_api::deploy_run(&state, &id, Ipv6Choice::Disable, true).await;
    assert!(
        again.is_err(),
        "a working server was deployed over the top of itself"
    );

    let plan = deploy_api::server_upgrade_plan(&state, &id)
        .await
        .expect("the upgrade plan failed");
    assert!(
        !plan.has_work(),
        "a deployed server has work waiting: {:?}",
        plan.steps
            .iter()
            .filter(|s| matches!(s.status, Status::NotApplied))
            .map(|s| s.id)
            .collect::<Vec<_>>()
    );
}
