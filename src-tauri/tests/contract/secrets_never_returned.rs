//! T037 — no command hands a secret back outside.
//!
//! Constitution, principle IV; the command layer's contract, rule 3: a secret crosses the
//! boundary **exactly once** — when the interface passes it while creating or changing a
//! profile. It never comes back.
//!
//! The check is built as a search rather than as an inspection of fields: a command's answer
//! is turned into what really goes to the interface — JSON — and the secret itself is looked
//! for in it. Inspecting fields would only cover the places the test's author thought of; a
//! search catches what nobody thought of too — a secret that reached the text of an error
//! from somebody else's library, for instance.

use super::support::{state, valid_input};
use vrcast_studio_lib::commands::servers::api as servers_api;
use vrcast_studio_lib::commands::{api, AppState};
use vrcast_studio_lib::domain::server_profile::AuthKind;

/// The secrets are deliberately long and unlike anything else: a short value could turn up
/// in an answer by chance and give false calm in the other direction.
///
/// They are deliberately not ASCII, either: `contains_secret` has a branch for the JSON
/// escape form, and with a Latin secret that branch would never run.
const PASSWORD: &str = "пароль-который-не-должен-выйти-наружу-a1b2c3";
const PASSPHRASE: &str = "парольная-фраза-ключа-которая-тоже-не-должна-выйти-d4e5f6";

/// Find a secret in the form the answer goes to the interface in.
///
/// Both representations are checked: as it stands, and as JSON writes it (some serialisers
/// turn non-ASCII into escape sequences, and a search over the original string would let
/// such a leak through).
fn contains_secret(json: &str, secret: &str) -> bool {
    if json.contains(secret) {
        return true;
    }
    let escaped: String = secret
        .chars()
        .map(|c| {
            if c.is_ascii() {
                c.to_string()
            } else {
                format!("\\u{:04x}", c as u32)
            }
        })
        .collect();
    json.contains(&escaped)
}

fn assert_clean<T: serde::Serialize>(what: &str, value: &T) {
    let json = serde_json::to_string(value).expect("the command's answer will not serialise");
    for secret in [PASSWORD, PASSPHRASE] {
        assert!(
            !contains_secret(&json, secret),
            "A SECRET IS IN THE ANSWER of the command {what}: {json}"
        );
    }
}

fn assert_error_clean(what: &str, err: &vrcast_studio_lib::commands::error::AppError) {
    let json = serde_json::to_string(err).expect("the error will not serialise");
    for secret in [PASSWORD, PASSPHRASE] {
        assert!(
            !contains_secret(&json, secret),
            "A SECRET IS IN THE ERROR of the command {what}: {json}"
        );
    }
}

fn state_with_two_profiles() -> (AppState, String, String) {
    let s = state();

    let by_password = servers_api::server_add(&s, valid_input("By password"), PASSWORD)
        .expect("the password profile was not created");

    let mut key_input = valid_input("By key");
    key_input.auth_kind = AuthKind::Key;
    key_input.key_path = Some(String::from("/home/user/.ssh/id_ed25519"));
    key_input.host = String::from("127.0.0.1");
    key_input.port = 1;
    let by_key = servers_api::server_add(&s, key_input, PASSPHRASE)
        .expect("the key profile was not created");

    (s, by_password, by_key)
}

#[test]
fn the_profile_list_holds_no_secrets() {
    let (s, _, _) = state_with_two_profiles();
    let list = servers_api::servers_list(&s).expect("the list was not handed back");

    assert_eq!(
        list.len(),
        2,
        "the test is built wrong: there are not two profiles"
    );
    assert_clean("servers_list", &list);
}

#[tokio::test]
async fn the_other_reading_commands_hold_no_secrets() {
    let (s, _, _) = state_with_two_profiles();

    assert_clean("app_versions", &api::app_versions(&s, None).await.unwrap());
    assert_clean("tasks_list", &api::tasks_list(&s).unwrap());
    assert_clean("tasks_on_close", &api::tasks_on_close(&s).unwrap());
}

#[test]
fn the_links_hold_no_secrets() {
    let (s, by_password, _) = state_with_two_profiles();
    let links = vrcast_studio_lib::commands::library::api::links_for(&s, &by_password, "a.mp4")
        .expect("the links were not built");
    assert_clean("links_for", &links);
}

#[tokio::test]
async fn a_failed_connection_check_does_not_carry_a_secret_in_its_details() {
    // The likeliest path for a leak: a detail arrives from somebody else's library, which
    // knows nothing of our rules, and settles in the text of an error. The port is closed —
    // so the steps will fail and the details will be real ones.
    let (s, _, by_key) = state_with_two_profiles();

    match servers_api::server_test(&s, &by_key).await {
        Ok(steps) => {
            assert!(
                steps.iter().any(|x| x.detail.is_some()),
                "the test is built wrong: not one detail, nothing to search"
            );
            assert_clean("server_test", &steps);
        }
        Err(e) => assert_error_clean("server_test", &e),
    }
}

#[tokio::test]
async fn errors_from_the_library_commands_carry_no_secret() {
    let (s, by_password, _) = state_with_two_profiles();

    if let Err(e) =
        vrcast_studio_lib::commands::library::api::library_list(&s, &by_password, true).await
    {
        assert_error_clean("library_list", &e);
    }
}

#[test]
fn changing_a_profile_does_not_hand_the_secret_back() {
    // The other side of the rule: the interface passes a secret in but never gets it back —
    // not even from the very command it just passed it to.
    let (s, by_password, _) = state_with_two_profiles();

    let result = servers_api::server_update(&s, &by_password, valid_input("By password"), None);
    match result {
        Ok(nothing) => assert_clean("server_update", &nothing),
        Err(e) => assert_error_clean("server_update", &e),
    }

    assert_clean(
        "servers_list after the change",
        &servers_api::servers_list(&s).unwrap(),
    );
}

#[test]
fn the_search_itself_can_find_a_secret() {
    // A test of the test. A search that by construction finds nothing would give calm
    // instead of a check — which is worse than no check at all.
    let planted = serde_json::json!({ "field": PASSWORD });
    let json = serde_json::to_string(&planted).unwrap();
    assert!(
        contains_secret(&json, PASSWORD),
        "the search does not find a secret put straight into the answer: {json}"
    );
}
