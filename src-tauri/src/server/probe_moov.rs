//! T044a — a file's particulars from its header, without downloading it all
//! (FR-012, R-19).
//!
//! The start of the file is fetched, `moov` is parsed (see `domain::moov`), and the
//! result goes into the local database. The cache is not decoration here: each file
//! costs hundreds of kilobytes off the server, and re-reading that every time the
//! library opens would mean downloading tens of megabytes for nothing.
//!
//! The cache key includes the file size. If a file was replaced, its size is almost
//! certainly different and the particulars are read again: the same duration at a
//! different size would give the wrong bitrate — and that is exactly the number
//! quality rungs are chosen by.

use crate::domain::moov::{self, MediaParams, MoovOutcome};
use crate::ssh::{Connection, Result, SshError};
use crate::store::db::{now_rfc3339, Db};

/// A file's parsed particulars, along with whether it is fit to serve.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileParams {
    pub params: MediaParams,
    /// `moov` at the start of the file. `None` means the question stayed open.
    pub faststart_ok: Option<bool>,
}

/// How many times we are willing to fetch more of the header.
///
/// The parser says how many bytes it needs, and each attempt asks for strictly more —
/// so few rounds are needed. The limit is there for a file built to ask forever.
const MAX_FETCHES: usize = 4;

/// Read a file's particulars, taking them from the cache if it is fresh.
pub async fn params_for(
    conn: &Connection,
    db: &Db,
    server_id: &str,
    video_dir: &str,
    name: &str,
    size_bytes: u64,
) -> Result<FileParams> {
    if let Some(cached) = cache_get(db, server_id, name, size_bytes) {
        return Ok(cached);
    }

    let path = super::join_remote(video_dir, name);
    let outcome = probe(conn, &path, size_bytes).await?;

    let result = FileParams {
        params: outcome.params().cloned().unwrap_or_default(),
        faststart_ok: outcome.faststart_ok(),
    };

    // A failed cache write is no reason to withhold what was parsed: the cache makes
    // things faster and decides nothing.
    if let Err(e) = cache_put(db, server_id, name, size_bytes, &result) {
        tracing::warn!(file = name, error = %e, "file particulars were not cached");
    }
    Ok(result)
}

/// Fetch the start of a file and parse it, reading exactly as much more as is asked for.
async fn probe(conn: &Connection, path: &str, size_bytes: u64) -> Result<MoovOutcome> {
    let mut want = moov::SUGGESTED_HEAD_BYTES.min(size_bytes.max(1));

    for _ in 0..MAX_FETCHES {
        let head = read_head(conn, path, want).await?;
        match moov::parse(&head, Some(size_bytes)) {
            MoovOutcome::NeedMoreBytes { need } if need > want && need <= size_bytes => {
                want = need;
            }
            // Either more is asked for than the file holds, or no more than has
            // already been read — another round moves nothing, and the answer has to
            // be given from what there is.
            MoovOutcome::NeedMoreBytes { .. } => return Ok(MoovOutcome::NotMp4),
            other => return Ok(other),
        }
    }
    Ok(MoovOutcome::NotMp4)
}

/// Read the first `bytes` bytes of a file.
async fn read_head(conn: &Connection, path: &str, bytes: u64) -> Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;

    let sftp = conn.sftp().await?;
    let file = sftp
        .open(path.to_owned())
        .await
        .map_err(|e| SshError::sftp(crate::store::redact::safe_display(&e)))?;

    let mut buf = Vec::with_capacity(bytes as usize);
    file.take(bytes)
        .read_to_end(&mut buf)
        .await
        .map_err(|e| SshError::sftp(crate::store::redact::safe_display(&e)))?;
    Ok(buf)
}

fn cache_get(db: &Db, server_id: &str, path: &str, size_bytes: u64) -> Option<FileParams> {
    db.with_conn(|c| {
        let mut stmt = c.prepare(
            "SELECT duration_s, width, height, bitrate_bps, video_codec, audio_codec, faststart_ok
             FROM file_params WHERE server_id = ?1 AND path = ?2 AND size_bytes = ?3",
        )?;
        let mut rows = stmt.query(rusqlite::params![server_id, path, size_bytes as i64])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(FileParams {
            params: MediaParams {
                duration_s: row.get(0)?,
                width: row.get::<_, Option<i64>>(1)?.map(|v| v as u32),
                height: row.get::<_, Option<i64>>(2)?.map(|v| v as u32),
                bitrate_bps: row.get::<_, Option<i64>>(3)?.map(|v| v as u64),
                video_codec: row.get(4)?,
                audio_codec: row.get(5)?,
            },
            faststart_ok: row.get::<_, Option<i64>>(6)?.map(|v| v != 0),
        }))
    })
    .ok()
    .flatten()
}

fn cache_put(
    db: &Db,
    server_id: &str,
    path: &str,
    size_bytes: u64,
    value: &FileParams,
) -> std::result::Result<(), crate::store::db::DbError> {
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO file_params
                (server_id, path, size_bytes, duration_s, width, height, bitrate_bps,
                 video_codec, audio_codec, faststart_ok, probed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT (server_id, path) DO UPDATE SET
                size_bytes = excluded.size_bytes,
                duration_s = excluded.duration_s,
                width = excluded.width,
                height = excluded.height,
                bitrate_bps = excluded.bitrate_bps,
                video_codec = excluded.video_codec,
                audio_codec = excluded.audio_codec,
                faststart_ok = excluded.faststart_ok,
                probed_at = excluded.probed_at",
            rusqlite::params![
                server_id,
                path,
                size_bytes as i64,
                value.params.duration_s,
                value.params.width.map(i64::from),
                value.params.height.map(i64::from),
                value.params.bitrate_bps.map(|v| v as i64),
                value.params.video_codec,
                value.params.audio_codec,
                value.faststart_ok.map(i64::from),
                now_rfc3339(),
            ],
        )?;
        Ok(())
    })
}
