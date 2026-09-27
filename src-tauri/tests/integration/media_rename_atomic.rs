//! T620 — `media_rename` moves the files and writes the catalogue as ONE step on the server.
//!
//! **The bug (QA-19 №5).** The rename read the catalogue at generation g, moved the files one
//! `mv` at a time, and only then wrote the catalogue with T604's compare-and-swap. A catalogue
//! somebody changed in between was refused as `MANIFEST_CONFLICT` — correctly — but the files
//! were already under their new names and nothing moved them back: the catalogue pointed at
//! names that were gone, and "read again and retry" (ipc-commands.md) looked for them.
//!
//! **The decision (owner, 2026-09-26).** Check the generation, move, write — one script under
//! the catalogue's lock (the same one T604 put on every catalogue write). A conflict is found
//! before anything is moved; a move that fails in the middle moves back what was moved.
//!
//! The window between the rename's read and its write is held open on purpose, by a `flock`
//! in front of the real one that waits for the test to say "go" the first time it is called
//! — the call the rename's own script makes. The other client's change goes in through the
//! real `media_create`, in that window, before the rename's script even has the lock.

use std::sync::Arc;
use std::time::Duration;

use vrcast_studio_lib::commands::error::{DetailCode, ErrorCode};
use vrcast_studio_lib::commands::library::api as library;
use vrcast_studio_lib::commands::servers::{api as servers, ServerInput};
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::domain::server_profile::AuthKind;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;

use super::fixture::{key_path, TestServer, KEY_PASSPHRASE};
use super::library_ops::confirm_fingerprint;

const VIDEO_DIR: &str = "/var/lib/vrcast/videos";

fn app_state() -> AppState {
    AppState::with_db(
        Arc::new(Db::open_in_memory().unwrap()),
        Arc::new(InMemorySecretStore::new()),
    )
    .expect("the application state would not assemble")
}

/// The container, a profile, and a medium `film` owning `film_10.mp4`, `film_22.mp4` and a
/// ladder directory `film/` — three top-level entries a rename to `kino` moves.
async fn setup() -> (TestServer, AppState, String, String) {
    let server = TestServer::start().expect("the container would not come up");
    server
        .exec_inside(&format!(
            "head -c 2048 /dev/urandom > {VIDEO_DIR}/film_10.mp4 && \
             head -c 2048 /dev/urandom > {VIDEO_DIR}/film_22.mp4 && \
             mkdir -p {VIDEO_DIR}/film/v6 && \
             printf '#EXTM3U\\n' > {VIDEO_DIR}/film/master.m3u8 && \
             printf '#EXTM3U\\n' > {VIDEO_DIR}/film/v6/stream.m3u8"
        ))
        .expect("the files would not be laid out");
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

    let media_id = library::media_create(&state, &id, "The film", Some("film"))
        .await
        .expect("the medium was not created");
    for name in ["film_10.mp4", "film_22.mp4", "film/master.m3u8"] {
        library::file_move(&state, &id, name, &media_id, true)
            .await
            .unwrap_or_else(|e| panic!("{name} was not attributed to the medium: {e}"));
    }
    (server, state, id, media_id)
}

/// What is in the serving directory, recursively, sorted — read around our own code.
fn tree(server: &TestServer) -> String {
    server
        .exec_inside(&format!(
            "cd {VIDEO_DIR} && find . -mindepth 1 | LC_ALL=C sort"
        ))
        .expect("the serving directory would not be listed")
}

fn catalogue(server: &TestServer) -> String {
    server
        .exec_inside(&format!("cat {VIDEO_DIR}/library.json"))
        .expect("the catalogue would not read")
}

/// Every path the catalogue names for `media_id` really is on the server, and it is under
/// `slug`.
async fn catalogue_matches_the_files(
    state: &AppState,
    id: &str,
    media_id: &str,
    slug: &str,
) -> Vec<String> {
    let view = library::library_list(state, id, true).await.unwrap();
    let media = view
        .media
        .iter()
        .find(|m| m.id == media_id)
        .expect("the medium is gone from the catalogue");
    assert_eq!(media.slug, slug);
    assert!(
        media.files.iter().all(|f| f.exists_on_server),
        "the catalogue points at files that are not there: {media:?}"
    );
    let paths: Vec<String> = media.files.iter().map(|f| f.path.clone()).collect();
    assert!(
        paths.iter().all(|p| p.starts_with(slug)),
        "a path does not follow the short name {slug}: {paths:?}"
    );
    paths
}

/// A `flock` in front of the real one that, the first time it is called after the test
/// arms it, says it is waiting and waits for "go" (at most 30 s). `/usr/local/bin` comes
/// before `/usr/bin` in the PATH sshd gives a command.
fn install_held_flock(server: &TestServer) {
    let real = server
        .exec_inside("command -v flock")
        .expect("no flock on the server");
    assert_eq!(real.trim(), "/usr/bin/flock", "flock is not where expected");
    server
        .exec_inside(
            "printf '%s\\n' '#!/bin/bash' \
             'if [ -e /tmp/t620-arm ]; then rm -f /tmp/t620-arm; touch /tmp/t620-waiting; \
               for i in $(seq 1 300); do [ -e /tmp/t620-go ] && break; sleep 0.1; done; fi' \
             'exec /usr/bin/flock \"$@\"' > /usr/local/bin/flock && chmod 755 /usr/local/bin/flock \
             && rm -f /tmp/t620-waiting /tmp/t620-go && touch /tmp/t620-arm",
        )
        .expect("the held flock would not go in");
}

fn remove_tool(server: &TestServer, tool: &str) {
    server
        .exec_inside(&format!("/usr/bin/rm -f /usr/local/bin/{tool}"))
        .expect("the stand-in tool would not come out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_catalogue_changed_meanwhile_stops_the_rename_before_anything_moves() {
    let (server, state, id, media_id) = setup().await;
    let tree_before = tree(&server);

    install_held_flock(&server);
    let rename =
        async { library::media_rename(&state, &id, &media_id, None, Some("kino"), true).await };
    let other_client = async {
        // Wait until the rename has read the catalogue and staged its own, and its script
        // is at the lock — then change the catalogue the way another copy of the
        // application would.
        let mut waiting = false;
        for _ in 0..300 {
            if server
                .exec_inside("test -e /tmp/t620-waiting && echo YES || echo NO")
                .expect("the server would not answer")
                .contains("YES")
            {
                waiting = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(waiting, "the rename's script never reached the lock");
        let other = library::media_create(&state, &id, "Somebody else's", Some("drugoe")).await;
        server
            .exec_inside("touch /tmp/t620-go")
            .expect("could not let the rename go on");
        other
    };
    let (renamed, other) = tokio::join!(rename, other_client);
    remove_tool(&server, "flock");
    other.expect("the other client's change did not go in");

    let err = renamed.expect_err("the rename went through over somebody else's change");
    assert_eq!(
        err.code,
        ErrorCode::ManifestConflict,
        "a changed catalogue must read as \"read again and retry\": {err}"
    );
    // Nothing was moved: the files are exactly where they were (the staged catalogue is
    // gone too), and the catalogue is the other client's, naming the old paths.
    let tree_after = tree(&server);
    assert_eq!(
        tree_after, tree_before,
        "files were moved (or left behind) although the rename was refused"
    );
    let on_server = catalogue(&server);
    assert!(
        on_server.contains("\"drugoe\""),
        "the other change was lost"
    );
    assert!(
        !on_server.contains("kino"),
        "the rename reached the catalogue"
    );
    catalogue_matches_the_files(&state, &id, &media_id, "film").await;

    // Read again and retry — the contract's advice — now works.
    library::media_rename(&state, &id, &media_id, None, Some("kino"), true)
        .await
        .expect("the retry after reading again failed");
    let paths = catalogue_matches_the_files(&state, &id, &media_id, "kino").await;
    assert!(paths.contains(&String::from("kino_10.mp4")));
    let tree_now = tree(&server);
    assert!(
        !tree_now.contains("./film"),
        "an old name stayed: {tree_now}"
    );
    assert!(tree_now.contains("./kino/v6/stream.m3u8"));
    assert!(
        catalogue(&server).contains("\"drugoe\""),
        "the retry lost the other client's change"
    );
}

/// A `mv` in front of the real one that refuses exactly when its last argument is one of
/// `targets`.
fn install_refusing_mv(server: &TestServer, targets: &[String]) {
    let cases: String = targets
        .iter()
        .map(|t| format!("'if [ \"$last\" = \"{t}\" ]; then echo \"mv refused on purpose\" >&2; exit 1; fi' "))
        .collect();
    server
        .exec_inside(&format!(
            "printf '%s\\n' '#!/bin/bash' 'for last in \"$@\"; do :; done' {cases}\
             'exec /usr/bin/mv \"$@\"' > /usr/local/bin/mv && chmod 755 /usr/local/bin/mv"
        ))
        .expect("the refusing mv would not go in");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_move_refused_in_the_middle_moves_back_what_was_moved() {
    let (server, state, id, media_id) = setup().await;
    let tree_before = tree(&server);
    let catalogue_before = catalogue(&server);

    // The plan moves `film_10.mp4`, `film_22.mp4` and `film` in the order the catalogue
    // lists them; the second is refused, so the first has already been moved by then.
    install_refusing_mv(&server, &[format!("{VIDEO_DIR}/kino_22.mp4")]);
    let outcome = library::media_rename(&state, &id, &media_id, None, Some("kino"), true).await;
    remove_tool(&server, "mv");

    let err = outcome.expect_err("a refused move came back as a rename");
    assert!(
        err.says(DetailCode::RenameFailed),
        "the refusal does not name the move: {err}"
    );
    assert_eq!(
        tree(&server),
        tree_before,
        "what was moved before the refusal was not moved back"
    );
    assert_eq!(
        catalogue(&server),
        catalogue_before,
        "the catalogue changed although the rename failed"
    );
    catalogue_matches_the_files(&state, &id, &media_id, "film").await;

    // And the rename goes through once `mv` works again.
    library::media_rename(&state, &id, &media_id, None, Some("kino"), true)
        .await
        .expect("the rename after the refusal failed");
    catalogue_matches_the_files(&state, &id, &media_id, "kino").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_destination_that_already_exists_is_refused_and_nothing_moves() {
    // `mv -n` onto a name that is there does nothing — and may still exit 0. The old rename
    // then wrote a catalogue naming the new path for a file that had never moved.
    let (server, state, id, media_id) = setup().await;
    server
        .exec_inside(&format!("head -c 10 /dev/urandom > {VIDEO_DIR}/kino"))
        .expect("the stray file would not be made");
    let tree_before = tree(&server);
    let catalogue_before = catalogue(&server);

    let err = library::media_rename(&state, &id, &media_id, None, Some("kino"), true)
        .await
        .expect_err("a rename onto an existing name went through");
    assert!(err.says(DetailCode::RenameFailed), "{err}");
    assert_eq!(tree(&server), tree_before);
    assert_eq!(catalogue(&server), catalogue_before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_move_back_that_fails_too_is_told_apart_and_names_what_is_stuck() {
    let (server, state, id, media_id) = setup().await;
    let catalogue_before = catalogue(&server);

    // The second move is refused, and so is moving the first one back.
    install_refusing_mv(
        &server,
        &[
            format!("{VIDEO_DIR}/kino_22.mp4"),
            format!("{VIDEO_DIR}/film_10.mp4"),
        ],
    );
    let outcome = library::media_rename(&state, &id, &media_id, None, Some("kino"), true).await;
    remove_tool(&server, "mv");

    let err = outcome.expect_err("a refused move came back as a rename");
    let said = format!("{err:?}");
    assert!(
        said.contains("not all could be moved back") && said.contains("kino_10.mp4"),
        "the stuck entry is not named: {said}"
    );
    assert_eq!(
        catalogue(&server),
        catalogue_before,
        "the catalogue changed although the rename failed"
    );
    let now = tree(&server);
    assert!(
        now.contains("./kino_10.mp4") && now.contains("./film_22.mp4"),
        "{now}"
    );
}
