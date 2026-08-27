//! T152 — a helper that makes a viewer's requests.
//!
//! **Why a container of its own for every viewer.** What milestone C checks is the telling
//! of one viewer from another: the list of viewers is keyed by address, and a quality limit
//! is applied to an address (FR-060, FR-066). Everything sent to the serving from this
//! machine arrives from one and the same address — the network gateway's. A check built on
//! that would show two viewers merged into one and would call it correct.
//!
//! A container attached to the server's own network gets an address of its own, and the
//! serving sees exactly what it would see from a different person.
//!
//! **Why the container stays alive and the requests go through `docker exec`.** A viewer is
//! not one request: the same person asks for the description of a quality set and then
//! pulls its segments, and a check may need to ask "and what does *this* address get now"
//! after a limit has been applied. Were every request a container of its own, each would
//! come from a new address and there would be no viewer to speak of.
//!
//! This file lives beside the fixture rather than in `tests/support/`, unlike what T152
//! said: what is in support is shared with the unit tests, and this rests on a running
//! container, which they have not got.

use std::process::Command;
use std::time::{Duration, Instant};

use super::fixture::{TestServer, IMAGE, SERVER_ALIAS};

/// Someone who watches from an address of their own.
pub struct Viewer {
    id: String,
    ip: String,
}

/// What came back for one request, without its body.
#[derive(Debug)]
pub struct Probe {
    pub status: u16,
    pub bytes: u64,
    pub content_type: String,
}

fn docker(args: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new("docker").args(args).output()
}

impl Viewer {
    /// Attach a viewer to the server's network.
    ///
    /// The container does nothing by itself — it exists to hold an address. What it asks
    /// for is decided by the calls below.
    pub fn attach(server: &TestServer) -> Result<Self, String> {
        // The same image as the server's: it already holds curl, and pulling a second one
        // in would mean waiting for a download on the first run in continuous integration.
        let out = docker(&[
            "run",
            "-d",
            "--rm",
            "--network",
            server.network(),
            IMAGE,
            "sleep",
            "infinity",
        ])
        .map_err(|e| format!("could not start the viewer: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "the viewer's container would not start:\n{}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        let id = String::from_utf8_lossy(&out.stdout).trim().to_owned();

        // The value owns the container from this moment: if the address cannot be learnt,
        // it is cleaned up on drop rather than left hanging.
        let mut viewer = Self {
            id,
            ip: String::new(),
        };
        viewer.ip = viewer.discover_ip()?;
        Ok(viewer)
    }

    fn discover_ip(&self) -> Result<String, String> {
        let out = docker(&[
            "inspect",
            "-f",
            "{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}",
            &self.id,
        ])
        .map_err(|e| format!("could not learn the viewer's address: {e}"))?;
        let ip = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        if ip.is_empty() {
            return Err(String::from(
                "the viewer's container was given no address on the network",
            ));
        }
        Ok(ip)
    }

    /// The address this viewer arrives at the serving from.
    ///
    /// This is what the check compares against: the address in the list of viewers must be
    /// this one, and a quality limit is set on this one.
    pub fn ip(&self) -> &str {
        &self.ip
    }

    /// Start watching: pull a file and keep pulling until told to stop.
    ///
    /// `rate` is the ceiling on the speed, in curl's notation (`"100k"`). Passing it is how
    /// a viewer whose link is too narrow for what they are getting is made — without one
    /// there is nothing to check the `SlowLink` mark against (FR-053).
    pub fn start_watching(&self, path: &str, rate: Option<&str>) -> Result<(), String> {
        let url = format!("http://{SERVER_ALIAS}{path}");
        let mut command = String::from("curl -sS --no-buffer -o /dev/null");
        if let Some(rate) = rate {
            command.push_str(&format!(" --limit-rate {rate}"));
        }
        command.push_str(&format!(" '{url}'"));

        // Detached: the call comes back at once, and the pulling goes on. A viewer that
        // finished pulling before the check looked at the list is no viewer.
        let out = docker(&["exec", "-d", &self.id, "sh", "-c", &command])
            .map_err(|e| format!("could not start the watching: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "the watching would not start:\n{}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        Ok(())
    }

    /// Watch a quality set the way a player does: segment after segment, round and round.
    ///
    /// Not one long request. A player pulls a segment, plays it, pulls the next — and each
    /// of those finishes, so the serving records it at once and what is being watched is
    /// known throughout. One endless request would be the other case entirely, the one a
    /// directly served file makes, and it is checked separately.
    /// `rate` is the ceiling on the speed, in curl's notation (`"50k"`). A viewer of a set
    /// held below what one segment needs falls behind the clock, and that — not a slow single
    /// file — is what the ratio of content received to time lived is worked out from
    /// (`domain::stalls`). Without it there is nothing to check a starving viewer against.
    pub fn start_watching_a_set(
        &self,
        slug: &str,
        rung: &str,
        segments: usize,
        rate: Option<&str>,
    ) -> Result<(), String> {
        let names: Vec<String> = (0..segments)
            .map(|i| format!("http://{SERVER_ALIAS}/videos/{slug}/{rung}/seg{i}.ts"))
            .collect();
        let limit = rate
            .map(|r| format!(" --limit-rate {r}"))
            .unwrap_or_default();
        let command = format!(
            "while true; do for u in {}; do curl -sS{limit} -o /dev/null \"$u\" || sleep 1; done; done",
            names.join(" ")
        );
        let out = docker(&["exec", "-d", &self.id, "sh", "-c", &command])
            .map_err(|e| format!("could not start the watching: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "the watching of the set would not start:\n{}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        Ok(())
    }

    /// Ask for something once and look only at what the answer was.
    ///
    /// Good for anything whose body is not text: a segment is a few megabytes of bytes, and
    /// dragging it through a string to count them would be silly.
    pub fn probe(&self, path: &str) -> Result<Probe, String> {
        let url = format!("http://{SERVER_ALIAS}{path}");
        let out = docker(&[
            "exec",
            &self.id,
            "curl",
            "-sS",
            "-o",
            "/dev/null",
            "-m",
            "60",
            "-w",
            "%{http_code} %{size_download} %{content_type}",
            &url,
        ])
        .map_err(|e| format!("could not make the request: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "the request for {path} from {} failed: {}",
                self.ip,
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let mut parts = text.split_whitespace();
        let status = parts.next().and_then(|s| s.parse().ok());
        let bytes = parts.next().and_then(|s| s.parse().ok());
        match (status, bytes) {
            (Some(status), Some(bytes)) => Ok(Probe {
                status,
                bytes,
                content_type: parts.next().unwrap_or_default().to_owned(),
            }),
            // Not a fallback value: a fallback would let a check pass while measuring
            // nothing at all.
            _ => Err(format!("curl's answer would not parse: \"{text}\"")),
        }
    }

    /// Ask for something once and get the answer back.
    ///
    /// This is how "and what does *this* address get" is asked — the question a quality
    /// limit exists to give a different answer to (FR-061).
    pub fn fetch(&self, path: &str) -> Result<String, String> {
        let url = format!("http://{SERVER_ALIAS}{path}");
        let out = docker(&[
            "exec",
            &self.id,
            "curl",
            "-sS",
            "--fail-with-body",
            "-m",
            "20",
            &url,
        ])
        .map_err(|e| format!("could not make the request: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "the request for {path} from {} failed: {}{}",
                self.ip,
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Whether this viewer is still pulling something.
    pub fn is_watching(&self) -> bool {
        matches!(docker(&["exec", &self.id, "pgrep", "-x", "curl"]), Ok(o) if o.status.success())
    }

    /// Wait until this viewer really starts pulling.
    ///
    /// Without this a check races the helper: `docker exec -d` comes back before curl has
    /// opened anything, and "the viewer is not in the list" would mean "we looked too
    /// early" rather than anything about the code.
    pub fn wait_until_watching(&self, limit: Duration) -> Result<(), String> {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            if self.is_watching() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Err(format!(
            "the viewer {} did not start pulling within the time allowed",
            self.ip
        ))
    }

    /// Stop watching, staying attached — the address is kept.
    ///
    /// Needed for the check that a viewer leaves the active ones after the threshold
    /// (FR-055): they have to stop watching without ceasing to exist.
    pub fn stop_watching(&self) -> Result<(), String> {
        let out = docker(&["exec", &self.id, "sh", "-c", "pkill -x curl; exit 0"])
            .map_err(|e| format!("could not stop the watching: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "the watching would not stop:\n{}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        Ok(())
    }
}

impl Drop for Viewer {
    fn drop(&mut self) {
        // Killed rather than left to self-removal: the server's network is removed right
        // after, and Docker will not remove one that still has something attached.
        let _ = docker(&["kill", &self.id]);
    }
}
