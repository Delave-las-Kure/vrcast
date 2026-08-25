//! T013 — слой команд: единственная граница между интерфейсом и ядром.
//!
//! Интерфейс не знает ни про SSH, ни про FFmpeg, ни про устройство сервера. Он вызывает
//! команды и слушает поток событий — больше ничего.
//!
//! **Команды устроены двухслойно**, и это не лишняя прослойка. Внутри — обычные функции
//! (`api`), не знающие про оболочку приложения; снаружи — тонкие обёртки, которые оболочка
//! выставляет наружу. Так договор проверяется тестами напрямую, без запуска окна: иначе
//! договорные тесты потребовали бы живого приложения с графикой, а в непрерывной
//! интеграции его нет.

pub mod convert;
pub mod error;
pub mod events;
pub mod library;
pub mod servers;
pub mod upload;

use crate::store::db::Db;
use crate::store::secrets::{OsSecretStore, SecretStore};
use crate::tasks::engine::TaskEngine;
use crate::tasks::state::PauseKind;
use crate::tasks::store::TaskRecord;
use error::{AppError, ErrorCode, Result};
use serde::Serialize;
use std::sync::Arc;

/// Событие о состоянии приложения, не связанное с задачами.
///
/// Ядро рассылает их в свой канал, а оболочка пересылает наружу (см. `events`).
/// Так ядро не знает про окно и остаётся проверяемым без запуска графики.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum AppEvent {
    /// Библиотека сервера изменилась: перечитайте её.
    LibraryChanged { server_id: String },
}

/// Общее состояние приложения. Всё, что нужно командам, лежит здесь.
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Db>,
    pub tasks: TaskEngine,
    pub secrets: Arc<dyn SecretStore>,
    /// Канал событий, не связанных с задачами.
    events: tokio::sync::broadcast::Sender<AppEvent>,
}

impl AppState {
    /// Собрать состояние с настоящими хранилищами и разобрать последствия прошлого запуска.
    pub fn bootstrap() -> Result<Self> {
        let path = Db::default_path()?;
        let db = Arc::new(Db::open(path)?);
        Self::with_db(db, Arc::new(OsSecretStore::new()))
    }

    /// То же, но с заданными хранилищами — для тестов.
    pub fn with_db(db: Arc<Db>, secrets: Arc<dyn SecretStore>) -> Result<Self> {
        let tasks = TaskEngine::new(db.clone());

        // Порядок важен. Сначала добиваем программы, уцелевшие от прошлого запуска, и
        // только потом разбираем задачи: иначе задача будет объявлена приостановленной,
        // пока её процесс ещё жив и продолжает писать в файл результата.
        match crate::tasks::registry::sweep_on_startup(&db) {
            Ok(report) if !report.is_clean() => {
                tracing::warn!(
                    добито = report.killed.len(),
                    пропущено = report.reused.len(),
                    "уборка программ, уцелевших от прошлого запуска"
                );
            }
            Ok(_) => {}
            Err(e) => tracing::error!(error = %e, "уборка при запуске не удалась"),
        }

        if let Ok(report) = tasks.recover_after_start() {
            if !report.interrupted.is_empty() {
                tracing::warn!(
                    count = report.interrupted.len(),
                    "задачи прошлого запуска приостановлены и ждут продолжения"
                );
            }
        }

        let (events, _) = tokio::sync::broadcast::channel(64);
        Ok(Self {
            db,
            tasks,
            secrets,
            events,
        })
    }

    /// Подписаться на события приложения.
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<AppEvent> {
        self.events.subscribe()
    }

    /// Сообщить, что библиотека сервера изменилась.
    ///
    /// Отсутствие слушателей — норма, а не ошибка: события уходят в никуда, пока
    /// интерфейс не открыт, и падать из-за этого было бы нелепо.
    pub fn notify_library_changed(&self, server_id: &str) {
        let _ = self.events.send(AppEvent::LibraryChanged {
            server_id: server_id.to_owned(),
        });
    }
}

/// Версии приложения и серверной части (FR-128).
#[derive(Debug, Clone, Serialize)]
pub struct Versions {
    pub app: String,
    /// Версия серверной части активного сервера. Появится в Фазе 7.
    pub server: Option<u32>,
    /// Версия локального хранилища — нужна при разборе неполадок.
    pub schema: u32,
}

/// Что станет с задачей, если закрыть приложение (FR-086).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct TaskOnClose {
    pub id: String,
    pub kind: String,
    pub progress: f64,
    /// `resumes` — продолжится с достигнутого места; `restarts` — начнётся заново.
    pub outcome: &'static str,
    /// Готовая к показу строка: общего «идут задачи, закрыть?» недостаточно,
    /// оно не даёт принять решение.
    pub explanation: String,
}

/// Обычные функции — то, что на самом деле выполняется.
pub mod api {
    use super::*;

    pub fn app_versions(state: &AppState) -> Result<Versions> {
        Ok(Versions {
            app: env!("CARGO_PKG_VERSION").to_owned(),
            server: None,
            schema: state.db.schema_version()?,
        })
    }

    pub fn tasks_list(state: &AppState) -> Result<Vec<TaskRecord>> {
        Ok(state.tasks.list()?)
    }

    pub fn task_get(state: &AppState, id: &str) -> Result<TaskRecord> {
        state
            .tasks
            .get(id)?
            .ok_or_else(|| AppError::new(ErrorCode::TaskNotFound).with_cause(id))
    }

    pub fn task_cancel(state: &AppState, id: &str) -> Result<()> {
        Ok(state.tasks.cancel(id)?)
    }

    pub fn task_pause(state: &AppState, id: &str) -> Result<()> {
        Ok(state.tasks.pause(id)?)
    }

    pub fn task_resume(state: &AppState, id: &str) -> Result<()> {
        Ok(state.tasks.resume(id)?)
    }

    /// Переставить ждущие задачи в очереди (FR-083).
    ///
    /// `ordered` — номера задач в желаемом порядке. Выполняющиеся не трогаются:
    /// прервать начатую передачу ради изменения порядка значило бы выбросить уже
    /// сделанную работу. Возвращается, сколько задач переставлено, — список
    /// у человека на экране всегда чуть отстаёт, и часть из них могла уже начаться.
    pub fn tasks_reorder(state: &AppState, ordered: &[String]) -> Result<usize> {
        Ok(state.tasks.reorder_queue(ordered)?)
    }

    /// Номера ждущих задач в том порядке, в каком они пойдут в работу.
    pub fn tasks_queue_order(state: &AppState) -> Result<Vec<String>> {
        Ok(state.tasks.queue_order())
    }

    /// Что сказать пользователю при закрытии приложения (FR-086).
    ///
    /// Разница между видами задач здесь не косметическая: передача продолжится с
    /// достигнутого места, а приостановленная подготовка держится живым процессом и
    /// закрытия не переживёт. Пользователь обязан узнать об этом **до** закрытия.
    pub fn tasks_on_close(state: &AppState) -> Result<Vec<TaskOnClose>> {
        use crate::tasks::state::TaskState;

        let mut out = Vec::new();
        for t in state.tasks.list()? {
            if t.state.is_final() {
                continue;
            }
            let percent = (t.progress * 100.0).round() as i64;
            let (outcome, explanation) = match t.kind.pause_kind() {
                PauseKind::ResumableAcrossRestart => (
                    "resumes",
                    format!("продолжится с {percent} % при следующем запуске"),
                ),
                PauseKind::SuspendedProcess => (
                    "restarts",
                    format!("придётся начать заново — потеряется {percent} % работы"),
                ),
                PauseKind::NotPausable => {
                    if t.state == TaskState::Queued {
                        (
                            "resumes",
                            String::from("ещё не начиналась, запустится позже"),
                        )
                    } else {
                        ("restarts", String::from("придётся выполнить заново"))
                    }
                }
            };
            out.push(TaskOnClose {
                id: t.id,
                kind: t.kind.as_str().to_owned(),
                progress: t.progress,
                outcome,
                explanation,
            });
        }
        Ok(out)
    }

    /// Проверить вложенный в поставку FFmpeg (FR-112, T115).
    ///
    /// Вызывается при запуске и перед подготовкой. Вложенный файл может не запуститься:
    /// его вырезал антивирус, установщик распаковался наполовину, у файла нет права
    /// на выполнение. Узнать об этом в начале — значит сказать человеку, что чинить;
    /// узнать в середине двухчасовой подготовки — значит отнять эти два часа.
    pub async fn ffmpeg_probe_self() -> Result<crate::media::ffmpeg::FfmpegInfo> {
        let info = crate::media::ffmpeg::probe_self().await.map_err(|e| {
            AppError::new(ErrorCode::FfmpegBroken)
                .with_message(
                    "Вложенный FFmpeg не работает — готовить файлы нечем.                      Переустановите приложение: возможно, антивирус удалил часть файлов.",
                )
                .with_cause(e.to_string())
        })?;

        // Без программного кодировщика подготовка невозможна на машине без подходящей
        // видеокарты — то есть у части людей приложение молча оказалось бы бесполезным.
        if !info.has_x264 {
            return Err(AppError::new(ErrorCode::FfmpegBroken)
                .with_message(
                    "Вложенный FFmpeg собран без программного кодировщика H.264.                      На машине без подходящей видеокарты готовить файлы будет нечем.",
                )
                .with_cause(info.version));
        }
        Ok(info)
    }

    /// Разобрать исходник: что за файл нам дали (FR-020).
    ///
    /// Быстрая операция, а не задача: разбор занимает доли секунды и человек ждёт
    /// ответа прямо сейчас. Заводить ради него запись в очереди значило бы засорить
    /// список задач тем, что кончается раньше, чем успевает в нём появиться.
    pub async fn source_probe(path: &str) -> Result<crate::domain::source::SourceFile> {
        use crate::media::probe::ProbeError;

        crate::media::probe::probe(std::path::Path::new(path))
            .await
            .map_err(|e| match e {
                ProbeError::Ffmpeg(_) => AppError::new(ErrorCode::FfmpegBroken)
                    .with_message(
                        "Вложенный FFmpeg не работает — разбирать файлы нечем.                          Переустановите приложение: возможно, антивирус удалил часть файлов.",
                    )
                    .with_cause(e.to_string()),
                ProbeError::NoVideo => AppError::new(ErrorCode::InvalidInput)
                    .with_message("В этом файле нет видео — возможно, выбран не тот файл.")
                    .with_cause(path),
                ProbeError::Unreadable(_) => AppError::new(ErrorCode::InvalidInput)
                    .with_message("Файл не удалось разобрать: он повреждён или это не видео.")
                    // Жалоба разборщика оставлена как есть: «moov atom not found»
                    // непонятно, но её можно найти поиском, а «файл плохой» — нельзя.
                    .with_cause(e.to_string()),
            })
    }

    /// Узнать отпечаток сервера, ничего ему не предъявляя (FR-092).
    pub async fn server_probe_fingerprint(host: &str, port: u16) -> Result<String> {
        let addr = crate::ssh::ServerAddress::new(host, port);
        Ok(crate::ssh::fingerprint::probe(&addr).await?)
    }
}

/// Тонкие обёртки, которые оболочка выставляет интерфейсу.
///
/// Здесь не должно быть логики — только преобразование состояния и вызов `api`.
pub mod ipc {
    use super::*;
    use tauri::State;

    #[tauri::command]
    pub fn app_versions(state: State<'_, AppState>) -> Result<Versions> {
        api::app_versions(&state)
    }

    #[tauri::command]
    pub fn tasks_list(state: State<'_, AppState>) -> Result<Vec<TaskRecord>> {
        api::tasks_list(&state)
    }

    #[tauri::command]
    pub fn task_get(state: State<'_, AppState>, id: String) -> Result<TaskRecord> {
        api::task_get(&state, &id)
    }

    #[tauri::command]
    pub fn task_cancel(state: State<'_, AppState>, id: String) -> Result<()> {
        api::task_cancel(&state, &id)
    }

    #[tauri::command]
    pub fn task_pause(state: State<'_, AppState>, id: String) -> Result<()> {
        api::task_pause(&state, &id)
    }

    #[tauri::command]
    pub fn task_resume(state: State<'_, AppState>, id: String) -> Result<()> {
        api::task_resume(&state, &id)
    }

    #[tauri::command]
    pub fn tasks_reorder(state: State<'_, AppState>, ordered: Vec<String>) -> Result<usize> {
        api::tasks_reorder(&state, &ordered)
    }

    #[tauri::command]
    pub fn tasks_queue_order(state: State<'_, AppState>) -> Result<Vec<String>> {
        api::tasks_queue_order(&state)
    }

    #[tauri::command]
    pub fn tasks_on_close(state: State<'_, AppState>) -> Result<Vec<TaskOnClose>> {
        api::tasks_on_close(&state)
    }

    #[tauri::command]
    pub async fn ffmpeg_probe_self() -> Result<crate::media::ffmpeg::FfmpegInfo> {
        api::ffmpeg_probe_self().await
    }

    #[tauri::command]
    pub async fn source_probe(path: String) -> Result<crate::domain::source::SourceFile> {
        api::source_probe(&path).await
    }

    #[tauri::command]
    pub async fn server_probe_fingerprint(host: String, port: u16) -> Result<String> {
        api::server_probe_fingerprint(&host, port).await
    }
}
