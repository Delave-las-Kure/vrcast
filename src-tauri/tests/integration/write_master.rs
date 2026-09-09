//! T572 — `write_master` writes `master.m3u8` staged-and-renamed, the way `send` (a few
//! functions above it in `tasks/ladder_build.rs`) already writes every variant.
//!
//! **Why against a real container rather than a mock.** What is being checked depends
//! entirely on real SFTP/SSH behaviour: whether `sftp.create` on a path in a directory that
//! does not exist actually fails the way the code assumes, whether a `.part` file really
//! survives an aborted write until it is cleaned up, and whether `mv -f` really replaces an
//! existing file rather than refusing. None of that can be exercised against an invented
//! connection.
//!
//! `write_master` is `pub(crate)`... in fact `pub`, exactly for this: see its own doc
//! comment in `tasks::ladder_build` for why (T529 already made the same call for
//! `variant_already_there`, in the same file).

use vrcast_studio_lib::ssh::{fingerprint, Connection, Credentials, ServerAddress};
use vrcast_studio_lib::tasks::ladder_build::write_master;

use super::fixture::{key_path, TestServer, KEY_PASSPHRASE};
use super::viewer::Viewer;

const VIDEO_DIR: &str = "/var/lib/vrcast/videos";

/// A connection independent of the application's own gate — the same shape
/// `ladder_attach.rs` and `manifest_conflict.rs` already use for the same reason: setting
/// conditions up must not lean on the code under test.
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

#[tokio::test]
async fn a_rebuilt_master_replaces_the_old_one_with_no_part_file_left_behind() {
    let server = TestServer::start().expect("the container would not come up");
    let conn = connect(&server).await;

    let slug_dir = format!("{VIDEO_DIR}/rewrite-me");
    server
        .exec_inside(&format!(
            "mkdir -p '{slug_dir}' && printf '%s' 'OLD MASTER, ABOUT TO BE REPLACED' > \
             '{slug_dir}/master.m3u8'"
        ))
        .expect("could not lay out the pre-existing master.m3u8");

    // A viewer pulling the OLD master right up to the moment of the rewrite: the point of
    // T572 is that this request never sees a truncated or empty file, and there is no
    // in-between state at all for it to catch — the closest a test can come to proving that
    // without a race on the write itself is proving the two properties that together make it
    // true: no `.part` litter, and the final content is whole and new.
    let viewer = Viewer::attach(&server).expect("the viewer would not attach");
    let before = viewer
        .fetch("/videos/rewrite-me/master.m3u8")
        .expect("the pre-existing master.m3u8 was not servable");
    assert_eq!(before.trim(), "OLD MASTER, ABOUT TO BE REPLACED");

    let new_body = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000000\nv0/stream.m3u8\n";
    write_master(&conn, &format!("{slug_dir}/master.m3u8"), new_body)
        .await
        .expect("write_master failed on a plain, healthy write");

    let on_server = server
        .exec_inside(&format!("cat '{slug_dir}/master.m3u8'"))
        .expect("master.m3u8 is not on the server after write_master");
    assert_eq!(
        on_server, new_body,
        "master.m3u8 does not hold the new body after the rewrite"
    );

    // No `.part` file left over — the whole point of staging is that it is temporary.
    assert!(
        server
            .exec_inside(&format!("test -e '{slug_dir}/master.m3u8.part'"))
            .is_err(),
        "a .part file was left behind after a successful write"
    );

    // Served correctly, over the same connection a real viewer would use.
    let served = viewer
        .fetch("/videos/rewrite-me/master.m3u8")
        .expect("the rewritten master.m3u8 was not servable");
    assert_eq!(
        served, new_body,
        "what is served does not match what was written"
    );

    viewer.stop_watching().ok();
    conn.close().await;
}

#[tokio::test]
async fn a_failed_write_leaves_the_old_master_untouched_and_no_part_file_behind() {
    let server = TestServer::start().expect("the container would not come up");
    let conn = connect(&server).await;

    // `path` points into a directory that does not exist: `sftp.create` has nowhere to put
    // even the staged file, so the write fails before ever reaching the `mv`.
    let broken_path = format!("{VIDEO_DIR}/no-such-directory-at-all/master.m3u8");

    let err = write_master(&conn, &broken_path, "this must never land")
        .await
        .expect_err("a write into a non-existent directory was not reported as a failure");
    // Named, not silent: an `SshError` wrapped in `BuildError::Ssh`, exactly the shape
    // `write_master`'s doc comment promises for this failure.
    assert!(
        matches!(
            err,
            vrcast_studio_lib::tasks::ladder_build::BuildError::Ssh(_)
        ),
        "a failed write into a missing directory was not reported as an SSH failure: {err:?}"
    );

    assert!(
        server
            .exec_inside(&format!(
                "test -e '{VIDEO_DIR}/no-such-directory-at-all/master.m3u8'"
            ))
            .is_err(),
        "something was written despite the failure"
    );
    assert!(
        server
            .exec_inside(&format!(
                "test -e '{VIDEO_DIR}/no-such-directory-at-all/master.m3u8.part'"
            ))
            .is_err(),
        "a .part file was left behind after a failed write"
    );

    conn.close().await;
}

#[tokio::test]
async fn a_failed_rebuild_leaves_the_previous_master_exactly_as_it_was() {
    // The property T572 exists for, stated directly rather than only through the absence of
    // litter: on a REBUILD specifically — where, unlike the test above, an old master.m3u8
    // already exists and is presumably being served — a write that cannot even get its
    // staged copy down must not touch the file a viewer is currently reading.
    //
    // The connection here is `root`, the same as the application's own — and root's SFTP
    // writes bypass ordinary permission bits, so a `chmod 555` on the directory (tried
    // first) does not actually block anything and only proved this test wrong, not the
    // code. What DOES fail regardless of who is asking is `sftp.create` on a path that is
    // already a directory: that is the failure mode used here instead.
    let server = TestServer::start().expect("the container would not come up");
    let conn = connect(&server).await;

    let slug_dir = format!("{VIDEO_DIR}/rebuild-fails");
    let old_body = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=500000\nold/stream.m3u8\n";
    server
        .exec_inside(&format!(
            "mkdir -p '{slug_dir}' && printf '%s' '{old_body}' > '{slug_dir}/master.m3u8' \
             && mkdir -p '{slug_dir}/master.m3u8.part'"
        ))
        .expect("could not lay out the pre-existing master.m3u8 and blocking .part directory");

    let result = write_master(
        &conn,
        &format!("{slug_dir}/master.m3u8"),
        "NEW, MUST NOT LAND",
    )
    .await;

    assert!(
        result.is_err(),
        "a write whose staged path is already a directory was not reported as a failure"
    );
    assert!(
        matches!(
            result.unwrap_err(),
            vrcast_studio_lib::tasks::ladder_build::BuildError::Ssh(_)
        ),
        "a failed staged write was not reported as an SSH failure"
    );

    let on_server = server
        .exec_inside(&format!("cat '{slug_dir}/master.m3u8'"))
        .expect("master.m3u8 is gone after the failed rebuild");
    assert_eq!(
        on_server, old_body,
        "the previous master.m3u8 was changed despite the rewrite failing"
    );
    // The blocking directory itself must be exactly what it was — untouched, still a
    // directory, not swept away by whatever cleanup a partial write might attempt.
    assert!(
        server
            .exec_inside(&format!("test -d '{slug_dir}/master.m3u8.part'"))
            .is_ok(),
        "the pre-existing .part directory was removed or altered by the failed write"
    );

    conn.close().await;
}
