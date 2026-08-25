//! Checking the server-access layer against a real OpenSSH.
//!
//! What is checked here is what cannot be checked without a server: login by a
//! passphrase-protected key carrying through to the end, the fingerprint matching what the
//! server itself computes, a refusal on a foreign fingerprint happening **before** the
//! credentials are sent, channels multiplexing inside one connection, file operations
//! working.

use super::fixture::{key_path, TestServer, KEY_PASSPHRASE, ROOT_PASSWORD};
use vrcast_studio_lib::ssh::{fingerprint, Connection, Credentials, ServerAddress, SshError};

fn addr(server: &TestServer) -> ServerAddress {
    ServerAddress::new(server.host(), server.port)
}

fn key_credentials() -> Credentials {
    Credentials::Key {
        path: key_path(),
        passphrase: Some(KEY_PASSPHRASE.to_owned()),
    }
}

async fn connect(server: &TestServer) -> Connection {
    let a = addr(server);
    let fp = fingerprint::probe(&a)
        .await
        .expect("the fingerprint was not obtained");
    Connection::connect(a, "root", key_credentials(), &fp)
        .await
        .expect("connecting failed")
}

#[tokio::test]
async fn the_fingerprint_matches_what_the_server_itself_computes() {
    let server = TestServer::start().expect("the container would not come up");

    let ours = fingerprint::probe(&addr(&server))
        .await
        .expect("the fingerprint was not obtained");

    // Compared against the server's own tool rather than against ourselves: otherwise it
    // would only check that our code steadily produces one and the same value — right or
    // wrong.
    let theirs = server
        .exec_inside("ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub | awk '{print $2}'")
        .expect("the server did not name its fingerprint");

    assert_eq!(
        ours.trim(),
        theirs.trim(),
        "our fingerprint disagrees with what the server computes"
    );
}

#[tokio::test]
async fn login_by_a_passphrase_protected_key_carries_through() {
    // FR-096. The unit test checks only parsing the key; here it is that the key really gets
    // us into a real OpenSSH.
    let server = TestServer::start().expect("the container would not come up");
    let conn = connect(&server).await;

    let out = conn
        .exec("id -un")
        .await
        .expect("the command would not run");
    assert!(out.ok(), "the command ended unsuccessfully");
    assert_eq!(out.trimmed(), "root");
}

#[tokio::test]
async fn a_key_with_no_passphrase_does_not_get_through() {
    let server = TestServer::start().expect("the container would not come up");
    let a = addr(&server);
    let fp = fingerprint::probe(&a).await.unwrap();

    let err = Connection::connect(
        a,
        "root",
        Credentials::Key {
            path: key_path(),
            passphrase: None,
        },
        &fp,
    )
    .await
    .expect_err("a protected key with no passphrase got us in");

    assert!(
        matches!(err, SshError::KeyNeedsPassphrase { .. }),
        "the wrong error came back: {err}"
    );
}

#[tokio::test]
async fn login_by_password_works_and_a_wrong_password_is_refused() {
    let server = TestServer::start().expect("the container would not come up");
    let a = addr(&server);
    let fp = fingerprint::probe(&a).await.unwrap();

    let conn = Connection::connect(
        a.clone(),
        "root",
        Credentials::Password(ROOT_PASSWORD.to_owned()),
        &fp,
    )
    .await
    .expect("login with the right password failed");
    assert!(conn.exec("true").await.unwrap().ok());
    conn.close().await;

    let err = Connection::connect(
        a,
        "root",
        Credentials::Password(String::from("nothing-like-the-password")),
        &fp,
    )
    .await
    .expect_err("a wrong password got us in");

    match err {
        SshError::AuthFailed { methods } => {
            // The list of methods offered is what tells "wrong password" from "login by
            // password is forbidden". It must not be empty.
            assert!(!methods.is_empty(), "the server named not one way in");
        }
        other => panic!("the wrong error came back: {other}"),
    }

    // The wrong password DID reach the server — sshd recorded the refused attempt. The very
    // same line is looked for by the foreign-fingerprint test, only there it must NOT be
    // present: here it is proved that the marker is real and appears when credentials go
    // out.
    server
        .wait_in_sshd_log("Failed password", std::time::Duration::from_secs(10))
        .expect("the server did not record the refused login attempt");
}

#[tokio::test]
async fn with_a_foreign_fingerprint_the_credentials_are_not_sent() {
    // A decision stricter than the specification (see ssh/fingerprint.rs): the refusal
    // happens at the handshake. That is checked by the server's own log rather than by our
    // error — it must hold not one login attempt.
    let server = TestServer::start().expect("the container would not come up");

    let err = Connection::connect(
        addr(&server),
        "root",
        Credentials::Password(String::from("a-password-that-must-not-reach-the-server")),
        "SHA256:aCertainlyForeignServerFingerprint00000000000",
    )
    .await
    .expect_err("we connected to a server with a foreign fingerprint");

    assert!(
        matches!(err, SshError::HostKeyChanged { .. }),
        "the wrong error came back: {err}"
    );

    // Checked against sshd's log. First — that the log sees our connection at all: broken off
    // at the handshake, it leaves a "[preauth]" trace. Without that check the assertion below
    // would be empty — silence in the log would "prove" anything at all.
    let log = server
        .wait_in_sshd_log("[preauth]", std::time::Duration::from_secs(10))
        .expect("sshd's log holds not a trace of our connection — the check is looking in the wrong place");

    // The main thing: not one login attempt. "Failed password" is a real sshd marker, proved
    // by the neighbouring test where it must appear.
    assert!(
        !log.contains("Failed password") && !log.contains("Accepted password"),
        "a login attempt reached the server although the fingerprint did not match. The log:\n{log}"
    );
}

#[tokio::test]
async fn many_channels_in_one_connection() {
    // R-04: a server limits how many connections may be established at once, and that is
    // exactly what once broke a ladder build. The channels must go inside one.
    let server = TestServer::start().expect("the container would not come up");
    let conn = connect(&server).await;

    // Twelve — deliberately more than the server's limit (MaxSessions 10). The layer must
    // queue them rather than refuse: going over the limit is not a person's mistake.
    let mut handles = Vec::new();
    for i in 0..12 {
        let c = conn.clone();
        handles.push(tokio::spawn(async move {
            c.exec(&format!("echo channel-{i}")).await
        }));
    }

    for h in handles {
        let out = h.await.unwrap().expect("the channel did not work");
        assert!(out.ok(), "the command in the channel ended unsuccessfully");
    }

    // The server must see one connection, not twelve. A failure of the count itself is a
    // failure rather than a fallback value: with "unwrap_or(1)" the check would pass with ss
    // missing too (it was not in the image), that is, would check nothing.
    let established = server
        .exec_inside("ss -tn state established '( sport = :22 )' | tail -n +2 | wc -l")
        .expect("could not count the connections by the server's own means");
    let count: usize = established
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("ss's output would not parse: \"{established}\""));
    assert!(
        (1..=2).contains(&count),
        "the server sees {count} connections instead of one — multiplexing does not work"
    );
}

#[tokio::test]
async fn the_error_stream_is_kept_apart_from_the_ordinary_output() {
    let server = TestServer::start().expect("the container would not come up");
    let conn = connect(&server).await;

    let out = conn
        .exec("echo TOSTDOUT; echo TOSTDERR >&2; exit 3")
        .await
        .expect("the command would not run");

    assert_eq!(out.exit_code, Some(3), "the exit code was lost");
    assert!(!out.ok());
    assert!(
        out.stdout.contains("TOSTDOUT"),
        "the ordinary output was lost"
    );
    assert!(out.stderr.contains("TOSTDERR"), "the error stream was lost");
}

#[tokio::test]
async fn the_file_operations_work() {
    let server = TestServer::start().expect("the container would not come up");
    let conn = connect(&server).await;
    let sftp = conn.sftp().await.expect("the file session would not open");

    // The serving directory in the container is where it is on a real server.
    let entries = sftp
        .read_dir("/var/lib/vrcast")
        .await
        .expect("the directory would not read");
    let names: Vec<String> = entries.map(|e| e.file_name()).collect();
    assert!(
        names.iter().any(|n| n == "videos"),
        "there is no videos directory: {names:?}"
    );

    // Writing and reading back — what a resumable transfer will rest on.
    //
    // MIND the choice of call: the library's `write` opens a file for writing ONLY, without
    // creating it, and on a path that does not exist gives "no such file". `create` creates.
    // The name promises one thing and the behaviour is another — easy to run into during an
    // upload. The name is deliberately not ASCII: SFTP carries it as bytes, and a mangled
    // encoding would show up exactly here.
    let path = "/var/lib/vrcast/videos/проверка.txt";
    {
        use tokio::io::AsyncWriteExt;
        let mut file = sftp.create(path).await.expect("the file was not created");
        file.write_all("содержимое проверки".as_bytes())
            .await
            .expect("the file would not write");
        file.flush()
            .await
            .expect("the file would not finish writing");
    }

    let read_back = server
        .exec_inside(&format!("cat '{path}'"))
        .expect("the file would not read by the server's own means");
    assert_eq!(read_back.trim(), "содержимое проверки");

    let meta = sftp
        .metadata(path)
        .await
        .expect("there is no information about the file");
    assert_eq!(meta.size, Some("содержимое проверки".len() as u64));

    sftp.remove_file(path)
        .await
        .expect("the file would not delete");
    assert!(
        server.exec_inside(&format!("test -e '{path}'")).is_err(),
        "the file was left behind after deleting"
    );
}

#[tokio::test]
async fn a_broken_connection_is_noticed() {
    // Breaks are the way of things rather than an exception. The application must notice
    // them rather than count a connection alive forever.
    let server = TestServer::start().expect("the container would not come up");
    let conn = connect(&server).await;
    assert!(conn.is_alive(), "a fresh connection counts as dead");

    // The server is dropped out from under the connection.
    let _ = server.exec_inside("pkill -f 'sshd: root' || true");
    drop(server);

    // The command must end in an error — quickly and plainly. The Err(_) arm (20 seconds
    // elapsed, that is, A HANG) used to count as success, and the test could not fail under
    // any implementation.
    let result =
        tokio::time::timeout(std::time::Duration::from_secs(20), conn.exec("echo alive")).await;

    match result {
        Ok(Err(_)) => {}
        Ok(Ok(out)) => panic!("the command ran on a dead connection: {out:?}"),
        Err(_) => {
            panic!("the break went unnoticed: the command hung for 20 seconds instead of erroring")
        }
    }
}
