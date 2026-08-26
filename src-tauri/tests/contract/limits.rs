//! T208 — contract tests for the quality-limit commands.
//!
//! Contract: `contracts/ipc-commands.md`, "Зрители и ограничения".
//!
//! Only what is visible from outside. Two things here are not decoration: that a limit
//! cannot go on without somebody having said yes, and that a medium with one quality is
//! refused by name rather than left doing nothing.

use vrcast_studio_lib::commands::error::ErrorCode;
use vrcast_studio_lib::commands::limits::{api as limits, LimitRequest};

use super::support::state;

fn asking_about(server_id: &str) -> LimitRequest {
    LimitRequest {
        server_id: server_id.to_owned(),
        // An address from the block set aside for documentation: it belongs to nobody.
        ip: String::from("203.0.113.10"),
        slug: String::from("demo"),
        cap_bps: 6_000_000,
    }
}

#[tokio::test]
async fn a_limit_on_a_server_that_does_not_exist_is_a_failure_naming_it() {
    let state = state();
    let err = limits::limit_preview(&state, &asking_about("nowhere"))
        .await
        .expect_err("a limit was previewed for a server that does not exist");
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

#[tokio::test]
async fn a_limit_is_not_applied_without_somebody_having_said_yes() {
    // FR-066. What is being edited is the configuration of the thing serving somebody's
    // film at that moment. A change made without a word is not a thing this application
    // does — and the refusal has to come **before** any server is touched, which is why it
    // is checked here rather than after a connection.
    let state = state();
    let err = limits::limit_set(&state, asking_about("nowhere"), false)
        .await
        .expect_err("a limit went on unconfirmed");
    assert_eq!(
        err.code,
        ErrorCode::ConfirmationRequired,
        "the refusal was about something else, so the confirmation is not what stopped it"
    );
}

#[tokio::test]
async fn clearing_and_listing_on_a_server_that_does_not_exist_fail_by_name() {
    let state = state();
    assert_eq!(
        limits::limit_clear(&state, "nowhere", "203.0.113.10", "demo")
            .await
            .expect_err("a limit was cleared on a server that does not exist")
            .code,
        ErrorCode::InvalidInput
    );
    assert_eq!(
        limits::limits_list(&state, "nowhere")
            .await
            .expect_err("limits were listed for a server that does not exist")
            .code,
        ErrorCode::InvalidInput
    );
}

#[tokio::test]
async fn the_codes_the_contract_promises_all_exist() {
    // A code named in the contract and absent from the core is a promise to an interface
    // that will never be kept — and it is the interface that finds out, at a person's
    // machine, by showing them nothing.
    for code in [
        ErrorCode::NoLadderForMedia,
        ErrorCode::CaddyValidateFailed,
        ErrorCode::CaddyReloadFailed,
    ] {
        assert!(
            ErrorCode::parse(code.as_str()).is_some(),
            "{} does not read back",
            code.as_str()
        );
    }
}
