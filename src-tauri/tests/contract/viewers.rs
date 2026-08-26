//! T165 — contract tests for the viewers and the settings.
//!
//! The contract: `contracts/ipc-commands.md`, the "Viewers and limits" and "Settings"
//! sections.
//!
//! What shows from outside: the shape of the answers, what happens when a server is asked
//! for that does not exist, and that switching the watching off is quiet. Every check goes
//! down a path that breaks off **before** the server is reached — a contract test must not
//! depend on a network being at hand.

use super::support::{state, valid_input};
use vrcast_studio_lib::commands::error::{DetailCode, ErrorCode};
use vrcast_studio_lib::commands::servers::api as servers;
use vrcast_studio_lib::commands::settings::api as settings_api;
use vrcast_studio_lib::commands::viewers::api as viewers;
use vrcast_studio_lib::store::settings::{
    Settings, MAX_HEAVY_TASKS, MAX_THRESHOLD_S, MIN_THRESHOLD_S,
};

#[tokio::test]
async fn watching_a_server_that_does_not_exist_says_so() {
    let state = state();
    let error = viewers::viewers_watch_start(&state, "no-such-server")
        .await
        .expect_err("watching a server that is not there was allowed");
    // The same refusal the rest of the commands give for an unknown server, rather than one
    // of this module's own: a person meeting it in two places should meet the same thing.
    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert!(
        error
            .details
            .iter()
            .any(|d| d.key == DetailCode::ProfileNotFound),
        "the refusal does not say the profile is the thing missing: {error:?}"
    );
}

#[tokio::test]
async fn stopping_when_nothing_was_being_watched_is_quiet() {
    // The interface closes a screen it may never have opened, and it happens on every exit.
    // An error here would be an error in the ordinary case.
    let state = state();
    viewers::viewers_watch_stop(&state);
    viewers::viewers_watch_stop(&state);
    assert!(state.viewers.watching().is_none());
}

#[tokio::test]
async fn the_history_starts_empty_and_is_not_an_error() {
    let state = state();
    assert!(viewers::viewers_history(&state).is_empty());
}

#[tokio::test]
async fn a_failed_start_leaves_nothing_being_watched() {
    // The two standing channels are the whole budget for watching (R-04, T153). A start
    // that failed halfway and left itself recorded would mean the next one is refused for
    // want of channels that nobody is using.
    let state = state();
    let id = servers::server_add(&state, valid_input("Server"), "password")
        .expect("the profile would not set up");

    // There is no such server to connect to, so this cannot succeed. What matters is what
    // it leaves behind.
    let _ = viewers::viewers_watch_start(&state, &id).await;
    assert!(
        state.viewers.watching().is_none(),
        "a watch that never started is recorded as running"
    );
}

// ---------- the settings ----------

#[test]
fn the_settings_come_back_with_their_defaults_before_anything_is_set() {
    let state = state();
    let settings = settings_api::settings_get(&state).expect("the settings would not read");

    assert_eq!(settings, Settings::default());
    // The one default that is a decision rather than a value: asking an outside service
    // means handing it a viewer's address, and FR-057 says that is off unless the person
    // turns it on themselves.
    assert!(
        !settings.geo_refine_outside,
        "placing addresses through an outside service is on by default"
    );
}

#[test]
fn what_is_set_is_what_comes_back() {
    let state = state();
    let wanted = Settings {
        viewer_activity_threshold_s: 45,
        geo_refine_outside: true,
        concurrent_heavy_tasks: 2,
        mascot: false,
        animations: false,
        language: Some(String::from("en")),
        theme: Some(String::from("dark")),
    };

    let saved = settings_api::settings_set(&state, &wanted).expect("the settings would not save");
    assert_eq!(saved, wanted);
    assert_eq!(
        settings_api::settings_get(&state).expect("the settings would not read"),
        wanted
    );
}

#[test]
fn a_value_out_of_its_range_is_brought_back_into_it_rather_than_refused() {
    // Out-of-range values come from an older version or from somebody editing the file by
    // hand. Refusing to start over one would turn a convenience into a way of locking a
    // person out of their own application.
    let state = state();
    let saved = settings_api::settings_set(
        &state,
        &Settings {
            viewer_activity_threshold_s: 0,
            concurrent_heavy_tasks: 999,
            ..Settings::default()
        },
    )
    .expect("the settings would not save");

    assert_eq!(saved.viewer_activity_threshold_s, MIN_THRESHOLD_S);
    assert_eq!(saved.concurrent_heavy_tasks, MAX_HEAVY_TASKS);

    let saved = settings_api::settings_set(
        &state,
        &Settings {
            viewer_activity_threshold_s: 100_000,
            ..Settings::default()
        },
    )
    .expect("the settings would not save");
    assert_eq!(saved.viewer_activity_threshold_s, MAX_THRESHOLD_S);
}

#[test]
fn the_language_and_the_theme_can_be_left_to_the_system() {
    // "Not chosen" has to survive being stored: were it kept as an empty string, "the
    // system's" would be indistinguishable from a language named "".
    let state = state();
    settings_api::settings_set(
        &state,
        &Settings {
            language: Some(String::from("ru")),
            ..Settings::default()
        },
    )
    .expect("the settings would not save");

    let cleared = settings_api::settings_set(
        &state,
        &Settings {
            language: None,
            ..Settings::default()
        },
    )
    .expect("the settings would not save");
    assert_eq!(cleared.language, None);
    assert_eq!(
        settings_api::settings_get(&state)
            .expect("the settings would not read")
            .language,
        None
    );
}
