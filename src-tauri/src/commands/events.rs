//! T013 — поток событий из ядра в интерфейс.
//!
//! Интерфейс не опрашивает состояние, а слушает. Иначе показать продвижение
//! многочасовой задачи можно было бы только частым опросом, и он же стал бы причиной
//! подтормаживания, которого мы избегаем (FR-080, SC-009).
//!
//! Имена событий закреплены договором `contracts/ipc-commands.md`.

use crate::tasks::engine::{TaskEngine, TaskEvent};
use tauri::{AppHandle, Emitter};

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
    tokio::spawn(async move {
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
