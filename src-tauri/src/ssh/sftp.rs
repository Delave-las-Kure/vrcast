//! T025 — file operations on the server.
//!
//! A separate channel inside the same connection. This is what resumable transfer
//! (R-05) is built on: writing into a staged file at an offset, rather than cutting
//! the file into pieces.

use super::{Connection, Result, SshError};
use crate::store::redact;
use russh_sftp::client::SftpSession;

/// A file session on the server.
///
/// It holds a channel slot for as long as it lives: a connection has a limit on how
/// many channels are open at once, and a file session takes one of them for a long
/// while rather than for a moment.
pub struct Sftp {
    session: SftpSession,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

impl std::ops::Deref for Sftp {
    type Target = SftpSession;
    fn deref(&self) -> &Self::Target {
        &self.session
    }
}

impl Connection {
    /// Open a file session.
    ///
    /// There can be several at once — each takes its own channel, but not a new
    /// connection. When the limit is reached the call waits rather than refusing.
    pub async fn sftp(&self) -> Result<Sftp> {
        let permit = self.acquire_channel().await?;

        let channel = self.open_session().await?;

        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(SshError::protocol)?;

        let session = SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| SshError::sftp(redact::safe_display(&e)))?;

        Ok(Sftp {
            session,
            _permit: permit,
        })
    }
}
