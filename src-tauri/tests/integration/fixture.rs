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
//! | services, the network filter, kernel settings, the certificate | **no** | yes |
//!
//! So Phase 7 is checked against a dedicated test VPS rather than here.

use std::process::Command;
use std::time::{Duration, Instant};

/// The image's name. Built once and reused between test runs.
///
/// The number in the tag is the contents' version: when the Dockerfile changes it MUST be
/// raised, or `ensure_image` reuses the old image and the edit quietly has no effect. The
/// same tag stands in the clean-up step in .github/workflows/build.yml.
pub const IMAGE: &str = "vrcast-test-sshd:2";

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
    pub port: u16,
}

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

        // The system picks the port: the tests may run alongside anything at all, and
        // taking a port known in advance will not do.
        let out = docker(&["run", "-d", "--rm", "-p", "127.0.0.1::22", IMAGE])
            .map_err(|e| format!("could not start the container: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "the container would not start:\n{}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        let id = String::from_utf8_lossy(&out.stdout).trim().to_owned();

        // The value owns the container from this moment: should parsing the port or waiting
        // for readiness fail, the container is cleaned up on drop rather than left hanging.
        let mut server = Self { id, port: 0 };
        server.port = server.discover_port()?;
        server.wait_until_ready()?;
        Ok(server)
    }

    fn discover_port(&self) -> Result<u16, String> {
        let out = docker(&["port", &self.id, "22/tcp"])
            .map_err(|e| format!("could not learn the port: {e}"))?;
        let text = String::from_utf8_lossy(&out.stdout);
        text.lines()
            .find_map(|l| l.rsplit(':').next())
            .and_then(|p| p.trim().parse::<u16>().ok())
            .ok_or_else(|| format!("the port could not be parsed from \"{}\"", text.trim()))
    }

    /// Wait until the server starts accepting connections.
    ///
    /// The service does not come up instantly, and connecting right after the container
    /// starts would fail for reasons other than our code. What is waited for is readiness
    /// itself rather than "let us just wait a bit".
    fn wait_until_ready(&self) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if std::net::TcpStream::connect_timeout(
                &format!("127.0.0.1:{}", self.port)
                    .parse()
                    .map_err(|e| format!("a bad address: {e}"))?,
                Duration::from_millis(500),
            )
            .is_ok()
            {
                // The connection is accepted, but the service needs another moment to be
                // ready to talk.
                std::thread::sleep(Duration::from_millis(300));
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        Err(String::from(
            "the server in the container did not start accepting connections within 30 seconds",
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
        "127.0.0.1"
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
