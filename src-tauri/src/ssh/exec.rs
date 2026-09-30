//! T025 — running commands on the server.

use super::{Connection, Result, SshError};
use russh::ChannelMsg;
use std::time::Duration;

/// The result of running a command.
///
/// The exit code is an `Option`: the server need not send one if the channel broke.
/// A missing code is NOT success, and callers must tell the two apart.
#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub exit_code: Option<u32>,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutput {
    /// Success is an explicit zero exit code, and nothing else.
    pub fn ok(&self) -> bool {
        self.exit_code == Some(0)
    }

    /// Output without trailing newlines — what a one-line command is usually wanted for.
    pub fn trimmed(&self) -> &str {
        self.stdout.trim_end()
    }

    /// Turn a non-success into an error that says something.
    pub fn require_ok(self, what: &str) -> Result<Self> {
        if self.ok() {
            return Ok(self);
        }
        let code = match self.exit_code {
            Some(c) => c.to_string(),
            None => String::from("no exit code, the channel broke"),
        };
        let detail = if self.stderr.trim().is_empty() {
            self.stdout.trim().to_owned()
        } else {
            self.stderr.trim().to_owned()
        };
        Err(SshError::Exec(format!("{what}: code {code}. {detail}")))
    }
}

/// The longest a single `exec` may run before it is presumed hung.
///
/// ⚠ **T595, and a different failure from the one `WINDOW_CEILING` (`server/upload.rs`)
/// guards against.** That one is for a connection that is neither alive nor closed — a
/// state no keepalive reports. This one is for a connection that is alive and healthy, but
/// the REMOTE COMMAND itself never finishes and never sends `ChannelMsg::ExitStatus`:
/// measured on a live VPS, `apt-get install` blocks on `/var/lib/dpkg/lock`, held by
/// `unattended-upgrades`, for minutes at a time. The keepalive that closes a silent
/// connection in ~90-120s (`ssh::fingerprint::client_config`) does nothing here — the
/// network is not silent, the command simply has not exited.
///
/// A plain `tokio::time::timeout` is enough, unlike `write_window`'s double race against
/// `is_alive()`: `write_window`'s own doc comment says why the two differ — "`exec` does
/// not need this: a command in flight comes back by itself when its session is killed,
/// because the channel closes and the loop reading it ends" (measured 2026-09-04, returned
/// in ten seconds after the channel died). This function's whole problem is that the
/// channel is NOT dying — the process behind it just has not exited — so there is nothing
/// for a second race against `is_alive()` to add: only a firm ceiling forces the wait to
/// give up.
///
/// Set to the same **600s** anchor as `WINDOW_CEILING`, for the same reason: not seconds
/// (`apt-get`, a multi-gigabyte `sha256sum`, legitimately run for tens of seconds to a
/// couple of minutes) and not so long that a hang is indistinguishable from no timeout at
/// all. It must also stay well clear of `deploy_network_cut.rs`'s own ceiling
/// (`RUN_MUST_RETURN_WITHIN = 240s`, built around a keepalive that empties the channel in
/// ~90-120s): that test's mechanism returns long before this one would ever fire, so the
/// two timeouts do not race each other over the same failure.
pub const EXEC_CEILING: Duration = Duration::from_secs(600);

impl Connection {
    /// Run a command and wait for it to finish.
    ///
    /// Opens a separate channel inside the connection that already exists — no new
    /// connection is made (R-04: servers limit how many are established at once).
    ///
    /// Bounded by [`EXEC_CEILING`] (T595): every caller in the application goes through this
    /// one function — `Context::ran`/`Context::asks` (deploy steps), `library.rs`'s `rm -rf`,
    /// `checksum::remote`, `manifest_io::write`'s `mv`, `active_use`, `health`, and so on —
    /// so the fix sits here once rather than at each call site.
    pub async fn exec(&self, command: &str) -> Result<CommandOutput> {
        self.exec_with_timeout(command, EXEC_CEILING).await
    }

    /// The same, with the ceiling given rather than assumed — so a test can wait seconds
    /// instead of ten minutes for a hung command to be given up on (T595).
    ///
    /// `pub` rather than `pub(crate)` (the brief's own suggestion) because the check that
    /// matters — a genuinely hung remote command against a real container — lives in
    /// `tests/integration`, a separate crate linking this one from the outside; nothing
    /// inside this crate calls it with anything but the default ceiling.
    pub async fn exec_with_timeout(
        &self,
        command: &str,
        ceiling: Duration,
    ) -> Result<CommandOutput> {
        // A gate that always answers yes never holds a command back; the error is only
        // there so that no `expect` stands in a path every command of the application takes.
        self.exec_gated(command, ceiling, || true)
            .await?
            .ok_or_else(|| SshError::Exec(format!("the command was not sent: {command}")))
    }

    /// Run a command **only if `still_wanted` says yes at the moment it would be sent** —
    /// `Ok(None)` when it said no, and then nothing was sent (T637).
    ///
    /// ⚠ **Getting ready to send is not sending.** Before the command goes, `exec` waits for
    /// a place for a channel (behind other work on the same connection) and for the channel
    /// to open — refusals retried with pauses ([`Connection::open_session`]:
    /// `ResourceShortage`, `ConnectFailed`). A deployment's commands are asked "has the
    /// run been stopped?" before `exec` is called, and a cancel that came in during that
    /// preparation used to be let through: the channel opened and a command the run had not
    /// yet started went to the server (QA-21 №4). `still_wanted` is asked once the channel
    /// is open, immediately before `channel.exec`; a no closes the channel unused.
    ///
    /// Only a deployment's run uses it (`server::deploy::send_settled`); every other caller
    /// goes through [`Connection::exec`], which sends whatever happens, as before.
    pub async fn exec_if(
        &self,
        command: &str,
        still_wanted: impl FnOnce() -> bool,
    ) -> Result<Option<CommandOutput>> {
        self.exec_gated(command, EXEC_CEILING, still_wanted).await
    }

    async fn exec_gated(
        &self,
        command: &str,
        ceiling: Duration,
        still_wanted: impl FnOnce() -> bool,
    ) -> Result<Option<CommandOutput>> {
        // The channel slot is held for the whole run: a server limits how many
        // channels a connection may have at once, and that must not be exceeded
        // (see connection.rs).
        let _permit = self.acquire_channel().await?;

        let mut channel = self.open_session().await?;

        // Nothing is awaited between this answer and `channel.exec`: a stop asked after it
        // is a stop asked after the command went.
        if !still_wanted() {
            // A channel is not closed by being dropped (russh 0.63), and one left open would
            // hold one of the server's `MaxSessions` for as long as the connection lives.
            let _ = channel.close().await;
            return Ok(None);
        }

        channel
            .exec(true, command)
            .await
            .map_err(SshError::protocol)?;

        match tokio::time::timeout(ceiling, read_to_end(&mut channel)).await {
            Ok(outcome) => outcome.map(Some),
            // The connection is alive — nothing here says otherwise — but the command
            // behind it has not exited within any reasonable time. Said plainly rather than
            // as a silent hang: a deployment stuck here used to sit at `running` forever,
            // with a "cancel" button that did nothing until this same ceiling was reached
            // from inside `tasks::deploy::run`'s own `select!` (see there).
            Err(_) => Err(SshError::Exec(format!(
                "the command did not finish within {}s and was given up on: {command}",
                ceiling.as_secs()
            ))),
        }
    }
}

/// Read a channel's output to the end — the loop `exec` used to run inline, pulled out so
/// it can be raced against a timeout without duplicating it.
async fn read_to_end(channel: &mut russh::Channel<russh::client::Msg>) -> Result<CommandOutput> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_code = None;

    while let Some(msg) = channel.wait().await {
        match msg {
            ChannelMsg::Data { ref data } => stdout.extend_from_slice(data),
            // The error stream arrives as its own message type; ext == 1 is stderr.
            ChannelMsg::ExtendedData { ref data, ext: 1 } => stderr.extend_from_slice(data),
            ChannelMsg::ExitStatus { exit_status } => {
                // Leaving the loop at once will not do: output not yet read can
                // arrive after the exit code.
                exit_code = Some(exit_status);
            }
            _ => {}
        }
    }

    Ok(CommandOutput {
        exit_code,
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    })
}
