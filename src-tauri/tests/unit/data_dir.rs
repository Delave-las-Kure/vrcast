//! The e2e build's data directory and secrets — and that no other build has them.
//!
//! **Why a guard at all.** On Windows `ProjectDirs` asks the system for the known folder and
//! ignores the environment, and secrets go to the Credential Manager. A harness running the
//! real binary on the owner's machine therefore opened the owner's REAL profiles once already.
//! The `e2e` feature moves both somewhere disposable; these checks hold the other half: that an
//! ordinary build does not read the variable at all, and that nothing that makes a release
//! turns the feature on.

use std::ffi::OsString;
use std::path::PathBuf;

use vrcast_studio_lib::store::data_dir::{self, E2E, ENV};

/// A path that is absolute on whichever platform runs the test.
fn absolute(name: &str) -> PathBuf {
    std::env::temp_dir().join(name)
}

#[test]
fn the_e2e_rule_takes_only_an_absolute_path() {
    let abs = absolute("vrcast-e2e-data");
    assert_eq!(data_dir::from_env(Some(abs.clone().into())), Some(abs));
    assert_eq!(data_dir::from_env(None), None);
    assert_eq!(data_dir::from_env(Some(OsString::new())), None);
    // A relative path would land wherever the driver happened to start the binary.
    assert_eq!(data_dir::from_env(Some("relative/dir".into())), None);
}

/// The one test in this binary that sets the variable. With the feature, it is the answer;
/// without it, it changes nothing — the answer is the system's, byte for byte.
#[test]
fn the_variable_is_read_only_by_the_e2e_build() {
    let abs = absolute("vrcast-e2e-guard");
    std::env::set_var(ENV, &abs);
    let root = data_dir::root();
    let db = vrcast_studio_lib::store::db::Db::default_path().ok();
    let geo = vrcast_studio_lib::store::geo::dir();
    let logs = vrcast_studio_lib::logging::default_dir();
    std::env::remove_var(ENV);

    if E2E {
        assert_eq!(root.as_deref(), Some(abs.as_path()));
        assert_eq!(db, Some(abs.join("vrcast-studio.sqlite")));
        assert_eq!(geo.as_deref(), Some(abs.as_path()));
        assert_eq!(logs, Some(abs.join("logs")));
        // And without the variable the e2e build has NO directory — it does not fall back
        // to the person's real one.
        assert_eq!(data_dir::root(), None);
    } else {
        let system = data_dir::system();
        assert_eq!(root, system, "an ordinary build read {ENV}");
        assert_ne!(root.as_deref(), Some(abs.as_path()));
        assert_eq!(db, system.as_ref().map(|d| d.join("vrcast-studio.sqlite")));
        assert_eq!(geo, system);
        assert_eq!(logs, system.as_ref().map(|d| d.join("logs")));
    }
}

#[test]
fn an_ordinary_build_names_the_same_directory_as_before() {
    // What `ProjectDirs` was asked before the change, verbatim: the owner's data must be
    // found where it has always been.
    let before = directories::ProjectDirs::from("ru", "VRCast", "VRCast Studio")
        .map(|d| d.data_dir().to_path_buf());
    assert_eq!(data_dir::system(), before);
}

/// Nothing that makes a release may turn the feature on.
#[test]
fn no_release_path_turns_the_feature_on() {
    let manifest = include_str!("../../Cargo.toml");
    let default_line = manifest
        .lines()
        .find(|l| l.trim_start().starts_with("default"))
        .unwrap_or("");
    assert!(
        !default_line.contains("e2e"),
        "the e2e feature is among the defaults: {default_line}"
    );

    for (name, text) in [
        ("tauri.conf.json", include_str!("../../tauri.conf.json")),
        (
            "tauri.release.conf.json",
            include_str!("../../tauri.release.conf.json"),
        ),
        (
            ".github/workflows/release.yml",
            include_str!("../../../.github/workflows/release.yml"),
        ),
        (
            ".github/workflows/build.yml",
            include_str!("../../../.github/workflows/build.yml"),
        ),
        ("package.json", include_str!("../../../package.json")),
    ] {
        for pattern in ["features e2e", "features=e2e", "\"e2e\"", ",e2e", "e2e,"] {
            assert!(
                !text.contains(pattern),
                "{name} turns the e2e feature on ({pattern:?})"
            );
        }
    }
}

#[cfg(feature = "e2e")]
#[test]
fn the_e2e_secrets_live_in_a_file_in_the_given_directory() {
    use vrcast_studio_lib::store::secrets::{FileSecretStore, SecretRef, SecretStore};

    let dir = std::env::temp_dir().join(format!("vrcast-e2e-secrets-{}", uuid::Uuid::new_v4()));
    let store = FileSecretStore::in_dir(&dir);
    let r = SecretRef::for_server("one");
    assert!(store.get(&r).is_err());
    store.set(&r, "test-key-value").unwrap();
    assert!(dir.join(FileSecretStore::FILE_NAME).exists());
    // A second store on the same directory — the application after a restart — finds it.
    assert_eq!(
        FileSecretStore::in_dir(&dir).get(&r).unwrap(),
        "test-key-value"
    );
    store.delete(&r).unwrap();
    assert!(store.get(&r).is_err());
    store.delete(&r).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}
