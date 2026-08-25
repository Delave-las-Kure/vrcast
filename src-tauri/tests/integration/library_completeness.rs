//! T039 — the library's completeness: not one file is lost.
//!
//! FR-015. A file the application did not show has not gone anywhere: it takes up room on
//! the disk and goes on being served by its direct link. Hiding it is the worst decision
//! possible, because a person believes their library is complete and wonders where the disk
//! space went.
//!
//! An equality is checked: the number of entries in the serving directory (housekeeping
//! aside) equals the number accounted for — the media's files plus the quality ladders plus
//! the "not recognised" group.

use super::fixture::{key_path, TestServer, KEY_PASSPHRASE};
use std::sync::Arc;
use vrcast_studio_lib::commands::library::api as library_api;
use vrcast_studio_lib::commands::servers::{api as servers_api, ServerInput};
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::domain::server_profile::AuthKind;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;

const VIDEO_DIR: &str = "/var/lib/vrcast/videos";

/// What is put into the serving directory. The names are deliberately varied — with a
/// space, with non-Latin characters, with dots: the application must cope with more than
/// the exemplary ones.
const FILES: [&str; 4] = [
    "Backrooms_10.mp4",
    "Backrooms_22.mp4",
    "одинокий ролик.mp4",
    "Blue.Eye.Samurai.S01E01.mp4",
];

fn app_state() -> AppState {
    AppState::with_db(
        Arc::new(Db::open_in_memory().unwrap()),
        Arc::new(InMemorySecretStore::new()),
    )
    .expect("the application state would not assemble")
}

fn profile_for(server: &TestServer) -> ServerInput {
    ServerInput {
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
    }
}

/// Lay out the files, a quality ladder, the catalogue and the housekeeping directory.
fn prepare(server: &TestServer) {
    for name in FILES {
        server
            .exec_inside(&format!("head -c 4096 /dev/urandom > '{VIDEO_DIR}/{name}'"))
            .unwrap_or_else(|e| panic!("could not create {name}: {e}"));
    }

    // A quality ladder lies as a directory — to a person that is one entry, not a hundred
    // segments.
    server
        .exec_inside(&format!(
            "mkdir -p '{VIDEO_DIR}/backrooms' && \
             printf '#EXTM3U\\n' > '{VIDEO_DIR}/backrooms/master.m3u8' && \
             head -c 1024 /dev/urandom > '{VIDEO_DIR}/backrooms/seg1.ts'"
        ))
        .expect("could not create the quality ladder");

    // The housekeeping directory of trimmed descriptions: it belongs to the application and
    // is not part of the library.
    server
        .exec_inside(&format!("mkdir -p '{VIDEO_DIR}/_slow'"))
        .expect("could not create the housekeeping directory");

    // The catalogue knows about two of the four files and about the quality ladder.
    let manifest = r#"{
      "generation": 3,
      "media": [
        { "id": "m_back", "title": "Backrooms", "slug": "backrooms",
          "files": ["Backrooms_10.mp4", "Backrooms_22.mp4"],
          "ladders": ["backrooms/master.m3u8"],
          "created_at": "2026-08-01T10:00:00Z" }
      ]
    }"#;
    server
        .exec_inside(&format!(
            "cat > '{VIDEO_DIR}/library.json' <<'EOF'\n{manifest}\nEOF"
        ))
        .expect("could not write the catalogue");
}

#[tokio::test]
async fn not_one_file_of_the_directory_is_lost_in_the_library() {
    let server = TestServer::start().expect("the container would not come up");
    prepare(&server);

    let state = app_state();
    let server_id =
        servers_api::server_add(&state, profile_for(&server), KEY_PASSPHRASE).expect("no profile");
    super::library_ops::confirm_fingerprint(&state, &server_id, &server).await;

    let view = library_api::library_list(&state, &server_id, true)
        .await
        .expect("the library would not read");

    // Counted by the server's own means: checking a number with the same code that produced
    // it is not checking anything.
    let counted = server
        .exec_inside(&format!(
            "ls -A '{VIDEO_DIR}' | grep -v '^library.json$' | grep -v '^_slow$' | wc -l"
        ))
        .expect("could not count the directory's entries");
    let expected: usize = counted.trim().parse().expect("the number would not parse");

    assert_eq!(
        expected,
        FILES.len() + 1,
        "the test is built wrong: the directory holds something other than expected"
    );
    assert_eq!(
        view.accounted_entries(),
        expected,
        "some of the directory's entries were not shown to the person: {view:?}"
    );
}

#[tokio::test]
async fn the_unrecognised_files_are_shown_as_a_group_of_their_own() {
    let server = TestServer::start().expect("the container would not come up");
    prepare(&server);

    let state = app_state();
    let server_id =
        servers_api::server_add(&state, profile_for(&server), KEY_PASSPHRASE).expect("no profile");
    super::library_ops::confirm_fingerprint(&state, &server_id, &server).await;
    let view = library_api::library_list(&state, &server_id, true)
        .await
        .expect("the library would not read");

    let unrecognized: Vec<&str> = view.unrecognized.iter().map(|f| f.path.as_str()).collect();
    assert!(
        unrecognized.contains(&"одинокий ролик.mp4"),
        "the file with a space in its name was lost: {unrecognized:?}"
    );
    assert!(
        unrecognized.contains(&"Blue.Eye.Samurai.S01E01.mp4"),
        "a file outside the catalogue was not shown: {unrecognized:?}"
    );
    assert_eq!(
        unrecognized.len(),
        2,
        "something surplus is in the group: {unrecognized:?}"
    );
}

#[tokio::test]
async fn the_catalogue_and_the_housekeeping_directories_are_not_shown_as_video() {
    // Otherwise a person sees "library.json" and "_slow" in their library and thinks those
    // are their files.
    let server = TestServer::start().expect("the container would not come up");
    prepare(&server);

    let state = app_state();
    let server_id =
        servers_api::server_add(&state, profile_for(&server), KEY_PASSPHRASE).expect("no profile");
    super::library_ops::confirm_fingerprint(&state, &server_id, &server).await;
    let view = library_api::library_list(&state, &server_id, true)
        .await
        .expect("the library would not read");

    let all: Vec<&str> = view
        .media
        .iter()
        .flat_map(|m| m.files.iter())
        .chain(view.unrecognized.iter())
        .map(|f| f.path.as_str())
        .collect();

    for housekeeping in ["library.json", "_slow"] {
        assert!(
            !all.iter().any(|p| p.starts_with(housekeeping)),
            "the housekeeping entry \"{housekeeping}\" was shown as video: {all:?}"
        );
    }
}

#[tokio::test]
async fn a_catalogued_file_missing_from_the_server_is_marked_as_gone() {
    // FR-018: the file was deleted outside the application. The catalogue still remembers it
    // — and a link to it must not be shown as working.
    let server = TestServer::start().expect("the container would not come up");
    prepare(&server);
    server
        .exec_inside(&format!("rm '{VIDEO_DIR}/Backrooms_10.mp4'"))
        .expect("the file would not delete");

    let state = app_state();
    let server_id =
        servers_api::server_add(&state, profile_for(&server), KEY_PASSPHRASE).expect("no profile");
    super::library_ops::confirm_fingerprint(&state, &server_id, &server).await;
    let view = library_api::library_list(&state, &server_id, true)
        .await
        .expect("the library would not read");

    let media = view
        .media
        .iter()
        .find(|m| m.slug == "backrooms")
        .expect("the medium vanished along with the file");
    let missing = media
        .files
        .iter()
        .find(|f| f.path == "Backrooms_10.mp4")
        .expect("the missing file vanished from the medium — a person will not learn of the loss");

    assert!(
        !missing.exists_on_server,
        "a deleted file counts as existing"
    );
    let present = media
        .files
        .iter()
        .find(|f| f.path == "Backrooms_22.mp4")
        .expect("the surviving file went missing");
    assert!(present.exists_on_server);
}

#[tokio::test]
async fn a_file_s_parameters_are_read_from_the_header_rather_than_invented() {
    // FR-012 and R-19: the resolution, the duration and the codecs come from parsing the
    // beginning of a file. Our blanks made of random bytes have no header — and the
    // application must say "unknown" honestly rather than put in plausible numbers.
    let server = TestServer::start().expect("the container would not come up");
    prepare(&server);

    let state = app_state();
    let server_id =
        servers_api::server_add(&state, profile_for(&server), KEY_PASSPHRASE).expect("no profile");
    super::library_ops::confirm_fingerprint(&state, &server_id, &server).await;
    let view = library_api::library_list(&state, &server_id, true)
        .await
        .expect("the library would not read");

    let any = view
        .media
        .iter()
        .flat_map(|m| m.files.iter())
        .chain(view.unrecognized.iter())
        .next()
        .expect("the library holds not one file");

    assert!(any.size_bytes > 0, "the file's size was not read");
    assert_eq!(
        any.width, None,
        "the resolution came out of nowhere: this file has no header"
    );
    assert_eq!(any.duration_s, None, "the duration was invented");
}

#[tokio::test]
async fn the_room_on_the_server_s_disk_is_shown() {
    // FR-017.
    let server = TestServer::start().expect("the container would not come up");
    prepare(&server);

    let state = app_state();
    let server_id =
        servers_api::server_add(&state, profile_for(&server), KEY_PASSPHRASE).expect("no profile");
    super::library_ops::confirm_fingerprint(&state, &server_id, &server).await;
    let view = library_api::library_list(&state, &server_id, true)
        .await
        .expect("the library would not read");

    let disk = view.disk.expect("the room on the disk was not shown");
    assert!(disk.total_bytes > 0, "the disk's size was not read");
    assert!(
        disk.free_bytes <= disk.total_bytes,
        "more is free than there is in total: {disk:?}"
    );
    assert!(
        disk.used_by_videos_bytes > 0,
        "the serving directory's size was not counted although there are files in it"
    );
}

#[tokio::test]
async fn with_the_server_unreachable_the_last_known_state_is_shown_with_a_mark() {
    // An empty screen or an endless loading spinner is the worst answer: a person cannot
    // tell whether they lost their library or their connection.
    let server = TestServer::start().expect("the container would not come up");
    prepare(&server);

    let state = app_state();
    let server_id =
        servers_api::server_add(&state, profile_for(&server), KEY_PASSPHRASE).expect("no profile");
    super::library_ops::confirm_fingerprint(&state, &server_id, &server).await;

    let fresh = library_api::library_list(&state, &server_id, true)
        .await
        .expect("the library would not read");
    assert!(!fresh.stale, "fresh data was marked stale");

    // The server is dropped and the question asked again.
    drop(server);
    let after = library_api::library_list(&state, &server_id, true)
        .await
        .expect(
            "with the server unreachable the library must come from the cache, not as an error",
        );

    assert!(after.stale, "stale data was not marked");
    assert_eq!(
        after.accounted_entries(),
        fresh.accounted_entries(),
        "the cache lost some of the entries"
    );
}
