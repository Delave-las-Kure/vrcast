//! T086–T092 — передача файла на сервер с продолжением после обрыва (R-05).
//!
//! Устройство: файл пишется во временный **вне каталога раздачи**, окнами
//! по несколько мегабайт, с записью по смещению. Позиция продолжения берётся
//! из размера этого временного файла на сервере. По завершении — сверка
//! контрольных сумм и ввод в раздачу одним переименованием.
//!
//! Почему не нарезка на куски с последующей склейкой (как делал прежний скрипт):
//! куски занимают на диске сервера второй такой же объём, склейка — это ещё один
//! проход по всему файлу, а позиция продолжения требует отдельного учёта. Запись
//! по смещению даёт то же самое даром: позиция — это размер файла.
//!
//! **Один заход, а не вся передача.** Модуль делает попытку и возвращает, докуда
//! дошёл. Переподключение и повторы — этажом выше (`commands::upload`), там же,
//! где известен профиль сервера. Смешивать это здесь значило бы протащить сюда
//! и секреты, и правила повторов.

use super::{join_remote, shell_quote};
use crate::domain::progress_estimate::ProgressEstimate;
use crate::domain::rate_limit::RateLimiter;
use crate::domain::transfer::{decide_resume, ResumeDecision, WINDOW_BYTES};
use crate::ssh::{Connection, SshError};
use crate::tasks::engine::TaskContext;
use russh_sftp::protocol::OpenFlags;
use std::path::PathBuf;
use std::time::Instant;

/// Что и куда передавать.
#[derive(Debug, Clone)]
pub struct UploadPlan {
    pub local_path: PathBuf,
    /// Полный путь временного файла на сервере.
    pub remote_temp: String,
    /// Полный путь конечного файла в каталоге раздачи.
    pub remote_final: String,
    pub total_bytes: u64,
    pub limit_bps: Option<u64>,
}

/// Чем кончился заход.
#[derive(Debug, thiserror::Error)]
pub enum UploadError {
    /// Связь оборвалась. Это не поломка, а обычное дело на многочасовой передаче:
    /// переподключиться и продолжить с достигнутого.
    #[error("передача прервана: {0}")]
    Interrupted(String),

    /// На сервере лежит больше, чем есть в источнике. Продолжать нельзя: получится
    /// склейка двух разных файлов, и обнаружится это только на сверке — когда время
    /// уже потрачено.
    #[error("временный файл на сервере ({temp} Б) больше исходного ({total} Б)")]
    SourceChanged { temp: u64, total: u64 },

    #[error("задача отменена")]
    Cancelled,

    #[error("{0}")]
    Failed(String),
}

impl UploadError {
    /// Стоит ли пробовать снова.
    pub fn is_retriable(&self) -> bool {
        matches!(self, Self::Interrupted(_))
    }
}

pub type Result<T> = std::result::Result<T, UploadError>;

impl From<SshError> for UploadError {
    fn from(e: SshError) -> Self {
        let text = crate::store::redact::safe_display(&e);
        match e {
            // Обрыв и всё, что с ним связано, — повод повторить.
            SshError::Unreachable { .. } | SshError::Protocol(_) | SshError::Sftp(_) => {
                Self::Interrupted(text)
            }
            _ => Self::Failed(text),
        }
    }
}

/// Подготовить каталог сборки и убедиться, что он на той же файловой системе.
///
/// Проверка не формальность: переименование неделимо только внутри одной файловой
/// системы. Через границу оно превращается в копирование с удалением — то есть
/// в те самые минуты, когда в каталоге раздачи лежит наполовину скопированный файл.
/// Молча получить это вместо неделимого ввода в раздачу — худший исход, потому что
/// проявится он у зрителя.
pub async fn ensure_staging(conn: &Connection, staging_dir: &str, video_dir: &str) -> Result<()> {
    let out = conn
        .exec(&format!(
            "mkdir -p -- {staging} && stat -c %d {staging} && stat -c %d {videos}",
            staging = shell_quote(staging_dir),
            videos = shell_quote(video_dir)
        ))
        .await?;

    if !out.ok() {
        return Err(UploadError::Failed(format!(
            "не подготовить каталог сборки {staging_dir}: {}",
            out.stderr.trim()
        )));
    }

    let mut lines = out.stdout.lines();
    let staging_fs = lines.next().unwrap_or_default().trim().to_owned();
    let videos_fs = lines.next().unwrap_or_default().trim().to_owned();

    if staging_fs.is_empty() || videos_fs.is_empty() {
        return Err(UploadError::Failed(String::from(
            "не удалось узнать файловую систему каталогов на сервере",
        )));
    }
    if staging_fs != videos_fs {
        return Err(UploadError::Failed(format!(
            "каталог сборки {staging_dir} и каталог раздачи {video_dir} — на разных файловых \
             системах. Ввод в раздачу перестал бы быть неделимым, и зритель мог бы получить \
             наполовину скопированный файл"
        )));
    }
    Ok(())
}

/// Сколько уже лежит во временном файле на сервере.
pub async fn uploaded_so_far(conn: &Connection, remote_temp: &str) -> Result<u64> {
    // Отсутствие файла — законный ответ «ноль», а не ошибка: так выглядит первая
    // попытка. Поэтому спрашиваем размер командой, которая на этот случай молчит.
    let out = conn
        .exec(&format!(
            "stat -c %s -- {} 2>/dev/null || echo 0",
            shell_quote(remote_temp)
        ))
        .await?;
    Ok(out.trimmed().trim().parse::<u64>().unwrap_or(0))
}

/// Один заход передачи. Возвращает, сколько всего лежит во временном файле.
///
/// `estimate` передаётся снаружи и переживает переподключения: иначе после каждого
/// обрыва оценка времени начиналась бы с нуля и первые секунды показывала бы
/// бессмыслицу.
pub async fn transfer_once(
    conn: &Connection,
    ctx: &TaskContext,
    plan: &UploadPlan,
    estimate: &mut ProgressEstimate,
) -> Result<u64> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

    let already = uploaded_so_far(conn, &plan.remote_temp).await?;
    let offset = match decide_resume(already, plan.total_bytes, WINDOW_BYTES) {
        ResumeDecision::AlreadyComplete => return Ok(already),
        ResumeDecision::Mismatch { temp, total } => {
            return Err(UploadError::SourceChanged { temp, total })
        }
        ResumeDecision::FromStart => 0,
        ResumeDecision::Continue { offset } => offset,
    };

    let mut local = tokio::fs::File::open(&plan.local_path)
        .await
        .map_err(|e| UploadError::Failed(format!("исходник не открылся: {e}")))?;
    local
        .seek(std::io::SeekFrom::Start(offset))
        .await
        .map_err(|e| UploadError::Failed(format!("не встать на позицию в исходнике: {e}")))?;

    let sftp = conn.sftp().await?;
    let mut remote = sftp
        .open_with_flags(
            plan.remote_temp.clone(),
            OpenFlags::WRITE | OpenFlags::CREATE,
        )
        .await
        .map_err(|e| UploadError::Interrupted(crate::store::redact::safe_display(&e)))?;
    remote
        .seek(std::io::SeekFrom::Start(offset))
        .await
        .map_err(|e| UploadError::Interrupted(format!("не встать на позицию на сервере: {e}")))?;

    let mut limiter = RateLimiter::new(plan.limit_bps);
    let mut sent = offset;
    let mut buf = vec![0u8; WINDOW_BYTES as usize];

    loop {
        // Отмена и приостановка проверяются между окнами: рвать запись посередине
        // значило бы оставить в файле оборванный хвост, который потом придётся
        // переписывать.
        ctx.wait_while_paused().await;
        if ctx.is_cancelled() {
            return Err(UploadError::Cancelled);
        }

        let read = local
            .read(&mut buf)
            .await
            .map_err(|e| UploadError::Failed(format!("исходник не читается: {e}")))?;
        if read == 0 {
            break;
        }

        let wait = limiter.delay_for(read as u64, Instant::now());
        if !wait.is_zero() {
            // Ждём с оглядкой на отмену: иначе при ограничении в сотню килобайт
            // отмена ждала бы своей очереди десятки секунд.
            //
            // Токен именуется отдельной переменной намеренно: временное значение
            // внутри `select!` живёт до конца выражения и до конца ожидания
            // не доживает.
            let cancel = ctx.cancel_token();
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = cancel.cancelled() => return Err(UploadError::Cancelled),
            }
        }

        remote
            .write_all(&buf[..read])
            .await
            .map_err(|e| UploadError::Interrupted(format!("запись на сервер оборвалась: {e}")))?;

        sent += read as u64;
        estimate.record(Instant::now(), sent);
        report(ctx, plan, estimate, sent);
    }

    remote
        .flush()
        .await
        .map_err(|e| UploadError::Interrupted(format!("запись на сервер не дописалась: {e}")))?;
    remote
        .shutdown()
        .await
        .map_err(|e| UploadError::Interrupted(format!("файл на сервере не закрылся: {e}")))?;

    Ok(sent)
}

fn report(ctx: &TaskContext, plan: &UploadPlan, estimate: &ProgressEstimate, sent: u64) {
    let progress = if plan.total_bytes == 0 {
        1.0
    } else {
        sent as f64 / plan.total_bytes as f64
    };
    let remaining = plan.total_bytes.saturating_sub(sent);
    ctx.report_transfer(
        progress,
        estimate.speed_bps().unwrap_or(0) as i64,
        estimate.eta(remaining).map_or(0, |d| d.as_secs() as i64),
    );
    // И отдельно — на диск, много реже. Заливка идёт часами, и после перезапуска
    // приложения человек должен увидеть, сколько уже передано, а не ноль.
    ctx.save_progress(progress);
}

/// Ввести файл в раздачу одним неделимым действием (FR-033).
///
/// До этого мгновения файл по ссылке недоступен: он лежит вне каталога раздачи.
/// После — доступен целиком. Промежуточного состояния нет, и это главное, ради
/// чего вся сборка идёт в стороне.
pub async fn publish(conn: &Connection, plan: &UploadPlan) -> Result<()> {
    let out = conn
        .exec(&format!(
            "mv -f -- {} {}",
            shell_quote(&plan.remote_temp),
            shell_quote(&plan.remote_final)
        ))
        .await?;
    if !out.ok() {
        return Err(UploadError::Failed(format!(
            "файл не удалось ввести в раздачу: {}",
            out.stderr.trim()
        )));
    }
    tracing::info!(file = %plan.remote_final, "файл введён в раздачу");
    Ok(())
}

/// Убрать за собой при отмене (FR-038).
///
/// Ошибка уборки не возвращается: отмена уже произошла, и превращать её в неудачу
/// из-за не удалившегося временного файла незачем. Но и молчать нельзя — иначе
/// мусор копится незаметно.
pub async fn cleanup(conn: &Connection, remote_temp: &str) {
    let result = conn
        .exec(&format!("rm -f -- {}", shell_quote(remote_temp)))
        .await;
    match result {
        Ok(out) if out.ok() => {}
        Ok(out) => {
            tracing::warn!(file = remote_temp, stderr = %out.stderr.trim(), "временный файл не удалился")
        }
        Err(e) => tracing::warn!(file = remote_temp, error = %e, "временный файл не удалился"),
    }
}

/// Полный путь конечного файла в каталоге раздачи.
pub fn final_path(video_dir: &str, remote_name: &str) -> String {
    join_remote(
        video_dir,
        &crate::domain::remote_name::sanitize(remote_name),
    )
}
