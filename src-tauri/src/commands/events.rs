//! T013 — the stream of events from the core to the interface.
//!
//! The interface listens rather than polling. Otherwise the progress of a task that
//! runs for hours could only be shown by frequent polling, and that polling would
//! itself cause the stuttering we avoid (FR-080, SC-009).
//!
//! The event names are fixed by the contract `contracts/ipc-commands.md`.

use crate::commands::{AppEvent, AppState};
use crate::tasks::engine::{TaskEngine, TaskEvent};
use crate::tasks::state::{TaskKind, TaskState};
use tauri::{AppHandle, Emitter, Manager};

/// How long a task has to run to count as long (FR-084).
///
/// Notifying about every trifle teaches people to dismiss notifications unread, and
/// then what matters goes past with the rest. Half a minute is roughly the line beyond
/// which a person has moved on to something else and forgotten the task.
const LONG_TASK: std::time::Duration = std::time::Duration::from_secs(30);

/// Event names. The strings are fixed by the contract — changing one means changing
/// the contract.
pub mod names {
    pub const TASK_PROGRESS: &str = "task:progress";
    pub const TASK_DONE: &str = "task:done";
    /// The core has decided a system notification is warranted; the interface
    /// composes the text and shows it.
    pub const TASK_NOTIFY: &str = "task:notify";
    pub const LIBRARY_CHANGED: &str = "library:changed";
    pub const SERVER_STATE: &str = "server:state";
    pub const VIEWERS_UPDATE: &str = "viewers:update";
    /// A deployment moved on a step (FR-123).
    pub const DEPLOY_PROGRESS: &str = "deploy:progress";
    /// "Exit" was chosen in the tray menu while something was running (T400, FR-086).
    ///
    /// A question, not an announcement: the core has decided that leaving costs something,
    /// and the interface names what, task by task, and asks. Nothing exits until it is
    /// answered — see `commands::api::app_exit`.
    pub const APP_QUIT: &str = "app:quit-requested";
}

/// Start forwarding task events to the interface.
///
/// The core knows nothing of the shell: it broadcasts events into its own channel, and
/// this bridge carries them outwards. That is what keeps the core testable without a
/// window.
pub fn bridge_task_events(app: AppHandle, engine: &TaskEngine) {
    let mut rx = engine.subscribe();
    // The runtime comes from the shell rather than being our own. Calling tokio::spawn
    // directly here crashes the application at start-up: the shell is prepared outside
    // the runtime, and there is no reactor yet (caught by a run on 2026-08-25 — the
    // build and the tests showed nothing).
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let name = match &event {
                        TaskEvent::Progress { .. } => names::TASK_PROGRESS,
                        TaskEvent::Done { .. } => names::TASK_DONE,
                    };
                    if let Err(e) = app.emit(name, &event) {
                        tracing::debug!(error = %e, "event not delivered to the interface");
                    }
                    if let TaskEvent::Done {
                        id, state, error, ..
                    } = &event
                    {
                        notify_if_long(&app, id, *state, error.as_ref());
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    // The interface cannot keep up with the stream. Losing progress
                    // events does no harm — the ones that matter (completion, error)
                    // arrive next and bring the display into line.
                    tracing::debug!(
                        skipped,
                        "the interface fell behind, some events were dropped"
                    );
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
        tracing::debug!(error = %e, "the request for a notification was not delivered");
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

/// Whether the task ran longer than the threshold.
///
/// The time comes from the record: no separate accounting is needed, and `created_at`
/// and `updated_at` are written anyway. A time that will not parse counts as short —
/// notifying once too often is worse than staying quiet, because surplus notifications
/// teach people not to read them at all.
pub fn long_enough(created_at: &str, updated_at: &str) -> bool {
    let (Ok(from), Ok(to)) = (
        crate::store::db::parse_rfc3339(created_at),
        crate::store::db::parse_rfc3339(updated_at),
    ) else {
        return false;
    };
    to.saturating_sub(from) >= LONG_TASK.as_secs()
}

/// Start forwarding the core's other events to the interface.
pub fn bridge_app_events(app: AppHandle, state: &AppState) {
    let mut rx = state.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let name = match &event {
                        AppEvent::LibraryChanged { .. } => names::LIBRARY_CHANGED,
                        AppEvent::ViewersUpdate(_) => names::VIEWERS_UPDATE,
                        AppEvent::DeployProgress { .. } => names::DEPLOY_PROGRESS,
                    };
                    if let Err(e) = app.emit(name, &event) {
                        tracing::debug!(error = %e, "event not delivered to the interface");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::debug!(
                        skipped,
                        "the interface fell behind, some events were dropped"
                    );
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}
