//! T026 — a throwaway server for the integration tests.
//!
//! The container is brought up by a test and removed along with it. The live server is
//! **never** used (constitution, the "Way of working" section): the checks here change the
//! server's state, and on the live one that would break somebody's viewing.
//!
//! What the container covers and what it does not is worth understanding, so as not to
//! count a check as fuller than it is:
//!
//! | | container | a real VPS |
//! |---|---|---|
//! | file operations, commands, the catalogue, transfers with breaks | yes | yes |
//! | serving over HTTP, the access log, the quality-limit rules (T149) | yes | yes |
//! | services, the network filter, kernel settings, the certificate | **no** | yes |
//!
//! So Phase 7 is checked against a dedicated test VPS rather than here.

use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// What a port is expected to say for itself.
#[derive(Debug, Clone, Copy)]
enum Speaks {
    /// OpenSSH announces its version as soon as it accepts.
    Ssh,
    /// A web server says nothing until it is asked something.
    Http,
}

impl Speaks {
    /// Whether whatever stands behind this address answers as this protocol.
    fn answer_from(self, address: &std::net::SocketAddr) -> Result<bool, String> {
        use std::io::{Read, Write};

        let mut stream = std::net::TcpStream::connect_timeout(address, Duration::from_millis(500))
            .map_err(|e| e.to_string())?;
        stream
            .set_read_timeout(Some(Duration::from_millis(1500)))
            .map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(Some(Duration::from_millis(1500)))
            .map_err(|e| e.to_string())?;

        if matches!(self, Self::Http) {
            // HTTP/1.0 on purpose: the answer ends with the connection, so nothing has to
            // be parsed to know it finished.
            stream
                .write_all(b"HEAD / HTTP/1.0\r\n\r\n")
                .map_err(|e| e.to_string())?;
        }

        let mut said = [0u8; 64];
        let heard = stream.read(&mut said).map_err(|e| e.to_string())?;
        let said = String::from_utf8_lossy(&said[..heard]);
        Ok(match self {
            Self::Ssh => said.starts_with("SSH-"),
            // Any status at all: a 404 from the root is still a web server answering, and
            // the question here is whether it is up, not whether it serves this path.
            Self::Http => said.starts_with("HTTP/"),
        })
    }
}

/// The image's name. Built once and reused between test runs.
///
/// The number in the tag is the contents' version: when the Dockerfile changes it MUST be
/// raised, or `ensure_image` reuses the old image and the edit quietly has no effect. The
/// same tag stands in the clean-up step in .github/workflows/build.yml.
///
/// Raised to 3 by T149, when the container gained a web server. Leaving it at 2 would have
/// been the worst of the possible outcomes: continuous integration would have taken the old
/// image out of its cache, and the checks of the viewers would have run against a container
/// that serves nothing — passing or failing for reasons unrelated to the code.
///
/// Raised to 5 by T252a, when the container gained its state file. Without it the server
/// reads as somebody else's under the recognition rule, and every changing operation would
/// be refused on it once the detector exists.
pub const IMAGE: &str = "vrcast-test-sshd:5";

/// The prefix of the networks the fixture creates. The clean-up step in
/// .github/workflows/build.yml looks for leftovers by it.
pub const NETWORK_PREFIX: &str = "vrcast-test-net";

/// The name the container answers to inside its own network. The viewer helpers reach the
/// serving by it, since the address is handed out by Docker and is not known in advance.
pub const SERVER_ALIAS: &str = "vrcast-server";

/// The key's passphrase. Not a secret: the key is made on the spot, lives only on this
/// machine and opens nothing but the throwaway container.
pub const KEY_PASSPHRASE: &str = super::test_key::PASSPHRASE;

/// The password inside the container — for checking the second way in. It is set in the
/// Dockerfile too, and the two must be changed together.
pub const ROOT_PASSWORD: &str = "test-container-password";

fn fixtures_dir() -> std::path::PathBuf {
    super::test_key::fixtures_dir()
}

/// The private key for logging in to the container. Made on the first test run and never
/// committed to the repository — see `tests/support/test_key.rs`.
pub fn key_path() -> std::path::PathBuf {
    super::test_key::key_path()
}

fn docker(args: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new("docker").args(args).output()
}

/// Whether there is a working Docker.
///
/// Without one the integration tests do not run — but do not pretend to have passed either:
/// a test that is quietly skipped is worse than none, because it creates the appearance of a
/// check.
pub fn docker_available() -> bool {
    matches!(docker(&["info", "--format", "{{.ServerVersion}}"]), Ok(o) if o.status.success())
}

/// Build the image.
///
/// The build runs ALWAYS rather than only when the image is missing. With the contents
/// unchanged Docker takes everything from its cache and fits in a fraction of a second,
/// while a new key or an edit to the Dockerfile is picked up by itself. The earlier
/// approach — "there is an image under that tag, so no build is needed" — would quietly
/// leave an earlier key's public half in the container, and logging in would stop working
/// with no hint as to why.
fn ensure_image() -> Result<(), String> {
    let dir = fixtures_dir().join("docker");
    let out = docker(&[
        "build",
        "-t",
        IMAGE,
        "-f",
        dir.join("Dockerfile").to_string_lossy().as_ref(),
        dir.to_string_lossy().as_ref(),
    ])
    .map_err(|e| format!("could not start the image build: {e}"))?;

    if !out.status.success() {
        return Err(format!(
            "the image would not build:\n{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

/// A running throwaway server.
///
/// Removed when the value is dropped — including when a test fails: otherwise dangling
/// containers would be left in the system after a failed run.
pub struct TestServer {
    id: String,
    /// The port the way in over SSH is published on.
    pub port: u16,
    /// The port the serving is published on (T149).
    pub http_port: u16,
    /// The container's own network. The viewer helpers are attached to it so that each
    /// arrives from an address of its own — see `integration/viewer.rs`.
    network: String,
}

/// Makes the name of a network unique within a run.
///
/// The process's number alone will not do: `cargo test` runs the tests of one binary in one
/// process, and two servers brought up in a row would collide over the name.
static NETWORK_SEQ: AtomicU32 = AtomicU32::new(0);

impl TestServer {
    /// Bring the container up and wait until it starts accepting connections.
    pub fn start() -> Result<Self, String> {
        // First of all — an intelligible cause instead of an unhelpful "the container would
        // not come up". Caught on a live run on 2026-08-25: Docker had gone to sleep, and
        // ten tests failed with a message the cause could not be read from.
        if !docker_available() {
            return Err(String::from(
                "Docker is not running. Open Docker Desktop and try again: the integration tests bring up a throwaway server in a container.",
            ));
        }
        super::test_key::ensure()?;
        ensure_image()?;

        // A network of its own, rather than the default one. On the default bridge every
        // request from the host reaches the server from one and the same address — the
        // gateway's — and the whole of milestone C rests on telling one viewer from
        // another. On a network of its own each attached helper gets an address of its own,
        // and the serving sees exactly what it would see from different people.
        let network = format!(
            "{NETWORK_PREFIX}-{}-{}",
            std::process::id(),
            NETWORK_SEQ.fetch_add(1, Ordering::Relaxed)
        );
        let out = docker(&["network", "create", &network])
            .map_err(|e| format!("could not create the network: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "the network would not be created:\n{}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }

        // The system picks the ports: the tests may run alongside anything at all, and
        // taking ports known in advance will not do.
        let out = docker(&[
            "run",
            "-d",
            "--rm",
            "--network",
            &network,
            "--network-alias",
            SERVER_ALIAS,
            "-p",
            "127.0.0.1::22",
            "-p",
            "127.0.0.1::80",
            IMAGE,
        ])
        .map_err(|e| format!("could not start the container: {e}"))?;
        if !out.status.success() {
            // The network is removed here and not left to `Drop`: there is no value to drop
            // yet, and a leftover network would outlive the run.
            let _ = docker(&["network", "rm", &network]);
            return Err(format!(
                "the container would not start:\n{}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        let id = String::from_utf8_lossy(&out.stdout).trim().to_owned();

        // The value owns the container and the network from this moment: should parsing a
        // port or waiting for readiness fail, both are cleaned up on drop rather than left
        // hanging.
        let mut server = Self {
            id,
            port: 0,
            http_port: 0,
            network,
        };
        server.port = server.discover_port("22/tcp")?;
        server.http_port = server.discover_port("80/tcp")?;
        server.wait_until_ready()?;
        Ok(server)
    }

    fn discover_port(&self, spec: &str) -> Result<u16, String> {
        let out = docker(&["port", &self.id, spec])
            .map_err(|e| format!("could not learn the port {spec}: {e}"))?;
        let text = String::from_utf8_lossy(&out.stdout);
        text.lines()
            .find_map(|l| l.rsplit(':').next())
            .and_then(|p| p.trim().parse::<u16>().ok())
            .ok_or_else(|| {
                format!(
                    "the port {spec} could not be parsed from \"{}\"",
                    text.trim()
                )
            })
    }

    /// Wait until the server starts accepting connections.
    ///
    /// The service does not come up instantly, and connecting right after the container
    /// starts would fail for reasons other than our code. What is waited for is readiness
    /// itself rather than "let us just wait a bit".
    fn wait_until_ready(&self) -> Result<(), String> {
        // Both ways in, and both are waited for. Waiting only for the way in over SSH would
        // let a test start while the serving is still coming up, and it would fail once in
        // a while for a reason having nothing to do with it — the worst kind of check,
        // since a flickering one teaches people to run it again until it goes green.
        self.wait_until_answers(self.port, "the way in over SSH", Speaks::Ssh)?;
        self.wait_until_answers(self.http_port, "the serving", Speaks::Http)?;
        Ok(())
    }

    /// Wait until the service **answers**, not until its port accepts.
    ///
    /// **An open port proves nothing** (R-20), and this is where the project learned it a
    /// second time. Docker publishes a port by putting a proxy in front of the container:
    /// the proxy accepts a connection whether or not anything inside is listening yet, and
    /// then resets it. A wait that stopped at "the port accepts" therefore stopped too
    /// early, and the sleep that followed papered over the gap on a fast machine and not on
    /// a loaded one. Continuous integration failed on 2026-08-26 with `Connection reset by
    /// peer` while probing a host key — in a test that had nothing to do with the change it
    /// was run for, which is the worst kind of failure: it teaches people to press the
    /// button again.
    ///
    /// So the proof is the protocol's own first words. OpenSSH says who it is the moment it
    /// accepts; a web server is asked something and has to answer.
    fn wait_until_answers(&self, port: u16, what: &str, speaks: Speaks) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(30);
        let address: std::net::SocketAddr = format!("127.0.0.1:{port}")
            .parse()
            .map_err(|e| format!("a bad address: {e}"))?;

        let mut last = String::from("nothing accepted the connection at all");
        while Instant::now() < deadline {
            match speaks.answer_from(&address) {
                Ok(true) => return Ok(()),
                Ok(false) => last = String::from("something answered, but not this protocol"),
                Err(e) => last = e,
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        Err(format!(
            "{what} in the container did not answer within 30 seconds ({last}). \
             The container's log:\n{}",
            self.sshd_log().unwrap_or_default()
        ))
    }

    /// Run a command inside the container, going around our own access layer.
    ///
    /// Needed to set conditions up and to check consequences **independently** of the very
    /// code under test: checking a result with the same means that produced it is not
    /// checking anything.
    pub fn exec_inside(&self, script: &str) -> Result<String, String> {
        let out = docker(&["exec", &self.id, "bash", "-lc", script])
            .map_err(|e| format!("could not run the command in the container: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "the command in the container failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    pub fn host(&self) -> &'static str {
        // **Где отвечают опубликованные порты, а это не всегда петля.**
        //
        // Контейнер публикует порты на машине, где работает демон. С самой машины `127.0.0.1`
        // верен и всегда был верен. А изнутри **другого** контейнера — а там теперь идут
        // проверки — петля своя, и на ней никого нет.
        //
        // Поэтому адрес можно назвать, а по умолчанию он прежний. Другой путь — отдать
        // заданию сеть машины — закрыт: исполнитель ведёт для задания-контейнера свою сеть и
        // при `--network host` не создаёт контейнер вовсе («Value cannot be null. (Parameter
        // 'ContainerId')», 2026-08-27).
        match option_env!("VRCAST_TEST_HOST") {
            Some(host) if !host.is_empty() => host,
            _ => "127.0.0.1",
        }
    }

    /// The name of the container's network — the one the viewer helpers are attached to.
    pub fn network(&self) -> &str {
        &self.network
    }

    /// The serving's access log, as the server itself wrote it.
    ///
    /// Half of what is known about a viewer is read out of here (R-02), and the check of
    /// the parsing is written against this shape. Absent means empty rather than an error:
    /// Caddy creates the file on the first request, and "nobody has asked for anything yet"
    /// is a normal state rather than a failure.
    pub fn access_log(&self) -> Result<String, String> {
        self.exec_inside("cat /var/log/caddy/access.log 2>/dev/null || true")
    }

    /// Wait for a line about the given path to appear in the access log.
    ///
    /// The log is written after the answer has gone out, so looking at it straight after a
    /// request is a race. When the time is up the error carries the log itself — so that a
    /// failure shows at once what the server really recorded.
    pub fn wait_in_access_log(&self, needle: &str, limit: Duration) -> Result<String, String> {
        let deadline = Instant::now() + limit;
        loop {
            let log = self.access_log()?;
            if log.contains(needle) {
                return Ok(log);
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "\"{needle}\" did not appear in the access log in the time allowed. The log:\n{log}"
                ));
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// Put a local file into the container, going around our own access layer.
    ///
    /// Needed to set conditions up independently of the code under test: planting a
    /// half-downloaded file by its own means would check that the code agrees with
    /// itself.
    pub fn put_file(&self, local: &std::path::Path, remote: &str) -> Result<(), String> {
        let out = docker(&[
            "cp",
            local.to_string_lossy().as_ref(),
            &format!("{}:{remote}", self.id),
        ])
        .map_err(|e| format!("could not run docker cp: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "the file could not be put into the container: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        Ok(())
    }

    /// sshd's log — what the service writes to the container's error stream (it is run with
    /// `-e`).
    ///
    /// There is no log file of its own in the container: no system-log daemon is installed
    /// there. The test used to check /var/log/auth.log — the file did not exist, and the
    /// check "no login attempt reached the server" passed whatever the behaviour.
    pub fn sshd_log(&self) -> Result<String, String> {
        let out =
            docker(&["logs", &self.id]).map_err(|e| format!("could not read the log: {e}"))?;
        // stdout and stderr are put together: sshd writes to stderr, but there is no point
        // separating them.
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        Ok(text)
    }

    /// Wait for a line to appear in sshd's log.
    ///
    /// The log is written asynchronously: checking it right after an action is a race. When
    /// the time is up an error comes back holding the log itself — so that a failure shows
    /// at once what the server really recorded.
    pub fn wait_in_sshd_log(&self, needle: &str, limit: Duration) -> Result<String, String> {
        let deadline = Instant::now() + limit;
        loop {
            let log = self.sshd_log()?;
            if log.contains(needle) {
                return Ok(log);
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "the line \"{needle}\" did not appear in sshd's log in the time allowed. The log:\n{log}"
                ));
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        // The container is run with self-removal, but it is stopped explicitly: otherwise
        // after a failed test it would hang about until the end of the session.
        let _ = docker(&["kill", &self.id]);
        // And the network after it. Docker refuses to remove one that still has something
        // attached, and self-removal of the container is not instant, so this is given a
        // few attempts rather than one. A leaked network is not harmless: the pool of
        // addresses they are handed out from is finite, and once it runs out no container
        // will start at all — with a message that says nothing about this place.
        for _ in 0..10 {
            if matches!(docker(&["network", "rm", &self.network]), Ok(o) if o.status.success()) {
                return;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

/// Switch the log on if it was asked for through `VRCAST_LOG`.
///
/// Only on request. The log here is not a convenience but the other half of the check
/// against leaking secrets (T064, T128, SC-011): it sets the level to `trace` so that
/// talkative libraries lay out everything they know, and searches that for traces of the
/// key. Switching it on always would bury an ordinary run in output nobody reads — and then
/// nobody would notice anything in it either.
///
/// Calling it again is harmless: the subscriber is set through `try_init`.
pub fn logging_if_requested() {
    if std::env::var_os("VRCAST_LOG").is_some() {
        vrcast_studio_lib::logging::init();
    }
}

/// Deliberately write a secret into the log — to check that it gets cut out.
///
/// Without this the search for leaks proved nothing, and that was found by measuring: with
/// redaction switched off COMPLETELY the run still came out clean. The reason turned out to
/// be harmless — the passphrase simply never gets written anywhere — but a check that is
/// equally silent with the protection working and with it removed checks nothing.
///
/// The line goes through the same writer as the whole application log, so its fate is the
/// answer to "does redaction work in this run".
///
/// Call it ONLY after the secret has been registered: before registration it leaks by
/// rights, and the check fails fairly but beside the point.
pub fn canary(secret: &str) {
    if std::env::var_os("VRCAST_LOG").is_some() {
        tracing::trace!(probe = %secret, "a probe for secret redaction");
    }
}
