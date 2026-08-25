//! T045 — чтение и запись описи библиотеки на сервере.
//!
//! Порядок записи обязателен (R-10, `contracts/server-contract.md`):
//! прочитать с поколением → изменить → записать во временный файл рядом → атомарно
//! заменить. И перед заменой поколение на сервере проверяется ещё раз: иначе второй
//! экземпляр приложения молча сотрёт работу первого.
//!
//! Почему «рядом», а не поверх: запись поверх — это окно, в котором на сервере лежит
//! наполовину записанный файл. Оборвись связь именно там — библиотека окажется
//! потеряна не наполовину, а целиком: разобрать обрезанный JSON нечем.

use super::{join_remote, shell_quote};
use crate::domain::manifest::Manifest;
use crate::ssh::Connection;

/// Имя файла описи внутри каталога раздачи.
pub const MANIFEST_NAME: &str = "library.json";

#[derive(Debug, thiserror::Error)]
pub enum ManifestIoError {
    /// Опись изменена другим экземпляром приложения между чтением и записью.
    /// Запись **не выполнена**: на сервере осталось чужое изменение.
    #[error("опись изменена другим приложением: прочитано поколение {base}, на сервере {current}")]
    Conflict { base: u64, current: u64 },

    #[error("опись не разобрать: {0}")]
    Malformed(String),

    #[error(transparent)]
    Ssh(#[from] crate::ssh::SshError),
}

pub type Result<T> = std::result::Result<T, ManifestIoError>;

/// Прочитать опись. Отсутствие файла — пустая библиотека, а не ошибка.
pub async fn read(conn: &Connection, video_dir: &str) -> Result<Manifest> {
    let path = join_remote(video_dir, MANIFEST_NAME);
    let sftp = conn.sftp().await?;

    let bytes = match sftp.read(path.clone()).await {
        Ok(b) => b,
        // Отличить «файла нет» от «нет доступа» по виду ошибки библиотеки нельзя
        // достаточно надёжно, поэтому спрашиваем сервер прямо. Считать любую
        // неудачу чтения пустой библиотекой опасно: приложение решило бы, что
        // описи нет, и следующей же записью стёрло бы настоящую.
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

/// Записать опись, если на сервере всё ещё `base_generation`.
///
/// `manifest.generation` обязан быть на единицу больше `base_generation` — это
/// заявка «записываю поверх того, что прочитал» (см. `Manifest::prepared_for_write`).
pub async fn write(
    conn: &Connection,
    video_dir: &str,
    manifest: &Manifest,
    base_generation: u64,
) -> Result<()> {
    // Проверка идёт ПЕРЕД созданием временного файла. Иначе после отказа в каталоге
    // раздачи оставался бы мусор, а каталог этот пользователь видит как библиотеку.
    let current = read(conn, video_dir).await?.generation;
    if !Manifest::write_allowed(base_generation, current) {
        return Err(ManifestIoError::Conflict {
            base: base_generation,
            current,
        });
    }

    let target = join_remote(video_dir, MANIFEST_NAME);
    // Имя временного файла привязано к попытке, а не общее: два экземпляра, дошедшие
    // до записи одновременно, не должны писать в один и тот же временный файл.
    let temp = join_remote(
        video_dir,
        &format!(".{MANIFEST_NAME}.{}.tmp", uuid::Uuid::new_v4().simple()),
    );

    let body = manifest.to_json();
    let sftp = conn.sftp().await?;

    // Создаём именно `create`: у библиотеки `write` открывает файл только на запись,
    // без создания, и на несуществующем пути даёт «нет такого файла».
    // Имя обещает одно, поведение другое — поймано на живом сервере 2026-08-25.
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
        // Убираем за собой сами: временный файл в каталоге раздачи попадёт
        // пользователю в группу «не распознано» и будет его пугать.
        let _ = sftp.remove_file(temp.clone()).await;
        return Err(ManifestIoError::Ssh(crate::ssh::SshError::sftp(
            crate::store::redact::safe_display(&*e),
        )));
    }

    // Замена именно переименованием: оно атомарно в пределах файловой системы —
    // читающий видит либо старую опись целиком, либо новую целиком.
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
            "опись не заменилась: {}",
            moved.stderr.trim()
        ))));
    }

    tracing::info!(
        поколение = manifest.generation,
        медиа = manifest.media.len(),
        "опись библиотеки записана"
    );
    Ok(())
}
