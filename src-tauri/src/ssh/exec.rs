//! T025 — running commands on the server.

use super::{Connection, Result, SshError};
use russh::ChannelMsg;

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

impl Connection {
    /// Run a command and wait for it to finish.
    ///
    /// Opens a separate channel inside the connection that already exists — no new
    /// connection is made (R-04: servers limit how many are established at once).
    pub async fn exec(&self, command: &str) -> Result<CommandOutput> {
        // The channel slot is held for the whole run: a server limits how many
        // channels a connection may have at once, and that must not be exceeded
        // (see connection.rs).
        let _permit = self.acquire_channel().await?;

        let mut channel = self.open_session().await?;

        channel
            .exec(true, command)
            .await
            .map_err(SshError::protocol)?;

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
}
