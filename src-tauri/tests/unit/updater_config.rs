//! T351 — the update settings parse the way the application will parse them.
//!
//! **Why this is worth a test at all.** Tauri reads a plugin's settings when the plugin is
//! registered, at startup, before any window opens. The updater's `pubkey` has no default, so a
//! section that is missing a field — or misspells one — does not produce an application without
//! updates: it produces an application that will not open, with "Error deserializing
//! 'plugins.updater'" and nothing on screen at all. Read out of `tauri/src/plugin.rs` (2.11.5)
//! on 2026-08-28, and the reason `lib.rs` registers the plugin only when the section is there.
//!
//! The same deserialization is done here, against the same type, from the same file. It costs
//! milliseconds and it stands between a typo and a build nobody can start.
//!
//! **And the two things around it that fail quietly rather than loudly:**
//!
//! - `createUpdaterArtifacts` — without it the bundler makes installers and no `.sig` files.
//!   Nothing fails; the release simply cannot be assembled, and the reason is a boolean four
//!   hundred lines away from the error.
//! - `https` on the endpoints — the plugin refuses a plain-http endpoint in release builds and
//!   only warns in debug ones. So a development run would look perfectly healthy and the
//!   released application would refuse to check for updates at all.

use serde_json::Value;

/// The very file the bundler and the application read.
const CONF: &str = include_str!("../../tauri.conf.json");

fn config() -> Value {
    serde_json::from_str(CONF).expect("tauri.conf.json is not valid JSON")
}

#[test]
fn the_update_settings_are_the_ones_the_plugin_expects() {
    let conf = config();
    let section = conf
        .pointer("/plugins/updater")
        .expect("there is no plugins.updater section — the application would still build and run, and never update");

    // The plugin's own type, and its own deserialization: not a hand-written check of the
    // fields, which would agree with itself and with nothing else.
    let parsed: tauri_plugin_updater::Config = serde_json::from_value(section.clone())
        .expect("the update settings do not parse — the application would not open at all");

    assert!(
        !parsed.pubkey.is_empty(),
        "the public key is empty: every update would be refused after being downloaded"
    );
    assert!(
        !parsed.endpoints.is_empty(),
        "there is nowhere to look for updates"
    );
}

#[test]
fn the_endpoints_are_all_https() {
    let conf = config();
    let parsed: tauri_plugin_updater::Config =
        serde_json::from_value(conf.pointer("/plugins/updater").unwrap().clone()).unwrap();

    for endpoint in &parsed.endpoints {
        assert_eq!(
            endpoint.scheme(),
            "https",
            "{endpoint} is not https. The plugin only warns about that in a development build \
             and refuses outright in a released one, so this is exactly the kind of fault that \
             passes every check here and reaches the person who was handed the application"
        );
    }
}

#[test]
fn the_signatures_are_asked_for() {
    let conf = config();
    assert_eq!(
        conf.pointer("/bundle/createUpdaterArtifacts"),
        Some(&Value::Bool(true)),
        "without this the bundler makes the installers and no .sig files beside them. Nothing \
         fails at the time; the release simply cannot be put together, and the cause is a \
         boolean a long way from the complaint"
    );
}
