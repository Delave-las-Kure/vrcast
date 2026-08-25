//! T038 — a catalogue conflict between two copies of the application.
//!
//! An edge case from the specification: a person has two copies of the application open (or
//! the application on two computers), and both work with one server. Without a guard the
//! second to write quietly wipes out the first one's work — and there is nowhere to learn of
//! it, because the catalogue keeps no history.
//!
//! Constitution, principle V: the application must refuse rather than pretend it worked.
//! That is exactly what is checked here — and, separately, that a refusal leaves **somebody
//! else's** change on the server rather than a half-made one.

use super::fixture::{key_path, TestServer, KEY_PASSPHRASE};
use vrcast_studio_lib::domain::manifest::Manifest;
use vrcast_studio_lib::domain::media::Media;
use vrcast_studio_lib::server::manifest_io::{self, ManifestIoError};
use vrcast_studio_lib::ssh::{fingerprint, Connection, Credentials, ServerAddress};

const VIDEO_DIR: &str = "/var/lib/vrcast/videos";

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

fn with_media(base: &Manifest, id: &str, slug: &str) -> Manifest {
    let mut next = base.prepared_for_write();
    next.media
        .push(Media::new(id, slug, slug, "2026-08-25T12:00:00Z"));
    next
}

#[tokio::test]
async fn a_missing_catalogue_reads_as_an_empty_library() {
    // On a fresh server the file is not there yet. That is a legitimate state rather than a
    // fault: failing here would declare an empty library a malfunction.
    let server = TestServer::start().expect("the container would not come up");
    let conn = connect(&server).await;

    let m = manifest_io::read(&conn, VIDEO_DIR)
        .await
        .expect("a missing catalogue would not read");
    assert_eq!(m.generation, 0);
    assert!(m.media.is_empty());
}

#[tokio::test]
async fn a_catalogue_survives_writing_and_reading() {
    let server = TestServer::start().expect("the container would not come up");
    let conn = connect(&server).await;

    let base = manifest_io::read(&conn, VIDEO_DIR).await.unwrap();
    let next = with_media(&base, "m_1", "film");
    manifest_io::write(&conn, VIDEO_DIR, &next, base.generation)
        .await
        .expect("the catalogue would not write");

    let back = manifest_io::read(&conn, VIDEO_DIR).await.unwrap();
    assert_eq!(back.generation, 1, "the generation did not grow");
    assert_eq!(back.media.len(), 1);
    assert_eq!(back.media[0].slug, "film");

    // Checked by the server's own means rather than by our code: otherwise it would check
    // that reading agrees with writing rather than that the right thing lies on the server.
    let on_server = server
        .exec_inside(&format!("cat {VIDEO_DIR}/library.json"))
        .expect("the catalogue would not read by the server's own means");
    assert!(
        on_server.contains("\"film\"") && on_server.contains("\"generation\""),
        "the wrong thing lies on the server: {on_server}"
    );
}

#[tokio::test]
async fn the_second_copy_is_refused_and_does_not_clobber_the_other() {
    // The point of the test: both copies read one and the same generation. The first wrote,
    // the second was too late and knew nothing of it.
    let server = TestServer::start().expect("the container would not come up");
    let first = connect(&server).await;
    let second = connect(&server).await;

    let read_by_first = manifest_io::read(&first, VIDEO_DIR).await.unwrap();
    let read_by_second = manifest_io::read(&second, VIDEO_DIR).await.unwrap();
    assert_eq!(
        read_by_first.generation, read_by_second.generation,
        "the test is built wrong: the copies read different generations"
    );

    manifest_io::write(
        &first,
        VIDEO_DIR,
        &with_media(&read_by_first, "m_first", "pervyy"),
        read_by_first.generation,
    )
    .await
    .expect("the first copy could not write");

    let err = manifest_io::write(
        &second,
        VIDEO_DIR,
        &with_media(&read_by_second, "m_second", "vtoroy"),
        read_by_second.generation,
    )
    .await
    .expect_err("the second copy clobbered the other's record");

    match err {
        ManifestIoError::Conflict { base, current } => {
            assert_eq!(base, read_by_second.generation);
            assert!(
                current > base,
                "the refusal names a generation no greater than the one read: {current} and {base}"
            );
        }
        other => panic!("the wrong error came back: {other}"),
    }

    // The main thing: the FIRST copy's record is left on the server, whole and parseable.
    let outcome = manifest_io::read(&second, VIDEO_DIR).await.unwrap();
    assert_eq!(
        outcome.media.len(),
        1,
        "the catalogue's contents are spoilt: {outcome:?}"
    );
    assert_eq!(
        outcome.media[0].slug, "pervyy",
        "somebody else's record was clobbered after all"
    );
}

#[tokio::test]
async fn after_re_reading_the_write_goes_through() {
    // A refusal is not a dead end: the application re-reads the catalogue and repeats the
    // action. Were a write not to go through even with a fresh generation after a conflict,
    // a person would be locked out.
    let server = TestServer::start().expect("the container would not come up");
    let first = connect(&server).await;
    let second = connect(&server).await;

    let base = manifest_io::read(&first, VIDEO_DIR).await.unwrap();
    manifest_io::write(
        &first,
        VIDEO_DIR,
        &with_media(&base, "m_first", "pervyy"),
        base.generation,
    )
    .await
    .unwrap();

    let fresh = manifest_io::read(&second, VIDEO_DIR).await.unwrap();
    manifest_io::write(
        &second,
        VIDEO_DIR,
        &with_media(&fresh, "m_second", "vtoroy"),
        fresh.generation,
    )
    .await
    .expect("a write with a fresh generation was refused too — a person would be locked out");

    let outcome = manifest_io::read(&first, VIDEO_DIR).await.unwrap();
    assert_eq!(
        outcome.media.len(),
        2,
        "one of the records was lost: {outcome:?}"
    );
    assert_eq!(outcome.generation, 2);
}

#[tokio::test]
async fn a_failed_write_leaves_no_litter_in_the_serving_directory() {
    // The staged file is a detail of how writing works, and it has no right to stay in the
    // directory the application shows a person as their library: it would land in the "not
    // recognised" group and alarm them.
    let server = TestServer::start().expect("the container would not come up");
    let first = connect(&server).await;
    let second = connect(&server).await;

    let base = manifest_io::read(&first, VIDEO_DIR).await.unwrap();
    manifest_io::write(
        &first,
        VIDEO_DIR,
        &with_media(&base, "m_1", "film"),
        base.generation,
    )
    .await
    .unwrap();

    let _ = manifest_io::write(
        &second,
        VIDEO_DIR,
        &with_media(&base, "m_2", "drugoe"),
        base.generation,
    )
    .await;

    let listing = server
        .exec_inside(&format!("ls -A {VIDEO_DIR}"))
        .expect("the directory would not read");
    let leftovers: Vec<&str> = listing
        .lines()
        .map(str::trim)
        .filter(|n| !n.is_empty() && *n != "library.json")
        .collect();
    assert!(
        leftovers.is_empty(),
        "litter was left in the directory after a failed write: {leftovers:?}"
    );
}
