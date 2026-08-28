//! T247, T249 — a server to deploy onto, and a guard that keeps us off any other.
//!
//! Two states are needed and neither is the ordinary throwaway container: that one already
//! carries a web server, the serving directories and the rules file, and now a state file
//! too (T252a), so the application is *supposed* to recognise it as its own. Deployment is
//! checked from a bare machine and refusal is checked against somebody else's.
//!
//! **Why this is the one fixture with a guard of its own** (T249). Everything else in the
//! integration tests writes files or runs commands; the worst a wrong address costs is a
//! spoilt file. Deployment rewrites the SSH configuration, turns off password logins and
//! switches the network filter on. Pointed at a working server it does not spoil a file —
//! it takes the serving down and can lock its owner out. "We are not aiming there" is not a
//! defence, so the address is checked rather than trusted.

use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// A bare server, the way one arrives from a hosting provider.
///
/// The number in the tag is the contents' version and MUST be raised when the Dockerfile
/// changes — the same rule, and the same reason, as for the ordinary fixture: with the tag
/// left alone continuous integration takes the old image out of its cache and the checks
/// run against a container in the wrong state.
pub const CLEAN_IMAGE: &str = "vrcast-test-clean:1";

/// Somebody else's server: an nginx serving its own site, and no state file.
pub const FOREIGN_IMAGE: &str = "vrcast-test-foreign:1";

/// The prefix the clean-up step in .github/workflows/build.yml looks for leftovers by.
pub const NAME_PREFIX: &str = "vrcast-deploy-target";

/// The password inside both images. Set in their Dockerfiles, and the three must be changed
/// together. A freshly bought server is reached by password — the key is what the
/// application's own `ssh-key` step puts there.
pub const ROOT_PASSWORD: &str = "test-container-password";

/// Which of the two states to bring up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavour {
    /// Nothing deployed: no web server, no /etc/vrcast, no /etc/caddy.
    Clean,
    /// Somebody else was here first.
    Foreign,
}

impl Flavour {
    fn image(self) -> &'static str {
        match self {
            Self::Clean => CLEAN_IMAGE,
            Self::Foreign => FOREIGN_IMAGE,
        }
    }

    fn dir(self) -> &'static str {
        match self {
            Self::Clean => "docker-clean",
            Self::Foreign => "docker-foreign",
        }
    }
}

fn docker(args: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new("docker").args(args).output()
}

fn fixtures_dir() -> std::path::PathBuf {
    super::test_key::fixtures_dir()
}

fn ensure_image(flavour: Flavour) -> Result<(), String> {
    let dir = fixtures_dir().join(flavour.dir());
    let out = docker(&[
        "build",
        "-t",
        flavour.image(),
        "-f",
        dir.join("Dockerfile").to_string_lossy().as_ref(),
        dir.to_string_lossy().as_ref(),
    ])
    .map_err(|e| format!("could not start the image build: {e}"))?;

    if !out.status.success() {
        return Err(format!(
            "the image {} would not build:\n{}",
            flavour.image(),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

/// **The guard.** Refuse any address that is not the throwaway stand.
///
/// Called by everything in this module that hands an address out, so a deployment test cannot
/// be written against another server without deliberately going around it.
///
/// Two shapes of address pass, and only two. The loopback, which is where the containers
/// publish their ports when these tests run on the machine itself. And a name this module made
/// for a container of its own — `NAME_PREFIX-…` — which is how the server is reached when the
/// tests are themselves inside a container and joined its network. Nothing outside this file
/// hands out either, and no real server is called that.
///
/// It panics rather than returning an error on purpose. An error can be swallowed by a `?` in
/// a helper somebody writes later; a panic stops the test.
pub fn only_the_stand(host: &str) {
    let stand = matches!(host, "127.0.0.1" | "::1" | "localhost")
        || host.starts_with(&format!("{NAME_PREFIX}-"));
    assert!(
        stand,
        "the deployment checks run against the throwaway stand only, and the address is \
         {host}. Deploying rewrites the login settings, turns password logins off and puts a \
         network filter up: on a working server that is not a spoilt file, it is serving down \
         and its owner locked out"
    );
}

/// Makes the container's name unique within a run.
static SEQ: AtomicU32 = AtomicU32::new(0);

/// A server standing ready to be deployed onto — or refused.
///
/// Removed when the value is dropped, a failed test included.
pub struct DeployTarget {
    flavour: Flavour,
    name: String,
    /// The port the way in over SSH is published on. Changes after `reset`.
    pub port: u16,
    /// A network of this target's own, made only when these tests are themselves running in a
    /// container: then our container joins it and the target answers to its own name there.
    /// See `fixture::own_container` for why neither loopback nor the machine's name will do.
    network: Option<String>,
    /// Our own container, while it is attached to that network.
    joined: Option<String>,
}

impl DeployTarget {
    /// Bring one up and wait until sshd answers.
    pub fn start(flavour: Flavour) -> Result<Self, String> {
        if !super::fixture::docker_available() {
            return Err(String::from(
                "Docker is not running. Open Docker Desktop and try again: the deployment \
                 checks bring up a throwaway server in a container.",
            ));
        }
        ensure_image(flavour)?;

        let name = format!(
            "{NAME_PREFIX}-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        );
        let mut target = Self {
            flavour,
            name,
            port: 0,
            network: None,
            joined: None,
        };
        target.run_container()?;
        Ok(target)
    }

    fn run_container(&mut self) -> Result<(), String> {
        // Everything systemd needs, established by measurement rather than by copying an
        // incantation (T246): PID 1 is /sbin/init, the cgroup namespace is the container's
        // own, and /run is a tmpfs. What this buys and what it does not is written out in
        // the clean image's Dockerfile — services, sshd and the network filter work here;
        // swap, the global kernel keys and udev do not, anywhere.
        // Made once and kept across a `reset`: the container is thrown away and started
        // again, the network it answers on is not.
        if self.network.is_none() {
            if let Some(id) = super::fixture::own_container() {
                let network = format!("{}-net", self.name);
                // **A leftover of a run that was killed rather than finished.** The name is
                // the same every time, so one abandoned network blocks these tests for good:
                // `network create` refuses, and nothing here ever gets as far as the cleanup
                // that would have removed it. A cancelled job leaves exactly this — and a job
                // is cancelled every time a push supersedes one still running, which is not
                // rare. Caught on 2026-08-28 by two of this session's own pushes.
                //
                // Asked about before being removed: `remove_network` waits between attempts
                // for the ordinary race, and spending that wait on a network that was never
                // there would cost every deployment test the same seconds for nothing.
                // Everything left by a run that was killed rather than finished — not only
                // a network of this very name. The name carries the process id that made it,
                // so an abandoned one is never met again under its own name and would sit
                // there for good.
                super::fixture::sweep_abandoned_networks();
                let out = docker(&["network", "create", &network])
                    .map_err(|e| format!("could not create the network: {e}"))?;
                if !out.status.success() {
                    return Err(format!(
                        "the network would not be created:\n{}",
                        String::from_utf8_lossy(&out.stderr)
                    ));
                }
                let out = docker(&["network", "connect", &network, &id])
                    .map_err(|e| format!("could not join the network: {e}"))?;
                if !out.status.success() {
                    let _ = docker(&["network", "rm", &network]);
                    return Err(format!(
                        "this container would not join the network:\n{}",
                        String::from_utf8_lossy(&out.stderr)
                    ));
                }
                self.network = Some(network);
                self.joined = Some(id);
            }
        }

        let image = self.flavour.image();
        let mut args: Vec<&str> = vec![
            "run",
            "-d",
            "--rm",
            "--name",
            &self.name,
            "--privileged",
            "--cgroupns=private",
            "--tmpfs",
            "/run",
            "--tmpfs",
            "/run/lock",
        ];
        match &self.network {
            Some(network) => {
                args.extend_from_slice(&["--network", network, "--network-alias", &self.name])
            }
            None => args.extend_from_slice(&["-p", "127.0.0.1::22"]),
        }
        args.push(image);
        let out = docker(&args).map_err(|e| format!("could not start the container: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "the container would not start:\n{}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }

        // On the network the port is the container's own; published, it is whatever the
        // machine had free.
        self.port = if self.network.is_some() {
            22
        } else {
            self.discover_port()?
        };
        self.wait_until_ssh_answers()?;
        Ok(())
    }

    /// Back to the state the image describes.
    ///
    /// Needed because safety on a repeat (FR-124) is checked by **interrupting** a
    /// deployment, and there is only one thing to interrupt: a deployment that began on a
    /// bare machine. Undoing the steps one by one would be undoing them with the same code
    /// that applied them, which checks nothing.
    ///
    /// The container is thrown away and started again from the image. The worry that this
    /// would cost minutes turned out to be about the wrong thing: the build costs minutes,
    /// the start costs seconds, and the build has already happened.
    ///
    /// The published port changes, so anything holding the old one must read `port` again.
    pub fn reset(&mut self) -> Result<(), String> {
        let _ = docker(&["rm", "-f", &self.name]);
        self.run_container()
    }

    fn discover_port(&self) -> Result<u16, String> {
        let out = docker(&["port", &self.name, "22/tcp"])
            .map_err(|e| format!("could not learn the port: {e}"))?;
        let text = String::from_utf8_lossy(&out.stdout);
        text.lines()
            .find_map(|l| l.rsplit(':').next())
            .and_then(|p| p.trim().parse::<u16>().ok())
            .ok_or_else(|| format!("the port could not be parsed from \"{}\"", text.trim()))
    }

    /// Wait until OpenSSH **says who it is** — not until the port accepts (R-20).
    ///
    /// Docker publishes a port through a proxy that accepts before anything inside is
    /// listening and then resets the connection; a wait that stopped at "the port accepts"
    /// stopped too early and cost continuous integration a run on 2026-08-26. Here there is
    /// a second reason to wait for the protocol: systemd is still bringing the system up,
    /// and sshd is one of the last things it starts.
    fn wait_until_ssh_answers(&self) -> Result<(), String> {
        use std::io::Read;

        // Through the address the tests are given, not through a loopback written out here:
        // a readiness probe that knocks somewhere else answers about somewhere else.
        let (host, port) = self.address();
        let address = std::net::ToSocketAddrs::to_socket_addrs(&(host.as_str(), port))
            .map_err(|e| format!("the address {host}:{port} could not be resolved: {e}"))?
            .next()
            .ok_or_else(|| format!("the address {host}:{port} resolved to nothing"))?;
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut last = String::from("nothing accepted the connection at all");

        while Instant::now() < deadline {
            match std::net::TcpStream::connect_timeout(&address, Duration::from_millis(500)) {
                Ok(mut stream) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(1500)));
                    let mut said = [0u8; 64];
                    match stream.read(&mut said) {
                        Ok(heard)
                            if String::from_utf8_lossy(&said[..heard]).starts_with("SSH-") =>
                        {
                            return Ok(())
                        }
                        Ok(_) => last = String::from("something answered, but not OpenSSH"),
                        Err(e) => last = e.to_string(),
                    }
                }
                Err(e) => last = e.to_string(),
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        Err(format!(
            "sshd in the container did not answer within 60 seconds ({last}). systemd said:\n{}",
            self.boot_log().unwrap_or_default()
        ))
    }

    /// What systemd made of the boot. Only for a failure's message.
    pub fn boot_log(&self) -> Result<String, String> {
        let out = docker(&["logs", "--tail", "40", &self.name])
            .map_err(|e| format!("could not read the container's log: {e}"))?;
        Ok(format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ))
    }

    /// The address to reach the container at — and the only way to obtain it, so the guard
    /// is unavoidable.
    pub fn address(&self) -> (String, u16) {
        let host = match &self.network {
            // Its own name on the network we joined. The guard knows this shape, and knows it
            // because this module is the only thing that makes it.
            Some(_) => self.name.clone(),
            None => String::from("127.0.0.1"),
        };
        only_the_stand(&host);
        (host, self.port)
    }

    /// Run a command inside, going around our own access layer.
    ///
    /// Setting conditions up and checking consequences **independently** of the code under
    /// test: checking a result with the same means that produced it checks nothing.
    pub fn exec_inside(&self, script: &str) -> Result<String, String> {
        let out = docker(&["exec", &self.name, "bash", "-lc", script])
            .map_err(|e| format!("could not run the command in the container: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "the command failed in the container:\n{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Whether a path exists inside. Says so plainly rather than through a command's exit
    /// code, because "the file is not there" and "the command would not run" are different
    /// answers and only one of them is an answer.
    pub fn has(&self, path: &str) -> Result<bool, String> {
        let said = self.exec_inside(&format!("test -e {path} && echo yes || echo no"))?;
        match said.trim() {
            "yes" => Ok(true),
            "no" => Ok(false),
            other => Err(format!("an unintelligible answer about {path}: {other:?}")),
        }
    }
}

impl Drop for DeployTarget {
    fn drop(&mut self) {
        // Forced: systemd shuts down politely and there is nothing here worth waiting for.
        let _ = docker(&["rm", "-f", &self.name]);
        // Ourselves off the network, then the network. In that order: Docker will not remove
        // one with anything still attached, and a leaked network eats from a finite pool of
        // addresses — when it runs out, nothing starts at all, with a message that says
        // nothing about this place.
        if let Some(network) = &self.network {
            if let Some(id) = &self.joined {
                let _ = docker(&["network", "disconnect", "-f", network, id]);
            }
            super::fixture::remove_network(network);
        }
    }
}
