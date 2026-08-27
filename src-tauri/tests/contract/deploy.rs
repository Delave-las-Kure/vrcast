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
    let err = api::deploy_run(&state, "nowhere", Ipv6Choice::Keep, false)
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
    let err = api::deploy_run(&state, "nowhere", Ipv6Choice::Keep, true)
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
