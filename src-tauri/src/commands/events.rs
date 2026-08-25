//! T013 — поток событий из ядра в интерфейс.
//!
//! Интерфейс не опрашивает состояние, а слушает. Иначе показать продвижение
//! многочасовой задачи можно было бы только частым опросом, и он же стал бы причиной
//! подтормаживания, которого мы избегаем (FR-080, SC-009).
//!
//! Имена событий закреплены договором `contracts/ipc-commands.md`.

use crate::commands::{AppEvent, AppState};
use crate::tasks::engine::{TaskEngine, TaskEvent};
use crate::tasks::state::{TaskKind, TaskState};
use tauri::{AppHandle, Emitter, Manager};

/// С какой продолжительности задача считается длительной (FR-084).
///
/// Уведомлять о каждой мелочи — значит приучить закрывать уведомления не читая,
/// и тогда важное пройдёт мимо вместе с остальными. Полминуты — примерно та граница,
/// за которой человек успевает переключиться на другое дело и забыть про задачу.
const LONG_TASK: std::time::Duration = std::time::Duration::from_secs(30);

/// Имена событий. Строки закреплены договором — менять их нельзя, не изменив договор.
pub mod names {
    pub const TASK_PROGRESS: &str = "task:progress";
    pub const TASK_DONE: &str = "task:done";
    /// The core has decided a system notification is warranted; the interface
    /// composes the text and shows it.
    pub const TASK_NOTIFY: &str = "task:notify";
    pub const LIBRARY_CHANGED: &str = "library:changed";
    pub const SERVER_STATE: &str = "server:state";
    pub const VIEWERS_UPDATE: &str = "viewers:update";
}

/// Начать пересылку событий задач в интерфейс.
///
/// Ядро ничего не знает про оболочку: оно рассылает события в свой канал, а этот мост
/// перекладывает их наружу. Так ядро остаётся проверяемым без запуска окна.
pub fn bridge_task_events(app: AppHandle, engine: &TaskEngine) {
    let mut rx = engine.subscribe();
    // Исполнитель берётся у оболочки, а не свой. Прямой вызов tokio::spawn здесь роняет
    // приложение при запуске: подготовка оболочки идёт вне исполнителя, и реактора ещё
    // нет (поймано запуском 2026-08-25 — сборка и тесты этого не показывали).
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let name = match &event {
                        TaskEvent::Progress { .. } => names::TASK_PROGRESS,
                        TaskEvent::Done { .. } => names::TASK_DONE,
                    };
                    if let Err(e) = app.emit(name, &event) {
                        tracing::debug!(error = %e, "событие не доставлено в интерфейс");
                    }
                    if let TaskEvent::Done { id, state, error } = &event {
                        notify_if_long(&app, id, *state, error.as_ref());
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    // Интерфейс не поспевает за потоком. Терять события о продвижении
                    // не страшно — важные (завершение, ошибка) придут следующими и
                    // приведут показ в соответствие.
                    tracing::debug!(skipped, "интерфейс отстал, часть событий пропущена");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

/// Ask for a system notification about a long task that has ended (FR-084).
///
/// The notification is asked for **only when the window is out of sight**: someone
/// looking at the task list has already seen it, and a second message about the same
/// thing is a nuisance rather than a courtesy. And only for tasks that ran longer than
/// [`LONG_TASK`]: a thirty-gigabyte upload takes hours and a person gets on with
/// something else meanwhile, while probing a source file takes a second.
///
/// The text is **not** composed here. The core no longer writes prose in any language
/// (FR-105, FR-106), so what goes out is the fact — which task, how it ended — and the
/// interface words it from the catalogue of the language in use. Showing the
/// notification from the core would mean a second set of wordings that could drift
/// from the first.
fn notify_if_long(
    app: &AppHandle,
    id: &str,
    state: TaskState,
    error: Option<&crate::error::AppError>,
) {
    // The window is in front of them — they can see it for themselves.
    let visible = app
        .get_webview_window("main")
        .map(|w| {
            w.is_focused().unwrap_or(false)
                && w.is_visible().unwrap_or(false)
                && !w.is_minimized().unwrap_or(false)
        })
        .unwrap_or(false);
    if visible {
        return;
    }

    let Some(state_handle) = app.try_state::<AppState>() else {
        return;
    };
    let Ok(Some(task)) = state_handle.tasks.get(id) else {
        return;
    };
    if !long_enough(&task.created_at, &task.updated_at) {
        return;
    }
    // Cancelling is something a person does themselves and already knows about.
    if !matches!(state, TaskState::Completed | TaskState::Failed) {
        return;
    }

    let request = NotifyRequest {
        id: id.to_owned(),
        kind: task.kind,
        state,
        error: error.cloned(),
    };
    if let Err(e) = app.emit(names::TASK_NOTIFY, &request) {
        tracing::debug!(error = %e, "просьба об уведомлении не доставлена");
    }
}

/// What the interface needs in order to word a notification.
#[derive(Debug, Clone, serde::Serialize)]
pub struct NotifyRequest {
    pub id: String,
    pub kind: TaskKind,
    pub state: TaskState,
    /// Present when the task failed. Worded by the interface, like any other error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<crate::error::AppError>,
}

/// Шла ли задача дольше порога.
///
/// Время берётся из записи: отдельного счёта не нужно, а `created_at` и `updated_at`
/// уже пишутся. Неразобранное время считается коротким — уведомить лишний раз хуже,
/// чем промолчать, потому что лишние уведомления учат не читать их вовсе.
pub fn long_enough(created_at: &str, updated_at: &str) -> bool {
    let (Ok(from), Ok(to)) = (
        crate::store::db::parse_rfc3339(created_at),
        crate::store::db::parse_rfc3339(updated_at),
    ) else {
        return false;
    };
    to.saturating_sub(from) >= LONG_TASK.as_secs()
}

/// Начать пересылку прочих событий ядра в интерфейс.
pub fn bridge_app_events(app: AppHandle, state: &AppState) {
    let mut rx = state.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let name = match &event {
                        AppEvent::LibraryChanged { .. } => names::LIBRARY_CHANGED,
                    };
                    if let Err(e) = app.emit(name, &event) {
                        tracing::debug!(error = %e, "событие не доставлено в интерфейс");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::debug!(skipped, "интерфейс отстал, часть событий пропущена");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}
