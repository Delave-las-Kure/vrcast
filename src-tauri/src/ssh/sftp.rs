//! T025 — файловые операции на сервере.
//!
//! Отдельный канал в том же соединении. Именно поверх этого будет работать передача
//! с возобновлением (R-05): запись во временный файл по смещению, а не нарезка на куски.

use super::{Connection, Result, SshError};
use crate::store::redact;
use russh_sftp::client::SftpSession;

impl Connection {
    /// Открыть файловую сессию.
    ///
    /// Сессий может быть несколько одновременно — каждая занимает свой канал, но не
    /// новое соединение.
    pub async fn sftp(&self) -> Result<SftpSession> {
        let channel = self
            .handle()
            .channel_open_session()
            .await
            .map_err(SshError::protocol)?;

        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(SshError::protocol)?;

        SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| SshError::Sftp(redact::safe_display(&e)))
    }
}
