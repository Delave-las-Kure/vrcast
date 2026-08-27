//! Operations on the library against a real server (T047, T048).
//!
//! Here is what cannot be checked without a catalogue: a short name already taken, demanding
//! confirmation with the number of files and the volume, renaming files to follow a short
//! name, moving a file between media, and deleting.
//!
//! A contract test on an invented catalogue would check the code's agreement with the
//! invention. Here the catalogue is a real one — the one the application wrote to the server
//! itself.

use super::fixture::{key_path, TestServer, KEY_PASSPHRASE};
use std::sync::Arc;
use vrcast_studio_lib::commands::error::{DetailCode, ErrorCode};
use vrcast_studio_lib::commands::library::api as library;
use vrcast_studio_lib::commands::servers::{api as servers, ServerInput};
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::domain::server_profile::AuthKind;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;

const VIDEO_DIR: &str = "/var/lib/vrcast/videos";

fn app_state() -> AppState {
    AppState::with_db(
        Arc::new(Db::open_in_memory().unwrap()),
        Arc::new(InMemorySecretStore::new()),
    )
    .expect("the application state would not assemble")
}

/// Bring the container up, set a profile up and lay the files out.
async fn setup(files: &[&str]) -> (TestServer, AppState, String) {
    let server = TestServer::start().expect("the container would not come up");
    for name in files {
        server
            .exec_inside(&format!("head -c 2048 /dev/urandom > '{VIDEO_DIR}/{name}'"))
            .unwrap_or_else(|e| panic!("could not create {name}: {e}"));
    }

    let state = app_state();
    let input = ServerInput {
        name: String::from("Container"),
        host: server.host().to_owned(),
        port: server.port,
        user: String::from("root"),
        auth_kind: AuthKind::Key,
        key_path: Some(key_path().to_string_lossy().into_owned()),
        domain: String::from("stream.example.com"),
        video_dir: Some(String::from(VIDEO_DIR)),
        cdn_base: None,
        ipv6_mode: None,
    };
    let id =
        servers::server_add(&state, input, KEY_PASSPHRASE).expect("the profile was not created");
    confirm_fingerprint(&state, &id, &server).await;
    (server, state, id)
}

/// Walk the same path a person walks in the setup wizard: learn the fingerprint and confirm
/// it.
///
/// Without this step the application does not connect at all — credentials are never sent to
/// a server whose fingerprint has not been confirmed (FR-092). Skipping it in the fixture
/// would check behaviour a person never reaches.
pub async fn confirm_fingerprint(state: &AppState, server_id: &str, server: &TestServer) {
    let fingerprint =
        vrcast_studio_lib::commands::api::server_probe_fingerprint(server.host(), server.port)
            .await
            .expect("the fingerprint was not obtained");
    servers::server_fingerprint_confirm(state, server_id, &fingerprint)
        .expect("the fingerprint was not confirmed");
}

#[tokio::test]
async fn a_medium_is_created_and_shows_in_the_library() {
    let (_server, state, id) = setup(&[]).await;

    // The title is deliberately not Latin: making a slug out of it is part of what is
    // checked here.
    let media_id = library::media_create(&state, &id, "Название фильма", None)
        .await
        .expect("the medium was not created");

    let view = library::library_list(&state, &id, true).await.unwrap();
    let media = view
        .media
        .iter()
        .find(|m| m.id == media_id)
        .expect("the medium that was created is not visible in the library");

    assert_eq!(media.title, "Название фильма");
    assert_eq!(
        media.slug, "nazvanie-filma",
        "the short name was not made by the rules"
    );
}

#[tokio::test]
async fn a_short_name_already_taken_is_refused_with_its_own_code() {
    // A code of its own is needed so the interface can offer another name rather than show a
    // general message about a fault.
    let (_server, state, id) = setup(&[]).await;
    library::media_create(&state, &id, "The first", Some("film"))
        .await
        .unwrap();

    let err = library::media_create(&state, &id, "The second", Some("film"))
        .await
        .expect_err("a second medium was set up under the same short name");
    assert_eq!(err.code, ErrorCode::SlugTaken);
}

#[tokio::test]
async fn deleting_without_confirmation_names_the_consequences() {
    // FR-014. There is nothing to confirm blind: a person must see how many files will
    // vanish and how much room will be freed.
    let (server, state, id) = setup(&["film_10.mp4", "film_22.mp4"]).await;

    let media_id = library::media_create(&state, &id, "The film", Some("film"))
        .await
        .unwrap();
    // Both files are attributed to the medium.
    for name in ["film_10.mp4", "film_22.mp4"] {
        library::file_move(&state, &id, name, &media_id, true)
            .await
            .expect("the file was not attributed to the medium");
    }

    let err = library::media_delete(&state, &id, &media_id, false)
        .await
        .expect_err("the medium was deleted without confirmation");

    assert_eq!(err.code, ErrorCode::ConfirmationRequired);
    // The refusal names the numbers: without them there is nothing to confirm. They travel
    // as numbers — what to turn them into is the interface's decision.
    let detail = err
        .details
        .iter()
        .find(|d| d.key == DetailCode::ConfirmDelete)
        .unwrap_or_else(|| panic!("the refusal does not name the consequences: {err}"));
    assert_eq!(
        detail.params.get("files").and_then(|v| v.as_u64()),
        Some(2),
        "the refusal does not name the number of files: {detail:?}"
    );
    assert!(
        detail
            .params
            .get("bytes")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            > 0,
        "the refusal does not name the volume: {detail:?}"
    );

    // The main thing: with no confirmation, nothing happened.
    let still_there = server
        .exec_inside(&format!("ls {VIDEO_DIR}/film_10.mp4"))
        .is_ok();
    assert!(
        still_there,
        "the file was deleted although there was no confirmation"
    );
}

#[tokio::test]
async fn a_confirmed_deletion_removes_both_the_files_and_the_catalogue_entry() {
    let (server, state, id) = setup(&["film_10.mp4", "film_22.mp4", "other.mp4"]).await;

    let media_id = library::media_create(&state, &id, "The film", Some("film"))
        .await
        .unwrap();
    for name in ["film_10.mp4", "film_22.mp4"] {
        library::file_move(&state, &id, name, &media_id, true)
            .await
            .unwrap();
    }

    library::media_delete(&state, &id, &media_id, true)
        .await
        .expect("the medium would not delete");

    for name in ["film_10.mp4", "film_22.mp4"] {
        assert!(
            server
                .exec_inside(&format!("test -e {VIDEO_DIR}/{name}"))
                .is_err(),
            "the file {name} was left on the server"
        );
    }
    // The other file is untouched: deleting a medium has no right to affect its neighbours.
    assert!(
        server
            .exec_inside(&format!("test -e '{VIDEO_DIR}/other.mp4'"))
            .is_ok(),
        "deleting the medium affected an unrelated file"
    );

    let view = library::library_list(&state, &id, true).await.unwrap();
    assert!(
        !view.media.iter().any(|m| m.id == media_id),
        "the medium's entry stayed in the catalogue"
    );
    assert_eq!(
        view.unrecognized.len(),
        1,
        "the surviving file was lost: {view:?}"
    );
}

#[tokio::test]
async fn changing_the_short_name_renames_the_files() {
    // And breaks the old links — the interface must warn about that before calling. What is
    // checked is that the renaming really reaches the server: a catalogue pointing at files
    // that do not exist is worse than no renaming at all.
    let (server, state, id) = setup(&["film_10.mp4", "film_22.mp4"]).await;

    let media_id = library::media_create(&state, &id, "The film", Some("film"))
        .await
        .unwrap();
    for name in ["film_10.mp4", "film_22.mp4"] {
        library::file_move(&state, &id, name, &media_id, true)
            .await
            .unwrap();
    }

    library::media_rename(&state, &id, &media_id, None, Some("kino"))
        .await
        .expect("the renaming failed");

    assert!(
        server
            .exec_inside(&format!("test -e {VIDEO_DIR}/kino_10.mp4"))
            .is_ok(),
        "the file was not renamed on the server"
    );
    assert!(
        server
            .exec_inside(&format!("test -e {VIDEO_DIR}/film_10.mp4"))
            .is_err(),
        "the old file was left — a copy appeared on the disk"
    );

    let view = library::library_list(&state, &id, true).await.unwrap();
    let media = view.media.iter().find(|m| m.id == media_id).unwrap();
    assert_eq!(media.slug, "kino");
    let paths: Vec<&str> = media.files.iter().map(|f| f.path.as_str()).collect();
    assert!(
        paths.contains(&"kino_10.mp4") && paths.contains(&"kino_22.mp4"),
        "the catalogue did not keep up with the renaming: {paths:?}"
    );
    assert!(
        media.files.iter().all(|f| f.exists_on_server),
        "the catalogue points at files that do not exist: {media:?}"
    );
    // The links were rebuilt under the new name — otherwise a person would copy an address
    // that no longer exists.
    assert!(
        media.files.iter().all(|f| f.origin_url.contains("kino_")),
        "the links stayed on the old name: {media:?}"
    );
}

#[tokio::test]
async fn renaming_only_the_title_leaves_the_files_alone() {
    // Changing the title is a harmless act, and breaking working links over it would be a
    // surprise to a person.
    let (server, state, id) = setup(&["film_10.mp4"]).await;
    let media_id = library::media_create(&state, &id, "The film", Some("film"))
        .await
        .unwrap();
    library::file_move(&state, &id, "film_10.mp4", &media_id, true)
        .await
        .unwrap();

    library::media_rename(
        &state,
        &id,
        &media_id,
        Some("A completely different title"),
        None,
    )
    .await
    .expect("the renaming failed");

    assert!(
        server
            .exec_inside(&format!("test -e {VIDEO_DIR}/film_10.mp4"))
            .is_ok(),
        "the file was renamed over a change of title alone"
    );
    let view = library::library_list(&state, &id, true).await.unwrap();
    let media = view.media.iter().find(|m| m.id == media_id).unwrap();
    assert_eq!(media.title, "A completely different title");
    assert_eq!(media.slug, "film");
}

#[tokio::test]
async fn deleting_a_file_without_confirmation_does_nothing() {
    let (server, state, id) = setup(&["lonely.mp4"]).await;

    let err = library::file_delete(&state, &id, "lonely.mp4", false)
        .await
        .expect_err("the file was deleted without confirmation");
    assert_eq!(err.code, ErrorCode::ConfirmationRequired);
    assert!(
        server
            .exec_inside(&format!("test -e '{VIDEO_DIR}/lonely.mp4'"))
            .is_ok(),
        "the file vanished with no confirmation"
    );

    library::file_delete(&state, &id, "lonely.mp4", true)
        .await
        .expect("the file would not delete with confirmation");
    assert!(
        server
            .exec_inside(&format!("test -e '{VIDEO_DIR}/lonely.mp4'"))
            .is_err(),
        "the file was left after a confirmed deletion"
    );
}

#[tokio::test]
async fn the_catalogue_cannot_be_deleted_through_a_command() {
    // The catalogue is a housekeeping record that belongs to the application. We do not show
    // it to a person, but we must also guard against a direct call.
    let (_server, state, id) = setup(&[]).await;
    library::media_create(&state, &id, "The film", Some("film"))
        .await
        .unwrap();

    let err = library::file_delete(&state, &id, "library.json", true)
        .await
        .expect_err("the library catalogue was deleted at the interface's request");
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

#[tokio::test]
async fn deleting_a_file_that_does_not_exist_says_so_with_its_own_code() {
    let (_server, state, id) = setup(&[]).await;
    let err = library::file_delete(&state, &id, "no-such-file.mp4", true)
        .await
        .expect_err("a file that does not exist was deleted");
    assert_eq!(err.code, ErrorCode::FileMissingOnServer);
}

#[tokio::test]
async fn a_second_copy_of_the_application_gets_a_refusal_code_of_its_own() {
    // The same catalogue conflict, but seen through the command layer: the interface must
    // get MANIFEST_CONFLICT so it can offer to re-read and try again, rather than a general
    // "internal error".
    let (server, state, id) = setup(&[]).await;
    library::media_create(&state, &id, "The first", Some("pervoe"))
        .await
        .unwrap();

    // A second copy changes the catalogue behind our back — and the generation moves on.
    server
        .exec_inside(&format!(
            "sed -i 's/\"generation\": 1/\"generation\": 99/' {VIDEO_DIR}/library.json"
        ))
        .expect("could not substitute the generation");

    // Our command re-reads the catalogue, so there will be no conflict on it — a conflict is
    // caught when the generation moves BETWEEN the read and the write. That is checked
    // directly, at the writing layer.
    use vrcast_studio_lib::server::manifest_io;
    // Through the door, like everything else. This check is about the catalogue and not about
    // the door, but going round it would leave one call site that says nothing about what it
    // is for — and one is all it takes.
    let conn = vrcast_studio_lib::server::gate::open(
        state.secrets.as_ref(),
        &vrcast_studio_lib::store::profiles::get(&state.db, &id)
            .unwrap()
            .unwrap(),
        vrcast_studio_lib::server::gate::Intent::Change,
    )
    .await
    .expect("could not connect")
    .conn;

    let read = manifest_io::read(&conn, VIDEO_DIR).await.unwrap();
    server
        .exec_inside(&format!(
            "sed -i 's/\"generation\": 99/\"generation\": 100/' {VIDEO_DIR}/library.json"
        ))
        .expect("could not substitute the generation a second time");

    let err = manifest_io::write(
        &conn,
        VIDEO_DIR,
        &read.prepared_for_write(),
        read.generation,
    )
    .await
    .expect_err("the write went through over somebody else's change");

    let app_err = vrcast_studio_lib::commands::error::AppError::from(err);
    // The code is the answer: the hint "refresh the list and try again" the interface takes
    // from its catalogue — one hint for every place this code turns up.
    assert_eq!(app_err.code, ErrorCode::ManifestConflict);
}
