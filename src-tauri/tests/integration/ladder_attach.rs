//! T528, T529 — a built quality set is filed under its medium as a nested path, with its
//! own particulars, not as an unrecognised top-level directory named by a bare string.
//!
//! **Why against a real server rather than a hand-assembled catalogue.** `attach_built_set`
//! reads the catalogue, files the set under its medium and writes it back — three round
//! trips through `manifest_io`, which is itself checked against a real OpenSSH. A contract
//! test with an invented `Manifest` would check the matching logic in isolation and say
//! nothing about whether the write actually lands, or whether `library_list` afterwards
//! reads back what was written the way a person would see it.
//!
//! **Why the fixture and not a real encode.** `tasks::ladder_build::run` needs FFmpeg and
//! minutes per rung; the only thing this task changed is what happens *after* a build
//! succeeds, so a set laid out to the same shape `hls_fixture::lay_out_ladder` already
//! produces for the serving checks is the real "just built" state — a master, three rungs —
//! without paying for an encode nobody is testing here. Only the `.facts` file (T529's own
//! source of a rung's length) is added on top, the way the cutting step leaves one.

use vrcast_studio_lib::commands::ladder::attach_built_set;
use vrcast_studio_lib::commands::library::api as library;
use vrcast_studio_lib::commands::servers::{api as servers, ServerInput};
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::domain::server_profile::AuthKind;
use vrcast_studio_lib::ssh::{fingerprint, Connection, Credentials, ServerAddress};
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;

use super::fixture::{key_path, TestServer, KEY_PASSPHRASE};
use super::hls_fixture;
use super::library_ops::confirm_fingerprint;

const VIDEO_DIR: &str = "/var/lib/vrcast/videos";

fn app_state() -> AppState {
    AppState::with_db(
        std::sync::Arc::new(Db::open_in_memory().unwrap()),
        std::sync::Arc::new(InMemorySecretStore::new()),
    )
    .expect("the application state would not assemble")
}

async fn setup() -> (TestServer, AppState, String) {
    let server = TestServer::start().expect("the container would not come up");
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

/// A connection independent of the application's own gate — the same shape
/// `manifest_conflict.rs` uses, so setting conditions up does not lean on the code under
/// test.
async fn connect(server: &TestServer) -> Connection {
    let addr = ServerAddress::new(server.host(), server.port);
    let fp = fingerprint::probe(&addr)
        .await
        .expect("the fingerprint was not obtained");
    Connection::connect(
        addr,
        "root",
        Credentials::Key {
            path: key_path(),
            passphrase: Some(KEY_PASSPHRASE.to_owned()),
        },
        &fp,
    )
    .await
    .expect("connecting failed")
}

/// Put a `.facts` file beside a rung the fixture laid out, the way the cutting step leaves
/// one — three segments, none of them a fragment `hls_master::peak_bps` would discount.
fn seed_facts(server: &TestServer, slug: &str, rung: &str, width: u32, height: u32) {
    let facts = format!(
        "sub={rung}\nwidth={width}\nheight={height}\nfps=24.000\nlevel=41\ncodec=h264\n\
         seg 4.000 4000000\nseg 4.000 5000000\nseg 4.000 3000000\n"
    );
    server
        .exec_inside(&format!(
            "cat > '{VIDEO_DIR}/{slug}/{rung}/.facts' <<'VRCAST_FACTS_EOF'\n{facts}VRCAST_FACTS_EOF"
        ))
        .expect("could not write the .facts file");
}

#[tokio::test]
async fn a_built_set_is_filed_under_its_medium_as_a_nested_ladder_with_real_particulars() {
    let (server, state, server_id) = setup().await;

    // The medium the build is meant to land in — created the way a person creates one, by
    // slug, before the build ever runs.
    let media_id = library::media_create(&state, &server_id, "Задние комнаты", Some("backrooms"))
        .await
        .expect("the medium was not created");

    // The set itself: three rungs, laid out exactly as the serving checks already lay one
    // out. `v1` is the heaviest by construction (see `hls_fixture::RUNGS`), so it is the
    // rung `ladder_probe::top_rung` is expected to read back.
    hls_fixture::lay_out_ladder(&server, "backrooms").expect("the quality set was not laid out");
    seed_facts(&server, "backrooms", "v1", 1920, 1080);

    // The step this task added: what `ladder_build::run` calls once the set is on the
    // server, over the same connection a real build would still be holding.
    let conn = connect(&server).await;
    let attached = attach_built_set(&conn, VIDEO_DIR, "backrooms")
        .await
        .expect("the set was not attached to any medium");
    assert_eq!(
        attached, media_id,
        "the set was attached to the wrong medium"
    );
    conn.close().await;

    // Read the whole way a person would: through the library command, over a fresh
    // connection, exactly as `LibraryScreen` does.
    let view = library::library_list(&state, &server_id, true)
        .await
        .expect("the library would not read");
    let media = view
        .media
        .iter()
        .find(|m| m.id == media_id)
        .expect("the medium vanished");

    assert!(
        media.files.is_empty(),
        "the set was filed as an ordinary file rather than as a ladder: {media:?}"
    );
    let ladder = media
        .ladders
        .first()
        .expect("the set is not in the medium's ladders at all");

    // T528: the nested path, not the bare slug — this is the whole of the fix.
    assert_eq!(
        ladder.path, "backrooms/master.m3u8",
        "the ladder was not recorded as a nested path"
    );
    assert!(
        ladder.exists_on_server,
        "the set exists on the server and was not recognised as existing"
    );

    // T529: a real size (the whole directory's total, not zero), and a real structure —
    // resolution, bitrate, length — rather than a bare string with nothing behind it.
    assert!(
        ladder.size_bytes > 0,
        "the set's size came back as zero, exactly the hole T529 closes"
    );
    assert_eq!(ladder.width, Some(1920), "the resolution was not read back");
    assert_eq!(ladder.height, Some(1080));
    assert!(
        ladder.bitrate_bps.is_some_and(|b| b > 0),
        "the bitrate was not read back: {:?}",
        ladder.bitrate_bps
    );
    assert!(
        ladder.duration_s.is_some_and(|d| d > 0.0),
        "the length was not read back: {:?}",
        ladder.duration_s
    );
    assert!(
        ladder.origin_url.contains("backrooms/master.m3u8"),
        "no link was built for the set: {}",
        ladder.origin_url
    );

    // And the medium's total counts the set's weight, the same as an ordinary file's would.
    assert!(
        media.total_bytes >= ladder.size_bytes,
        "the medium's total does not include the set's weight"
    );
}

#[tokio::test]
async fn a_slug_matching_no_medium_is_left_unattached_rather_than_invented() {
    // T528's own stated boundary: a build whose slug matches nothing is not this task's
    // problem to solve, and must behave exactly as it did before — the set surfaces as an
    // unrecognised directory, and nothing is invented on its behalf.
    let (server, state, server_id) = setup().await;
    hls_fixture::lay_out_ladder(&server, "nobody-owns-this")
        .expect("the quality set was not laid out");

    let conn = connect(&server).await;
    let attached = attach_built_set(&conn, VIDEO_DIR, "nobody-owns-this").await;
    conn.close().await;
    assert!(
        attached.is_none(),
        "a medium was invented for a slug that matches none: {attached:?}"
    );

    let view = library::library_list(&state, &server_id, true)
        .await
        .expect("the library would not read");
    assert!(
        view.unrecognized
            .iter()
            .any(|f| f.path == "nobody-owns-this"),
        "the unmatched set did not surface as unrecognised: {:?}",
        view.unrecognized
    );
}
