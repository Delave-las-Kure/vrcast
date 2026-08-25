//! T036 — contract tests for the library commands.
//!
//! What is checked here is what can be settled **without a server**: parsing the arguments,
//! the refusal codes and building the links. Everything that needs a real catalogue — a
//! short name already taken, demanding confirmation with the number of files and the
//! volume, a divergence of generations — is checked against a real OpenSSH in
//! `tests/integration/library_ops.rs` and `manifest_conflict.rs`.
//!
//! The split is not a formality: a contract test that slipped an invented catalogue to a
//! command would check the code's agreement with that invention rather than with what lies
//! on the server.

use super::support::{state, valid_input};
use vrcast_studio_lib::commands::error::{DetailCode, ErrorCode};
use vrcast_studio_lib::commands::library::{api, FileView, LibraryView, MediaView};
use vrcast_studio_lib::commands::servers::api as servers_api;

const SECRET: &str = "server-password-for-the-test-9f3a";

fn state_with_server() -> (vrcast_studio_lib::commands::AppState, String) {
    let s = state();
    let mut input = valid_input("Server");
    input.domain = String::from("stream.example.com");
    let id = servers_api::server_add(&s, input, SECRET).expect("the profile was not created");
    (s, id)
}

// ---------- links ----------

#[test]
fn a_file_s_links_are_built_from_the_profile() {
    // FR-016. The domain comes from a person's profile — the application has none and
    // cannot have one (FR-004).
    let (s, id) = state_with_server();

    let links = api::links_for(&s, &id, "Backrooms_22.mp4").expect("the links were not built");
    assert_eq!(
        links.origin,
        "https://stream.example.com/videos/Backrooms_22.mp4"
    );
    assert_eq!(
        links.cdn, None,
        "no CDN was set, yet a second link appeared"
    );
}

#[test]
fn with_a_cdn_set_both_links_are_handed_back() {
    let s = state();
    let mut input = valid_input("With a middleman");
    input.cdn_base = Some(String::from("https://cdn.example.net"));
    let id = servers_api::server_add(&s, input, SECRET).unwrap();

    let links = api::links_for(&s, &id, "a.mp4").unwrap();
    assert_eq!(links.origin, "https://stream.example.com/videos/a.mp4");
    assert_eq!(
        links.cdn.as_deref(),
        Some("https://cdn.example.net/videos/a.mp4")
    );
}

#[test]
fn links_for_a_server_that_does_not_exist_are_an_error() {
    let s = state();
    let err = api::links_for(&s, "no-such-server", "a.mp4")
        .expect_err("links into nothing were handed out");
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

// ---------- argument checks that need no server ----------

#[tokio::test]
async fn a_medium_with_a_disallowed_short_name_is_not_created() {
    // The check happens before the server is reached: there is no point going to the
    // network to reject what is rejected by its shape.
    let (s, id) = state_with_server();

    let err = api::media_create(&s, &id, "Title", Some("name with a space"))
        .await
        .expect_err("a medium with a space in its short name was created");
    assert_eq!(err.code, ErrorCode::InvalidInput);
    // The refusal names WHAT exactly is wrong with the name rather than only that it will
    // not do: the hint the interface takes from the code, but only the core knows about the
    // space.
    assert!(
        err.says(DetailCode::SlugBadChar),
        "the refusal does not name the disallowed character: {err}"
    );
}

#[tokio::test]
async fn a_medium_with_an_empty_title_is_not_created() {
    let (s, id) = state_with_server();
    let err = api::media_create(&s, &id, "   ", None)
        .await
        .expect_err("a medium with no title was created");
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

#[tokio::test]
async fn a_title_with_no_latin_counterpart_asks_a_person_for_the_short_name() {
    // The application does not invent a short name out of rubbish: it goes into the file
    // name and into the link, and it would be too late to put right.
    let (s, id) = state_with_server();
    let err = api::media_create(&s, &id, "日本語", None)
        .await
        .expect_err("a short name was invented out of nowhere");
    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert!(
        err.says(DetailCode::SlugUnmakeable),
        "the refusal does not explain that the short name has to be set by hand: {err}"
    );
}

#[tokio::test]
async fn a_rename_with_not_one_new_value_is_rejected() {
    // A call that changes nothing yet writes the catalogue is a needless generation and a
    // needless chance to diverge from another copy of the application.
    let (s, id) = state_with_server();
    let err = api::media_rename(&s, &id, "m1", None, None)
        .await
        .expect_err("a rename into nothing was accepted");
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

#[tokio::test]
async fn the_library_commands_refuse_for_a_server_that_does_not_exist() {
    let s = state();
    for err in [
        api::library_list(&s, "no-such-server", false).await.err(),
        api::media_create(&s, "no-such-server", "Title", None)
            .await
            .err(),
        api::media_delete(&s, "no-such-server", "m1", true)
            .await
            .err(),
        api::file_delete(&s, "no-such-server", "a.mp4", true)
            .await
            .err(),
    ] {
        let err = err.expect("the command worked on a server that does not exist");
        assert_eq!(err.code, ErrorCode::InvalidInput, "the wrong code: {err:?}");
    }
}

// ---------- the shape of the answer ----------

#[test]
fn a_library_s_completeness_is_counted_over_every_visible_file() {
    // The property the "not recognised" group exists for in the first place (FR-015): the
    // number of files a person can see must equal the number of files in the directory. A
    // file that landed neither in a medium nor in that group is a lost file.
    let view = LibraryView {
        server_id: String::from("srv"),
        media: vec![MediaView {
            id: String::from("m1"),
            title: String::from("Film"),
            slug: String::from("film"),
            files: vec![file_view("film_22.mp4"), file_view("film_10.mp4")],
            ladders: vec![String::from("film/master.m3u8")],
            total_bytes: 2048,
            created_at: String::from("2026-08-01T10:00:00Z"),
        }],
        unrecognized: vec![file_view("unclear.mp4")],
        disk: None,
        stale: false,
    };

    // Two files of the medium, one quality ladder, one unrecognised.
    assert_eq!(view.accounted_entries(), 4);
}

#[test]
fn the_library_s_answer_survives_the_crossing() {
    // The contract crosses the boundary between the core and the interface as JSON. A type
    // that does not survive the round trip is a contract that will lose data somewhere.
    let view = LibraryView {
        server_id: String::from("srv"),
        media: Vec::new(),
        unrecognized: vec![file_view("lonely.mp4")],
        disk: Some(vrcast_studio_lib::commands::library::DiskUsage {
            total_bytes: 100,
            free_bytes: 40,
            used_by_videos_bytes: 55,
        }),
        stale: true,
    };

    let json = serde_json::to_string(&view).expect("the answer will not serialise");
    let back: LibraryView = serde_json::from_str(&json).expect("the answer will not read back");
    assert_eq!(back, view);
    assert!(
        json.contains("\"stale\":true"),
        "the stale-data mark was lost: {json}"
    );
}

fn file_view(path: &str) -> FileView {
    FileView {
        path: path.to_owned(),
        size_bytes: 1024,
        duration_s: None,
        width: None,
        height: None,
        bitrate_bps: None,
        video_codec: None,
        audio_codec: None,
        faststart_ok: None,
        exists_on_server: true,
        origin_url: format!("https://stream.example.com/videos/{path}"),
        cdn_url: None,
    }
}
