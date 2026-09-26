//! T297 — the deployment commands, at their own layer.
//!
//! What is checked here is the **order** of the refusals, which is the part that cannot be
//! seen from either side alone. A deployment that asked the server first and the person second
//! would install packages and then discover the domain was wrong; one that took `confirmed`
//! for granted would rewrite the way in to a machine nobody agreed to.
//!
//! None of these reach a server: every one of them is refused before that, which is the
//! point. A command that got as far as connecting would fail with a different code, and that
//! difference is what makes these checks about ordering rather than about outcomes.

use vrcast_studio_lib::commands::deploy::api;
use vrcast_studio_lib::commands::error::ErrorCode;
use vrcast_studio_lib::domain::dns_verdict::Ipv6Choice;

use super::support::state;

#[tokio::test]
async fn deploying_without_a_yes_is_refused_before_anything_else() {
    // **Before anything else**, and that is what this checks: the server does not exist, so a
    // command that went looking would fail with a different code. FR-122 says the list of
    // changes is shown and agreed to first, and the confirmation is what carries that.
    let state = state();
    let err = api::deploy_run(&state, "nowhere", Ipv6Choice::Keep, false, false)
        .await
        .expect_err("a deployment ran without anybody agreeing to it");
    assert_eq!(err.code, ErrorCode::ConfirmationRequired);
}

#[tokio::test]
async fn upgrading_without_a_yes_is_refused_before_anything_else() {
    let state = state();
    let err = api::server_upgrade_run(&state, "nowhere", false)
        .await
        .expect_err("an upgrade ran without anybody agreeing to it");
    assert_eq!(err.code, ErrorCode::ConfirmationRequired);
}

#[tokio::test]
async fn a_confirmed_deployment_gets_past_the_confirmation_and_fails_on_the_server() {
    // The other half of the same rule: with a yes, the refusal must be about something else.
    // Without this, a confirmation check that refused everything would pass the two above.
    let state = state();
    let err = api::deploy_run(&state, "nowhere", Ipv6Choice::Keep, true, false)
        .await
        .expect_err("a deployment was started for a server that does not exist");
    assert_ne!(
        err.code,
        ErrorCode::ConfirmationRequired,
        "a confirmed deployment was still refused as unconfirmed"
    );
}

#[tokio::test]
async fn asking_about_a_server_that_is_not_in_the_profiles_says_so() {
    let state = state();
    for code in [
        api::server_detect(&state, "nowhere").await.map(|_| ()),
        api::dns_check(&state, "nowhere", Ipv6Choice::Keep)
            .await
            .map(|_| ()),
        api::deploy_plan(&state, "nowhere", Ipv6Choice::Keep)
            .await
            .map(|_| ()),
        api::server_upgrade_plan(&state, "nowhere")
            .await
            .map(|_| ()),
        api::server_rollback(&state, "nowhere").await,
    ] {
        let err = code.expect_err("a command answered about a server that does not exist");
        assert_eq!(
            err.code,
            ErrorCode::InvalidInput,
            "the wrong code for an unknown server: {err:?}"
        );
    }
}

// ---------- T611: somebody else's Caddyfile, and a rollback with nothing to put back ----------

/// A rollback with no copy to put back answers with its own code, not `INTERNAL`.
///
/// It used to be an SFTP-flavoured internal error with "there is nothing to roll back to" in
/// `cause`: a person was asked to report a bug about a server that simply had no copy. The
/// other case — a copy that would not go back — keeps the step's own mapping.
#[test]
fn a_rollback_with_no_copy_says_so_with_its_own_code() {
    use vrcast_studio_lib::commands::deploy::rollback_error;
    use vrcast_studio_lib::server::deploy::DeployError;
    use vrcast_studio_lib::server::upgrade::RollbackError;

    let none = rollback_error(RollbackError::NoCopy);
    assert_eq!(none.code, ErrorCode::RollbackNoCopy);
    assert_eq!(none.code.as_str(), "ROLLBACK_NO_COPY");

    let cancelled = rollback_error(RollbackError::Failed(DeployError::Cancelled));
    assert_eq!(
        cancelled.code,
        ErrorCode::TaskCancelled,
        "a rollback that failed for another reason lost its own code"
    );
}

/// The plan's new field crosses the boundary under the name `contract.ts` declares.
#[test]
fn the_deploy_preview_carries_foreign_caddyfile_by_that_name() {
    use vrcast_studio_lib::commands::deploy::{DeployPreview, DomainAnswer};
    use vrcast_studio_lib::domain::dns_verdict::Verdict;

    let preview = DeployPreview {
        domain: DomainAnswer {
            verdict: Verdict::Ok,
            a: Vec::new(),
            aaaa: Vec::new(),
            advice: None,
        },
        steps: Vec::new(),
        memory_mb: 1024,
        disk: String::from("vda"),
        foreign_caddyfile: true,
    };
    let json = serde_json::to_value(&preview).expect("the preview will not serialise");
    assert_eq!(json["foreign_caddyfile"], serde_json::json!(true));

    let ts = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/shared/contract.ts"),
    )
    .expect("contract.ts would not read");
    let at = ts
        .find("export interface DeployPreview {")
        .expect("contract.ts no longer declares DeployPreview");
    let body = &ts[at..at + ts[at..].find("\n}").expect("DeployPreview is not closed")];
    for field in json.as_object().expect("an object").keys() {
        assert!(
            body.contains(&format!("\n  {field}:")),
            "the core sends DeployPreview.{field} and contract.ts does not declare it"
        );
    }
}

/// `deploy_run` takes the answer under the name the interface sends, and reads nothing sent
/// as "not agreed".
///
/// Tauri maps a command's `snake_case` arguments to `camelCase` keys; the interface has to
/// send `replaceCaddyfile`, or the tick a person made never arrives and the run refuses at the
/// configuration step for a reason the screen says was answered.
#[test]
fn the_interface_sends_the_answer_about_the_caddyfile_under_the_name_the_command_reads() {
    let ipc = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/shared/ipc.ts"),
    )
    .expect("ipc.ts would not read");
    let at = ipc
        .find("call<string>(\"deploy_run\", {")
        .expect("ipc.ts no longer calls deploy_run the way this check reads it");
    let sent = &ipc[at..at + ipc[at..].find('}').expect("the payload is not closed")];
    assert!(
        sent.contains("replaceCaddyfile"),
        "ipc.deployRun does not send replaceCaddyfile: {sent}"
    );

    let core = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/deploy.rs"),
    )
    .expect("deploy.rs would not read");
    assert!(
        core.contains("replace_caddyfile: Option<bool>,"),
        "the deploy_run command no longer takes replace_caddyfile as an optional argument — an \
         interface that sends nothing must be read as \"not agreed\", not refused"
    );
}
