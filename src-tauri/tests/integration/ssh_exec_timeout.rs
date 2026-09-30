//! T595 — `Connection::exec` gives up on a remote command that hangs, instead of waiting on
//! it forever.
//!
//! **What this is not testing.** `deploy_network_cut.rs` already covers a connection that
//! goes silent — the keepalive closes it in ~90-120s, and `exec` returns because its channel
//! died out from under it. That is a different failure from the one T595 closes: here the
//! connection stays alive and healthy throughout (`ss` on the container would show it
//! ESTABLISHED the whole time), and the remote command itself simply never exits — the
//! realistic case named in the task is `apt-get install` stuck behind `/var/lib/dpkg/lock`.
//! No keepalive fires for that, because nothing about the network is wrong.
//!
//! `sleep 999` inside the container reproduces exactly that shape without needing `dpkg` or
//! any other real contention: a live, well-behaved SSH session running a command that will
//! not exit within any span this test is prepared to wait for.

use std::time::{Duration, Instant};

use vrcast_studio_lib::ssh::{fingerprint, Connection, Credentials, ServerAddress};

use super::fixture::{key_path, TestServer, KEY_PASSPHRASE};

async fn connect(server: &TestServer) -> Connection {
    let a = ServerAddress::new(server.host(), server.port);
    let fp = fingerprint::probe(&a)
        .await
        .expect("the fingerprint was not obtained");
    Connection::connect(
        a,
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

/// The ceiling given to `exec_with_timeout` in this test — short on purpose, so the test
/// itself runs in seconds rather than the production `EXEC_CEILING` of 600s.
const TEST_CEILING: Duration = Duration::from_secs(2);

/// How long the test is willing to wait for `exec_with_timeout` to come back before calling
/// the whole mechanism broken. Generous relative to `TEST_CEILING`, so a slightly slow CI
/// runner does not turn into a flaky failure — but bounded, so a regression back to "no
/// timeout at all" fails this test instead of hanging the suite forever.
const MUST_RETURN_WITHIN: Duration = Duration::from_secs(20);

#[tokio::test]
async fn a_hung_remote_command_is_given_up_on_rather_than_waited_for_forever() {
    let server = TestServer::start().expect("the container would not come up");
    let conn = connect(&server).await;

    let started = Instant::now();
    let result = tokio::time::timeout(
        MUST_RETURN_WITHIN,
        conn.exec_with_timeout("sleep 999", TEST_CEILING),
    )
    .await
    .expect(
        "exec_with_timeout() never returned at all within the time this test allows — a \
         regression back to no timeout would hang here forever, which is exactly what T595 \
         exists to make impossible",
    );
    let elapsed = started.elapsed();

    // Not instant: a timeout that fires before the ceiling it was given is not really
    // bounding anything, it is a different bug wearing this test's colours.
    assert!(
        elapsed >= TEST_CEILING,
        "exec_with_timeout() returned after {elapsed:?}, before its own {TEST_CEILING:?} \
         ceiling had even elapsed — the timeout did not really wait for the ceiling"
    );
    // And not much later than it: the whole point is that giving up happens close to the
    // ceiling, not eventually.
    assert!(
        elapsed < TEST_CEILING + Duration::from_secs(10),
        "exec_with_timeout() took {elapsed:?} against a {TEST_CEILING:?} ceiling — far \
         enough over that something other than the ceiling decided when it returned"
    );

    let err = result.expect_err(
        "exec_with_timeout() returned Ok for a command that never exits — `sleep 999` was \
         never going to send ChannelMsg::ExitStatus within the ceiling given",
    );
    let text = err.to_string();
    assert!(
        text.contains("did not finish") || text.contains("sleep 999"),
        "the timeout fired, but the error does not say a command was given up on: {text}"
    );

    // The connection itself is unharmed: this was a healthy session the whole way through,
    // and a second ordinary command on the very same `Connection` must still work.
    let out = tokio::time::timeout(Duration::from_secs(10), conn.exec("echo still-alive"))
        .await
        .expect("a plain command after the timeout hung too — the connection was left unusable")
        .expect("a plain command after the timeout failed outright");
    assert!(
        out.ok() && out.stdout.contains("still-alive"),
        "the connection did not survive a command timing out on it: {out:?}"
    );
}

/// **The deploy context: proof this is the same code path, not a separate one.**
///
/// `Context::ran`/`Context::asks` (`src/server/deploy/mod.rs`) go through
/// `Context::exec_marked`, and it — like `put_file`'s own commands — through one private
/// `send`, which calls `self.conn.exec_if(&wrapped, …)` with `wrapped = self.run.wrap(command)`
/// (T609: the command marked and in a group of its own; T637: `exec_if` is `exec` with the
/// last "stopping?" asked right before `channel.exec` — the same `exec_gated`, the same
/// `EXEC_CEILING`). There is no second implementation of running
/// a command for deploy steps to fall into. Grep rather than a second live-container run: a
/// step stuck on `exec` for real (the `apt-get`/dpkg-lock scenario from the task) would cost
/// minutes to reproduce honestly with a hung Docker container, for no more assurance than
/// reading the lines that prove the same function is called. `nothing_unchecked.rs` and
/// `cancelling.rs` make the identical trade for the identical reason: what a source read can
/// state exactly, a slow behavioural test would only restate at a much higher price.
#[test]
fn deploy_steps_run_commands_through_the_same_timeout_bounded_exec() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/server/deploy/mod.rs");
    let text = std::fs::read_to_string(&path).expect("could not read server/deploy/mod.rs");
    let why = "if a new path to run a remote command was added, it needs its own route through \
               the T595 timeout, or this whole test needs to follow it";
    assert_eq!(
        text.matches("self.conn.exec").count(),
        1,
        "deploy/mod.rs runs remote commands in more than one place, or in none — {why}"
    );
    assert!(
        text.contains("let wrapped = self.run.wrap(command);")
            && text.contains("self.conn.exec_if(&wrapped, move || gate.open())"),
        "the one send no longer calls `conn.exec_if` with the marked command — {why}"
    );
    let exec = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ssh/exec.rs"),
    )
    .expect("could not read ssh/exec.rs");
    assert!(
        exec.contains("self.exec_gated(command, EXEC_CEILING, still_wanted).await"),
        "`exec_if` no longer runs under the T595 `EXEC_CEILING` — {why}"
    );
    assert!(
        text.contains("let said = self.exec_marked(command).await?;")
            && text.contains("Ok(self.exec_marked(command).await?.stdout.trim() == \"yes\")"),
        "Context::ran/Context::asks no longer go through `exec_marked` — {why}"
    );
}

/// T637 — `exec_if` with a gate that says no sends nothing, and gives its channel back.
///
/// Nothing sent: the command would leave a file, and the file is not there. Channel given
/// back: twelve refusals in a row — more than sshd's `MaxSessions` of 10 — and an ordinary
/// command after them still gets a channel; a refused channel left open would hold a session
/// on the server until the connection closed. And a yes runs the command as `exec` does.
#[tokio::test]
async fn a_command_held_back_at_its_channel_is_not_sent_and_leaves_no_channel_open() {
    let server = TestServer::start().expect("the container would not come up");
    let conn = connect(&server).await;

    for _ in 0..12 {
        let held = tokio::time::timeout(
            Duration::from_secs(20),
            conn.exec_if("touch /tmp/t637-sent", || false),
        )
        .await
        .expect("a command held back at its channel hung")
        .expect("a command held back at its channel failed");
        assert!(
            held.is_none(),
            "a gate that said no still ran the command: {held:?}"
        );
    }
    assert_eq!(conn.brief_channels_in_use(), 0);

    let out = tokio::time::timeout(
        Duration::from_secs(20),
        conn.exec("test -e /tmp/t637-sent && echo sent || echo not-sent"),
    )
    .await
    .expect("an ordinary command after the held-back ones hung — their channels were not closed")
    .expect("an ordinary command after the held-back ones failed");
    assert_eq!(
        out.trimmed(),
        "not-sent",
        "a command held back at its channel reached the server"
    );

    let let_through = conn
        .exec_if("echo let-through", || true)
        .await
        .expect("a command let through failed")
        .expect("a gate that said yes held the command back");
    assert!(let_through.ok() && let_through.stdout.contains("let-through"));
}
