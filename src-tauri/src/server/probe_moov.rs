//! T044a — параметры файла из его заголовка, без скачивания целиком (FR-012, R-19).
//!
//! Берём начало файла, разбираем `moov` (см. `domain::moov`) и складываем результат
//! в локальную базу. Кеш здесь не украшение: ради каждого файла надо забрать с сервера
//! сотни килобайт, и перечитывать это при каждом открытии библиотеки значило бы
//! качать десятки мегабайт на ровном месте.
//!
//! Ключ кеша включает размер файла. Файл заменили — размер почти наверняка другой,
//! и параметры читаются заново: та же длительность при другом размере дала бы
//! неверный битрейт, а это как раз то число, по которому подбирают ступени качества.

use crate::domain::moov::{self, MediaParams, MoovOutcome};
use crate::ssh::{Connection, Result, SshError};
use crate::store::db::{now_rfc3339, Db};

/// Разобранные параметры файла вместе с признаком пригодности к раздаче.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileParams {
    pub params: MediaParams,
    /// `moov` в начале файла. `None` = вопрос остался открытым.
    pub faststart_ok: Option<bool>,
}

/// Сколько раз готовы дочитывать заголовок.
///
/// Разбор сам говорит, сколько байт ему нужно, и с каждой попыткой запрашивается
/// строго больше — поэтому кругов нужно немного. Предел стоит на случай файла,
/// собранного так, чтобы просить бесконечно.
const MAX_FETCHES: usize = 4;

/// Прочитать параметры файла, взяв их из кеша, если он свеж.
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

    // Неудача записи в кеш — не повод не отдать разобранное: кеш ускоряет, но
    // ничего не решает.
    if let Err(e) = cache_put(db, server_id, name, size_bytes, &result) {
        tracing::warn!(file = name, error = %e, "параметры файла не сохранились в кеш");
    }
    Ok(result)
}

/// Забрать начало файла и разобрать его, дочитывая ровно столько, сколько попросят.
async fn probe(conn: &Connection, path: &str, size_bytes: u64) -> Result<MoovOutcome> {
    let mut want = moov::SUGGESTED_HEAD_BYTES.min(size_bytes.max(1));

    for _ in 0..MAX_FETCHES {
        let head = read_head(conn, path, want).await?;
        match moov::parse(&head, Some(size_bytes)) {
            MoovOutcome::NeedMoreBytes { need } if need > want && need <= size_bytes => {
                want = need;
            }
            // Просят больше, чем есть в файле, либо не больше, чем уже прочитано, —
            // дальше круг не сдвинется, и ответ надо давать по имеющемуся.
            MoovOutcome::NeedMoreBytes { .. } => return Ok(MoovOutcome::NotMp4),
            other => return Ok(other),
        }
    }
    Ok(MoovOutcome::NotMp4)
}

/// Прочитать первые `bytes` байт файла.
async fn read_head(conn: &Connection, path: &str, bytes: u64) -> Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;

    let sftp = conn.sftp().await?;
    let file = sftp
        .open(path.to_owned())
        .await
        .map_err(|e| SshError::Sftp(crate::store::redact::safe_display(&e)))?;

    let mut buf = Vec::with_capacity(bytes as usize);
    file.take(bytes)
        .read_to_end(&mut buf)
        .await
        .map_err(|e| SshError::Sftp(crate::store::redact::safe_display(&e)))?;
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
