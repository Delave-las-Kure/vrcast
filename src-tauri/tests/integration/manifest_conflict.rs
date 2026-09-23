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

/// Everything in the serving directory except the catalogue itself — what a person would
/// see as "not recognised" in their library.
fn litter(server: &TestServer) -> Vec<String> {
    server
        .exec_inside(&format!("ls -A {VIDEO_DIR}"))
        .expect("the directory would not read")
        .lines()
        .map(str::trim)
        .filter(|n| !n.is_empty() && *n != "library.json")
        .map(str::to_owned)
        .collect()
}

fn catalogue_on_server(server: &TestServer) -> String {
    server
        .exec_inside(&format!("cat {VIDEO_DIR}/library.json"))
        .expect("the catalogue would not read by the server's own means")
}

#[tokio::test]
async fn two_simultaneous_writes_over_one_generation_one_wins_one_is_refused() {
    // T604. The test above lets the first write finish before the second begins, so it
    // never exercised the window the old code had: both copies checked the generation, then
    // both staged, then both moved — and the second move quietly wiped out the first. Here
    // the two writes run at once, over separate connections, from the same generation, and
    // round after round: exactly one must go through, the other must be refused as a
    // conflict, and what lies on the server must be the winner's record, whole.
    const ROUNDS: usize = 8;
    let server = TestServer::start().expect("the container would not come up");
    let first = connect(&server).await;
    let second = connect(&server).await;

    for round in 0..ROUNDS {
        let base = manifest_io::read(&first, VIDEO_DIR).await.unwrap();
        let a_slug = format!("a{round}");
        let b_slug = format!("b{round}");
        let a = with_media(&base, &format!("m_a{round}"), &a_slug);
        let b = with_media(&base, &format!("m_b{round}"), &b_slug);

        let (ra, rb) = tokio::join!(
            manifest_io::write(&first, VIDEO_DIR, &a, base.generation),
            manifest_io::write(&second, VIDEO_DIR, &b, base.generation),
        );

        let winner = match (&ra, &rb) {
            (Ok(()), Err(ManifestIoError::Conflict { .. })) => &a_slug,
            (Err(ManifestIoError::Conflict { .. }), Ok(())) => &b_slug,
            _ => panic!(
                "round {round}: exactly one write must win and the other be refused as a \
                 conflict, got {ra:?} and {rb:?}"
            ),
        };
        let loser = if winner == &a_slug { &b_slug } else { &a_slug };

        let outcome = manifest_io::read(&first, VIDEO_DIR).await.unwrap();
        assert_eq!(
            outcome.generation,
            base.generation + 1,
            "round {round}: the generation did not grow by exactly one"
        );
        assert_eq!(
            outcome.media.len(),
            base.media.len() + 1,
            "round {round}: the catalogue does not hold exactly the winner's change: {outcome:?}"
        );
        assert!(
            outcome.find_by_slug(winner).is_some(),
            "round {round}: the write reported as successful is not on the server"
        );
        assert!(
            outcome.find_by_slug(loser).is_none(),
            "round {round}: the write reported as refused is on the server"
        );
        // Every earlier winner is still there: nothing was lost quietly in any round.
        let on_server = catalogue_on_server(&server);
        assert!(
            on_server.contains(&format!("\"{winner}\"")),
            "round {round}: the server's own reading lacks the winner: {on_server}"
        );
        assert!(
            litter(&server).is_empty(),
            "round {round}: service files were left in the serving directory: {:?}",
            litter(&server)
        );
    }
}

/// Put a tool in front of the real one that refuses exactly when its last argument is
/// `target`, and passes everything else through. `/usr/local/bin` comes before `/usr/bin`
/// in the PATH sshd gives a command, so the application's own scripts run into it.
fn install_failing(server: &TestServer, tool: &str, target: &str) {
    server
        .exec_inside(&format!(
            "printf '%s\\n' '#!/bin/bash' \
             'for last in \"$@\"; do :; done' \
             'if [ \"$last\" = \"{target}\" ]; then echo \"{tool} refused on purpose\" >&2; exit 1; fi' \
             'exec /usr/bin/{tool} \"$@\"' > /usr/local/bin/{tool} && chmod 755 /usr/local/bin/{tool}"
        ))
        .expect("the failing tool would not go in");
}

fn remove_failing(server: &TestServer, tool: &str) {
    server
        .exec_inside(&format!("/usr/bin/rm -f /usr/local/bin/{tool}"))
        .expect("the failing tool would not come out");
}

#[tokio::test]
async fn a_replacement_that_fails_is_an_error_and_leaves_the_catalogue_as_it_was() {
    // T604, the lesson of T603: a move that did not happen must never read as a write
    // that did. The move is made to fail on the server; the write must come back with an
    // error, the catalogue must be the one from before to the byte, and the staged file
    // must be gone.
    let server = TestServer::start().expect("the container would not come up");
    let conn = connect(&server).await;

    let base = manifest_io::read(&conn, VIDEO_DIR).await.unwrap();
    manifest_io::write(
        &conn,
        VIDEO_DIR,
        &with_media(&base, "m_1", "film"),
        base.generation,
    )
    .await
    .unwrap();
    let before = catalogue_on_server(&server);

    let read = manifest_io::read(&conn, VIDEO_DIR).await.unwrap();
    install_failing(&server, "mv", &format!("{VIDEO_DIR}/library.json"));
    let outcome = manifest_io::write(
        &conn,
        VIDEO_DIR,
        &with_media(&read, "m_2", "drugoe"),
        read.generation,
    )
    .await;
    remove_failing(&server, "mv");

    match outcome {
        Err(ManifestIoError::Ssh(_)) => {}
        other => panic!("a failed replacement came back as {other:?}"),
    }
    assert_eq!(
        catalogue_on_server(&server),
        before,
        "the catalogue changed although its replacement failed"
    );
    assert!(
        litter(&server).is_empty(),
        "the staged file was left behind: {:?}",
        litter(&server)
    );
}

#[tokio::test]
async fn a_write_that_cannot_get_the_lock_is_refused_and_leaves_nothing() {
    // T604. Somebody holds the catalogue's lock (the test itself, on the serving directory)
    // for longer than a write will wait. The write must give up with "somebody else is
    // changing the library" — never replace anything without the lock — and leave neither
    // the catalogue changed nor a staged file behind.
    let server = TestServer::start().expect("the container would not come up");
    let conn = connect(&server).await;

    let base = manifest_io::read(&conn, VIDEO_DIR).await.unwrap();
    manifest_io::write(
        &conn,
        VIDEO_DIR,
        &with_media(&base, "m_1", "film"),
        base.generation,
    )
    .await
    .unwrap();
    let before = catalogue_on_server(&server);

    server
        .exec_inside(&format!(
            "setsid nohup flock -x {VIDEO_DIR} sleep 120 >/dev/null 2>&1 < /dev/null & \
             echo $! > /tmp/zz-t604-holder.pid"
        ))
        .expect("the test could not take the lock");
    let mut held = false;
    for _ in 0..50 {
        let probe = server
            .exec_inside(&format!(
                "flock -n -x {VIDEO_DIR} true && echo FREE || echo HELD"
            ))
            .expect("the lock would not be probed");
        if probe.contains("HELD") {
            held = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(held, "the test's own lock never took hold");

    let read = manifest_io::read(&conn, VIDEO_DIR).await.unwrap();
    let outcome = manifest_io::write(
        &conn,
        VIDEO_DIR,
        &with_media(&read, "m_2", "drugoe"),
        read.generation,
    )
    .await;

    // By the process group `setsid` made, not by a pattern: `pkill -f` would match the very
    // shell running it, whose command line holds the same words.
    server
        .exec_inside("kill -- -\"$(cat /tmp/zz-t604-holder.pid)\"; rm -f /tmp/zz-t604-holder.pid")
        .expect("the test's lock would not be let go");

    match outcome {
        Err(e @ ManifestIoError::Busy) => {
            let app = vrcast_studio_lib::commands::error::AppError::from(e);
            assert_eq!(
                app.code,
                vrcast_studio_lib::commands::error::ErrorCode::ManifestConflict,
                "a lock timeout must read as \"read again and retry\""
            );
        }
        other => panic!("a write without the lock came back as {other:?}"),
    }
    assert_eq!(
        catalogue_on_server(&server),
        before,
        "the catalogue changed without the lock"
    );
    assert!(
        litter(&server).is_empty(),
        "the staged file was left behind: {:?}",
        litter(&server)
    );
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
