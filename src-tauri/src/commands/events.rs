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
                    if let TaskEvent::Done { id, state, .. } = &event {
                        notify_if_long(&app, id, *state);
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

/// Сообщить системным уведомлением, что длительная задача кончилась (FR-084).
///
/// Уведомление показывается **только когда окна не видно**: если человек смотрит
/// в список задач, он уже всё увидел, и второе сообщение о том же — не забота,
/// а помеха. И только для задач, шедших дольше [`LONG_TASK`]: заливка на тридцать
/// гигабайт идёт часами, и человек за это время успевает заняться другим делом,
/// а разбор исходника укладывается в секунду и никого ждать не заставляет.
///
/// Неудача здесь ничего не ломает: уведомление — любезность, а не работа. На Linux
/// его может не быть вовсе (нет службы уведомлений), и это не повод для сообщений
/// об ошибке.
fn notify_if_long(app: &AppHandle, id: &str, state: TaskState) {
    use tauri_plugin_notification::NotificationExt;

    // Окно на виду — человек уже всё видит сам.
    let видно = app
        .get_webview_window("main")
        .map(|w| {
            w.is_focused().unwrap_or(false)
                && w.is_visible().unwrap_or(false)
                && !w.is_minimized().unwrap_or(false)
        })
        .unwrap_or(false);
    if видно {
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

    let (title, body) = match state {
        TaskState::Completed => ("Задача выполнена", done_text(task.kind)),
        TaskState::Failed => (
            "Задача не удалась",
            task.error
                .clone()
                .unwrap_or_else(|| String::from("Подробности — в разделе «Задачи».")),
        ),
        // Отмену человек делает сам и знает о ней; сообщать ему об этом незачем.
        _ => return,
    };

    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        tracing::debug!(error = %e, "уведомление не показано");
    }
}

fn done_text(kind: TaskKind) -> String {
    match kind {
        TaskKind::Upload => String::from("Файл залит на сервер и введён в раздачу."),
        TaskKind::Convert => String::from("Файл подготовлен и готов к заливке."),
        TaskKind::BuildLadder => String::from("Набор качеств собран."),
        TaskKind::Deploy => String::from("Раздача развёрнута."),
        TaskKind::UpgradeServer => String::from("Серверная часть обновлена."),
        _ => String::from("Подробности — в разделе «Задачи»."),
    }
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
