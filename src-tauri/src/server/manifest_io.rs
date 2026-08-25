//! T045 — reading and writing the library catalogue on the server.
//!
//! The order of writing is not optional (R-10, `contracts/server-contract.md`): read
//! with the generation, change, write to a staged file beside it, replace atomically.
//! And before the replacement the generation on the server is checked once more:
//! otherwise a second copy of the application quietly wipes out the first one's work.
//!
//! Why "beside" rather than "over": writing over is a window in which a half-written
//! file sits on the server. Should the connection break exactly there, the library is
//! lost not by half but entirely — there is nothing to parse truncated JSON with.

use super::{join_remote, shell_quote};
use crate::domain::manifest::Manifest;
use crate::ssh::Connection;

/// The name of the catalogue file inside the serving directory.
pub const MANIFEST_NAME: &str = "library.json";

#[derive(Debug, thiserror::Error)]
pub enum ManifestIoError {
    /// The catalogue was changed by another copy of the application between the read
    /// and the write. The write **did not happen**: the other change stands.
    #[error("the catalogue was changed by another application: read generation {base}, server has {current}")]
    Conflict { base: u64, current: u64 },

    #[error("catalogue could not be parsed: {0}")]
    Malformed(String),

    #[error(transparent)]
    Ssh(#[from] crate::ssh::SshError),
}

pub type Result<T> = std::result::Result<T, ManifestIoError>;

/// Read the catalogue. A missing file is an empty library, not an error.
pub async fn read(conn: &Connection, video_dir: &str) -> Result<Manifest> {
    let path = join_remote(video_dir, MANIFEST_NAME);
    let sftp = conn.sftp().await?;

    let bytes = match sftp.read(path.clone()).await {
        Ok(b) => b,
        // Telling "no file" from "no access" by the shape of the library's error is
        // not reliable enough, so the server is asked directly. Treating any failed
        // read as an empty library is dangerous: the application would decide there
        // is no catalogue and wipe out the real one with its very next write.
        Err(e) => {
            let exists = conn
                .exec(&format!("test -e {}", shell_quote(&path)))
                .await?
                .ok();
            if exists {
                return Err(ManifestIoError::Ssh(crate::ssh::SshError::sftp(
                    crate::store::redact::safe_display(&e),
                )));
            }
            return Ok(Manifest::empty());
        }
    };

    let text = String::from_utf8_lossy(&bytes);
    Manifest::parse(&text).map_err(|e| ManifestIoError::Malformed(e.to_string()))
}

/// Write the catalogue if the server still holds `base_generation`.
///
/// `manifest.generation` must be exactly one more than `base_generation` — that is the
/// claim "I am writing over what I read" (see `Manifest::prepared_for_write`).
pub async fn write(
    conn: &Connection,
    video_dir: &str,
    manifest: &Manifest,
    base_generation: u64,
) -> Result<()> {
    // The check comes BEFORE the staged file is created. Otherwise a refusal would
    // leave litter in the serving directory — and a person sees that directory as
    // their library.
    let current = read(conn, video_dir).await?.generation;
    if !Manifest::write_allowed(base_generation, current) {
        return Err(ManifestIoError::Conflict {
            base: base_generation,
            current,
        });
    }

    let target = join_remote(video_dir, MANIFEST_NAME);
    // The staged file's name belongs to this attempt rather than being shared: two
    // copies that reach the write at the same moment must not write into one file.
    let temp = join_remote(
        video_dir,
        &format!(".{MANIFEST_NAME}.{}.tmp", uuid::Uuid::new_v4().simple()),
    );

    let body = manifest.to_json();
    let sftp = conn.sftp().await?;

    // `create` specifically: the library's `write` opens a file for writing without
    // creating it, and on a path that does not exist gives "no such file". The name
    // promises one thing and the behaviour is another — caught on a live server on
    // 2026-08-25.
    let written = async {
        use tokio::io::AsyncWriteExt;
        let mut file = sftp.create(temp.clone()).await?;
        file.write_all(body.as_bytes()).await?;
        file.flush().await?;
        file.shutdown().await?;
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;

    if let Err(e) = written {
        // Cleaned up by us: a staged file in the serving directory lands in the
        // person's "not recognised" group and alarms them.
        let _ = sftp.remove_file(temp.clone()).await;
        return Err(ManifestIoError::Ssh(crate::ssh::SshError::sftp(
            crate::store::redact::safe_display(&*e),
        )));
    }

    // Replacement by renaming specifically: it is atomic within a file system — a
    // reader sees either the whole old catalogue or the whole new one.
    let moved = conn
        .exec(&format!(
            "mv -f -- {} {}",
            shell_quote(&temp),
            shell_quote(&target)
        ))
        .await?;
    if !moved.ok() {
        let _ = sftp.remove_file(temp).await;
        return Err(ManifestIoError::Ssh(crate::ssh::SshError::Exec(format!(
            "the catalogue was not replaced: {}",
            moved.stderr.trim()
        ))));
    }

    tracing::info!(
        generation = manifest.generation,
        media = manifest.media.len(),
        "library catalogue written"
    );
    Ok(())
}
