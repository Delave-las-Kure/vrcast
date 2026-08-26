//! T168 — a command whose output is read as it arrives, rather than when it ends.
//!
//! `exec` waits for the command to finish and hands back everything at once. That is right
//! for a listing and useless for following a log: the following never finishes, and waiting
//! for it to would mean waiting for the server to be switched off.
//!
//! The channel place taken here is a **standing** one (T153). Following the log holds it
//! for as long as a session lasts, and those places are set aside precisely so that a band
//! of tasks cannot leave the watching of viewers unable to start.

use super::{Connection, Result, SshError};
use russh::ChannelMsg;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// How many lines may be waiting to be read before the oldest are dropped.
///
/// A serving under load writes a great many lines. If whoever reads them falls behind, it
/// is better to lose the oldest than to hold up the reading of the channel — a blocked
/// channel would stop the connection itself, and with it the transfers and the commands.
const BACKLOG: usize = 4096;

impl Connection {
    /// Run a command and hand its output back line by line as it comes.
    ///
    /// The reading stops when `cancel` is triggered or the command ends. The place for the
    /// channel is held for as long as the reading goes on and is given back by itself
    /// afterwards.
    ///
    /// Only whole lines are handed on. What arrives in a chunk is not a line — the server
    /// sends what it has when it has it, and a line is regularly split across two chunks;
    /// passing the halves on as lines would turn every such moment into a parse failure.
    pub async fn stream_lines(
        &self,
        command: &str,
        cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<String>> {
        let permit = self.reserve_standing_channel().await?;
        let mut channel = self.open_session().await?;
        channel
            .exec(true, command)
            .await
            .map_err(SshError::protocol)?;

        let (tx, rx) = mpsc::channel(BACKLOG);

        tokio::spawn(async move {
            // Held for the whole life of the reading: dropped along with this task, which
            // is the point — the place comes back however the reading ends.
            let _permit = permit;
            let mut buffer = Vec::new();

            loop {
                let message = tokio::select! {
                    biased;
                    () = cancel.cancelled() => break,
                    message = channel.wait() => message,
                };
                let Some(message) = message else { break };

                match message {
                    ChannelMsg::Data { ref data } => {
                        buffer.extend_from_slice(data);
                        while let Some(end) = buffer.iter().position(|b| *b == b'\n') {
                            let line: Vec<u8> = buffer.drain(..=end).collect();
                            let line = String::from_utf8_lossy(&line).trim_end().to_owned();
                            if line.is_empty() {
                                continue;
                            }
                            // A full queue means whoever reads has fallen behind. The line
                            // is dropped rather than waited on — see BACKLOG.
                            if tx.try_send(line).is_err() && tx.is_closed() {
                                return;
                            }
                        }
                    }
                    // The error stream is not passed on but is worth knowing about: this is
                    // where "no such file" turns up when the serving keeps its log
                    // somewhere else.
                    ChannelMsg::ExtendedData { ref data, ext: 1 } => {
                        let text = String::from_utf8_lossy(data);
                        let text = text.trim();
                        if !text.is_empty() {
                            tracing::debug!(stderr = %text, "the followed command complained");
                        }
                    }
                    ChannelMsg::ExitStatus { exit_status } => {
                        tracing::debug!(exit_status, "the followed command ended");
                    }
                    _ => {}
                }
            }

            let _ = channel.close().await;
        });

        Ok(rx)
    }
}
