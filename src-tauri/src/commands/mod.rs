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

pub mod error;
pub mod events;

use crate::store::db::Db;
use crate::store::secrets::{OsSecretStore, SecretStore};
use crate::tasks::engine::TaskEngine;
use crate::tasks::state::PauseKind;
use crate::tasks::store::TaskRecord;
use error::{AppError, ErrorCode, Result};
use serde::Serialize;
use std::sync::Arc;

/// Общее состояние приложения. Всё, что нужно командам, лежит здесь.
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Db>,
    pub tasks: TaskEngine,
    pub secrets: Arc<dyn SecretStore>,
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

        Ok(Self { db, tasks, secrets })
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
    pub fn tasks_on_close(state: State<'_, AppState>) -> Result<Vec<TaskOnClose>> {
        api::tasks_on_close(&state)
    }

    #[tauri::command]
    pub async fn server_probe_fingerprint(host: String, port: u16) -> Result<String> {
        api::server_probe_fingerprint(&host, port).await
    }
}
