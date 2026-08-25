//! T082 — contract tests for the upload commands.
//!
//! The contract: `contracts/ipc-commands.md`, the "Upload" section.
//!
//! What is checked here is only what shows from outside: the shape of the answer, the
//! refusal codes, and which of them are lifted by confirming and which are not. The
//! transfer itself is checked against a throwaway server in a container — the contract has
//! nothing to do with it.
//!
//! Every check goes down a path that breaks off **before** the server is reached: a
//! contract test must not depend on whether a network is at hand.

use super::support::{state, valid_input};
use vrcast_studio_lib::commands::error::{DetailCode, ErrorCode};
use vrcast_studio_lib::commands::servers::api as servers;
use vrcast_studio_lib::commands::upload::{
    api as upload, space_error, warning_error, Preflight, SpaceShortage, UploadRequest,
};
use vrcast_studio_lib::commands::AppState;

/// A state with a profile already set up.
///
/// The profile has to be a real one: an upload looks for it first of all, and without one
/// any request is rejected before the rest is even examined. A test checking a refusal over
/// a file name on a server that does not exist would pass while checking nothing.
fn state_with_server() -> (AppState, String) {
    let state = state();
    let id = servers::server_add(&state, valid_input("Server"), "password")
        .expect("the profile would not set up");
    (state, id)
}

/// A request that is certainly fit. Each test changes what it checks.
fn request(server_id: &str, local_path: &str) -> UploadRequest {
    UploadRequest {
        server_id: String::from(server_id),
        local_path: String::from(local_path),
        remote_name: String::from("film_22.mp4"),
        media_id: None,
        limit_bps: None,
        confirmed: false,
    }
}

fn temp_file(name: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("vrcast-contract-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).expect("could not create the temporary directory");
    let path = dir.join(name);
    std::fs::write(&path, "not a video, but a file").expect("could not write the file");
    path
}

#[tokio::test]
async fn an_upload_to_a_server_that_does_not_exist_is_rejected_as_bad_input() {
    let state = state();
    let file = temp_file("film.mp4");

    let err = upload::upload_start(&state, request("no-such-server", &file.to_string_lossy()))
        .await
        .expect_err("an upload to a server that does not exist went through");

    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert!(
        err.says(DetailCode::ProfileNotFound),
        "it does not say the trouble is the server: {err}"
    );
}

#[tokio::test]
async fn a_missing_file_is_named_apart_from_faults() {
    // A typo in a path is not a failure, and the interface must highlight the field rather
    // than show an error notification. The only way to tell one from the other is the code.
    let (state, id) = state_with_server();

    let err = upload::upload_start(&state, request(&id, "F:/no/such/file.mp4"))
        .await
        .expect_err("an upload of a file that does not exist went through");

    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert!(
        err.says(DetailCode::UploadFileUnreadable),
        "it does not say the trouble is the file: {err}"
    );
}

#[tokio::test]
async fn a_directory_instead_of_a_file_is_rejected() {
    let (state, id) = state_with_server();
    let dir = std::env::temp_dir().join(format!("vrcast-dir-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).expect("could not create the directory");

    let err = upload::upload_start(&state, request(&id, &dir.to_string_lossy()))
        .await
        .expect_err("a directory was taken for a file");

    assert_eq!(err.code, ErrorCode::InvalidInput);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn an_empty_serving_name_is_rejected_before_connecting() {
    // A transfer cannot start under an empty name: the file could then be found neither by
    // link nor by eye. The check must fire before connecting — otherwise the refusal comes
    // after a network timeout, and the profile here deliberately points nowhere.
    let (state, id) = state_with_server();
    let file = temp_file("film.mp4");

    let mut req = request(&id, &file.to_string_lossy());
    // Nothing but spaces: the field looks filled, and after trimming there is nothing in it.
    req.remote_name = String::from("   ");

    let started = std::time::Instant::now();
    let err = upload::upload_start(&state, req)
        .await
        .expect_err("an empty name was accepted");

    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "the refusal over the name came after {:?} — so the network was reached first",
        started.elapsed()
    );
}

#[test]
fn a_name_with_directory_traversal_stays_a_name() {
    // The name comes from an input field and goes into a path on the server. Separators are
    // not rejected but replaced: refusing would be more correct in form, but a person
    // simply typed a file name off their disk — while escaping the serving directory it must
    // not do in any spelling.
    use vrcast_studio_lib::domain::remote_name::sanitize;

    for attempt in ["../../etc/passwd", "..\\..\\windows\\system32", "a/b/c.mp4"] {
        let clean = sanitize(attempt);
        assert!(
            !clean.contains('/') && !clean.contains('\\'),
            "directory traversal is still in the name \"{clean}\""
        );
    }

    // And a newline: the name goes into a server command, where a second line would become
    // a command of its own.
    let clean = sanitize("film.mp4\nrm -rf /");
    assert!(
        !clean.contains('\n'),
        "a newline is still in the name: {clean}"
    );
}

#[test]
fn carrying_on_a_task_that_does_not_exist_does_not_stay_quiet() {
    let state = state();
    let err = upload::upload_resume(&state, "no-such-task")
        .expect_err("carrying on a task that does not exist went through quietly");
    // The code must be recognisable: it is what the interface finds its wording by.
    assert_eq!(err.code, ErrorCode::TaskNotFound);
}

// ---------- refusals before the transfer starts ----------

#[test]
fn too_little_room_is_not_lifted_by_confirming() {
    // The difference between a bar and a warning is not a shade of politeness. Show too
    // little room as a warning and a person gets an "upload anyway" button, after which the
    // transfer runs into the end of the disk halfway through thirty gigabytes.
    let checks = Preflight {
        not_enough_space: Some(SpaceShortage {
            needed: 32 * 1024 * 1024 * 1024,
            free: 10 * 1024 * 1024 * 1024,
            short_by: 22 * 1024 * 1024 * 1024,
        }),
        active_connections: 0,
        name_exists: false,
        cdn_cached: false,
    };

    assert!(
        checks.is_blocking(),
        "too little room was declared liftable"
    );

    let err = space_error(checks.not_enough_space.unwrap());
    assert_eq!(err.code, ErrorCode::RemoteDiskFull);

    // The numbers travel as numbers rather than as a formatted size: the units and the
    // decimal separator differ between languages, and choosing them is the interface's
    // business. The core answers for all three numbers being named — without them there is
    // nothing to confirm.
    let detail = err
        .details
        .iter()
        .find(|d| d.key == DetailCode::NotEnoughSpace)
        .unwrap_or_else(|| panic!("the refusal does not name the shortage: {err}"));
    for (name, expected) in [
        ("short_by", 23_622_320_128_u64),
        ("needed", 34_359_738_368),
        ("free", 10_737_418_240),
    ] {
        assert_eq!(
            detail.params.get(name).and_then(|v| v.as_u64()),
            Some(expected),
            "the refusal holds no value for \"{name}\": {detail:?}"
        );
    }
}

#[test]
fn playback_under_way_is_named_by_its_own_code_and_consequence() {
    let checks = Preflight {
        not_enough_space: None,
        active_connections: 3,
        name_exists: false,
        cdn_cached: false,
    };

    assert!(checks.has_warnings());
    assert!(!checks.is_blocking(), "a warning was declared a bar");

    let err = warning_error(&checks, "film_22.mp4");
    assert_eq!(err.code, ErrorCode::ViewersActive);
    let detail = err
        .details
        .iter()
        .find(|d| d.key == DetailCode::ViewersActiveUpload)
        .unwrap_or_else(|| panic!("it does not say playback is under way: {err}"));
    assert_eq!(
        detail.params.get("connections").and_then(|v| v.as_u64()),
        Some(3),
        "it does not say how many connections are open: {detail:?}"
    );
}

#[test]
fn a_name_already_taken_is_named_by_its_own_code() {
    let checks = Preflight {
        not_enough_space: None,
        active_connections: 0,
        name_exists: true,
        cdn_cached: false,
    };

    let err = warning_error(&checks, "film_22.mp4");
    assert_eq!(err.code, ErrorCode::NameExists);
    let detail = err
        .details
        .iter()
        .find(|d| d.key == DetailCode::NameWillBeReplaced)
        .unwrap_or_else(|| panic!("it does not say the file will be replaced: {err}"));
    assert_eq!(
        detail.params.get("name").and_then(|v| v.as_str()),
        Some("film_22.mp4"),
        "it does not say which file exactly: {detail:?}"
    );
    assert!(
        !err.says(DetailCode::CdnKeepsOldCopy),
        "the CDN cache is mentioned where no CDN is set: {err}"
    );
}

#[test]
fn with_a_cache_set_a_replacement_warns_about_that_too() {
    // Otherwise a person replaces the file, opens the link, sees the old video and decides
    // the upload did not work.
    let checks = Preflight {
        not_enough_space: None,
        active_connections: 0,
        name_exists: true,
        cdn_cached: true,
    };

    let err = warning_error(&checks, "film_22.mp4");
    assert_eq!(err.code, ErrorCode::NameExists);
    assert!(
        err.says(DetailCode::CdnKeepsOldCopy),
        "the cached copy is not mentioned: {err}"
    );
}

#[test]
fn when_there_is_nothing_to_warn_about_there_is_no_refusal() {
    let checks = Preflight {
        not_enough_space: None,
        active_connections: 0,
        name_exists: false,
        cdn_cached: false,
    };
    assert!(!checks.has_warnings());
    assert!(!checks.is_blocking());
}

#[test]
fn the_pre_start_checks_survive_writing_and_reading() {
    // They go to the interface as they stand — so they must carry across without loss.
    let checks = Preflight {
        not_enough_space: Some(SpaceShortage {
            needed: 100,
            free: 40,
            short_by: 60,
        }),
        active_connections: 2,
        name_exists: true,
        cdn_cached: true,
    };
    let json = serde_json::to_string(&checks).expect("it would not write");
    let back: Preflight = serde_json::from_str(&json).expect("it would not read");
    assert_eq!(back, checks);
}
