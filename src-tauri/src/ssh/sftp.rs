//! T025 — файловые операции на сервере.
//!
//! Отдельный канал в том же соединении. Именно поверх этого будет работать передача
//! с возобновлением (R-05): запись во временный файл по смещению, а не нарезка на куски.

use super::{Connection, Result, SshError};
use crate::store::redact;
use russh_sftp::client::SftpSession;

/// Файловая сессия на сервере.
///
/// Держит место под канал, пока жива: у соединения есть предел на число одновременно
/// открытых каналов, и файловая сессия занимает один из них не на миг, а надолго.
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
    /// Открыть файловую сессию.
    ///
    /// Сессий может быть несколько одновременно — каждая занимает свой канал, но не
    /// новое соединение. При исчерпании предела вызов подождёт, а не откажет.
    pub async fn sftp(&self) -> Result<Sftp> {
        let permit = self.acquire_channel().await?;

        let channel = self
            .handle()
            .channel_open_session()
            .await
            .map_err(SshError::protocol)?;

        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(SshError::protocol)?;

        let session = SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| SshError::Sftp(redact::safe_display(&e)))?;

        Ok(Sftp {
            session,
            _permit: permit,
        })
    }
}
