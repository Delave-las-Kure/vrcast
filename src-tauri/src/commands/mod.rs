//! T013 — the command layer: the one boundary between the interface and the core.
//!
//! The interface knows nothing of SSH, of FFmpeg, or of how the server is arranged. It
//! calls commands and listens to a stream of events — nothing more.
//!
//! **Commands come in two layers**, and that is not a needless one. Inside are ordinary
//! functions (`api`) that know nothing of the application shell; outside are thin wrappers
//! the shell exposes. That way the contract is checked by tests directly, without opening a
//! window: otherwise contract tests would need a live application with graphics, and
//! continuous integration has none.

pub mod convert;
pub mod deploy;
pub mod diag;
pub mod error;
pub mod events;
pub mod forget;
pub mod geo;
pub mod ladder;
pub mod library;
pub mod limits;
pub mod quality;
pub mod servers;
pub mod settings;
pub mod update;
pub mod upload;
pub mod viewers;

use crate::domain::wording::Detail;
use crate::store::db::Db;
use crate::store::secrets::{OsSecretStore, SecretStore};
use crate::tasks::engine::TaskEngine;
use crate::tasks::state::PauseKind;
use crate::tasks::store::TaskRecord;
use error::{AppError, DetailCode, ErrorCode, Result};
use serde::Serialize;
use std::sync::Arc;

/// An event about the application's state that has nothing to do with tasks.
///
/// The core sends them into its own channel and the shell forwards them outside (see
/// `events`). That way the core knows nothing of the window and stays checkable without
/// starting up any graphics.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum AppEvent {
    /// The server's library changed: read it again.
    LibraryChanged { server_id: String },
    /// A deployment has got another step done (FR-123).
    ///
    /// The whole list goes out each time rather than the one step that moved: a screen that
    /// pieced the list together from a stream of single steps would show something different
    /// if it missed one, and it will miss one — a person opens the screen part-way through.
    DeployProgress {
        server_id: String,
        steps: Vec<crate::domain::deploy_steps::PlannedStep>,
    },
    /// Who is watching has changed (FR-054).
    ///
    /// Sent rather than waited to be asked for: the list changes every few seconds, and
    /// asking for it that often is what SC-009 exists to prevent.
    ViewersUpdate(crate::server::viewers::ViewersUpdate),
}

/// The application's shared state. Everything the commands need lives here.
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Db>,
    /// Where this run keeps everything, when it keeps it anywhere.
    ///
    /// **`None` in tests, and that is a safety measure rather than a shortcut.** Removal
    /// (FR-114) deletes this directory, and the first version of it worked the path out from
    /// the environment instead — so a contract test, running with an in-memory database,
    /// deleted the developer's own profiles and both place tables. Caught 2026-08-27, restored
    /// from a copy made beforehand.
    ///
    /// Carried on the state, a test cannot reach a real directory: it has not been given one.
    pub data_dir: Option<std::path::PathBuf>,
    pub tasks: TaskEngine,
    pub secrets: Arc<dyn SecretStore>,
    /// The channel for events unrelated to tasks.
    pub(crate) events: tokio::sync::broadcast::Sender<AppEvent>,
    /// The watching of viewers — at most one server at a time (T171).
    pub viewers: Arc<viewers::ViewersWatch>,
    /// Where addresses are, looked up on this machine and nowhere else (FR-057).
    ///
    /// Behind a lock because the tables arrive **after** the application has started: they
    /// are fetched in the background, and a watch that began before they landed must start
    /// answering once they have, rather than staying blind until the next restart.
    ///
    /// Empty is a working state, and the one the application ships in: every viewer is then
    /// "not determined", which is the truth.
    pub places: Arc<std::sync::RwLock<crate::store::geo::Places>>,
}

impl AppState {
    /// Build the state with the real stores and sort out what the previous run left.
    pub fn bootstrap() -> Result<Self> {
        let path = Db::default_path()?;
        let db = Arc::new(Db::open(path.clone())?);
        let mut state = Self::with_db(db, Arc::new(OsSecretStore::new()))?;
        // The one place a real directory is handed over. Everything else — tests included —
        // gets `None` and can therefore delete nothing.
        state.data_dir = path.parent().map(|p| p.to_path_buf());
        Ok(state)
    }

    /// The same, but with the stores given — for tests.
    pub fn with_db(db: Arc<Db>, secrets: Arc<dyn SecretStore>) -> Result<Self> {
        let tasks = TaskEngine::new(db.clone());

        // The order matters. First the programs that survived the previous run are
        // finished off, and only then are the tasks sorted out: otherwise a task would be
        // declared paused while its process is still alive and still writing into the
        // result file.
        match crate::tasks::registry::sweep_on_startup(&db) {
            Ok(report) if !report.is_clean() => {
                tracing::warn!(
                    killed = report.killed.len(),
                    skipped = report.reused.len(),
                    "swept up the programs left over from the previous run"
                );
            }
            Ok(_) => {}
            Err(e) => tracing::error!(error = %e, "the start-up sweep failed"),
        }

        if let Ok(report) = tasks.recover_after_start() {
            if !report.interrupted.is_empty() {
                tracing::warn!(
                    count = report.interrupted.len(),
                    "tasks from the previous run are paused and waiting to be carried on"
                );
            }
        }

        let (events, _) = tokio::sync::broadcast::channel(64);
        Ok(Self {
            db,
            data_dir: None,
            tasks,
            secrets,
            events,
            viewers: Arc::new(viewers::ViewersWatch::default()),
            places: Arc::new(std::sync::RwLock::new(
                crate::store::geo::dir()
                    .map(|d| crate::store::geo::Places::open(&d))
                    .unwrap_or_default(),
            )),
        })
    }

    /// Subscribe to the application's events.
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<AppEvent> {
        self.events.subscribe()
    }

    /// Say that the server's library changed.
    ///
    /// Having no listeners is normal rather than an error: the events go nowhere while the
    /// interface is not open, and failing over that would be absurd.
    pub fn notify_library_changed(&self, server_id: &str) {
        let _ = self.events.send(AppEvent::LibraryChanged {
            server_id: server_id.to_owned(),
        });
    }
}

/// The versions of the application and of the server side (FR-128).
#[derive(Debug, Clone, Serialize)]
pub struct Versions {
    pub app: String,
    /// The server-side version of the active server. Arrives in Phase 7.
    pub server: Option<u32>,
    /// The local store's version — needed when sorting out trouble.
    pub schema: u32,
}

/// What becomes of a task if the application is closed (FR-086).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct TaskOnClose {
    pub id: String,
    pub kind: String,
    pub progress: f64,
    /// `resumes` — carries on from where it got to; `restarts` — begins again.
    pub outcome: &'static str,
    /// What exactly will happen to this one. A general "tasks are running, close
    /// anyway?" is not enough: it gives nothing to decide on.
    pub explanation: Detail,
}

/// The ordinary functions — what actually runs.
pub mod api {
    use super::*;

    /// What versions are in play (FR-128, T290).
    ///
    /// `server_id` is optional. The About screen asks about none; a server's card asks
    /// about that server, and then the version of the server side goes in beside the
    /// application's — side by side because apart they say nothing. "Server side 1"
    /// means nothing until you know what this application deploys.
    ///
    /// A server that will not answer leaves `server` empty rather than failing the
    /// whole call: a version panel that could not be shown because a machine was
    /// asleep would be the application refusing to say what **it** is.
    pub async fn app_versions(state: &AppState, server_id: Option<&str>) -> Result<Versions> {
        let server = match server_id {
            None => None,
            Some(id) => deploy::api::server_detect(state, id)
                .await
                .ok()
                .and_then(|s| s.server_version),
        };
        Ok(Versions {
            app: env!("CARGO_PKG_VERSION").to_owned(),
            server,
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

    /// Reorder the waiting tasks in the queue (FR-083).
    ///
    /// `ordered` holds the task identifiers in the wanted order. Running ones are left
    /// alone: breaking off a transfer already under way for the sake of a reordering would
    /// throw away work already done. It returns how many tasks were moved — the list on a
    /// person's screen always lags a little, and some of them may already have started.
    pub fn tasks_reorder(state: &AppState, ordered: &[String]) -> Result<usize> {
        Ok(state.tasks.reorder_queue(ordered)?)
    }

    /// The waiting tasks' identifiers, in the order they will be taken up.
    pub fn tasks_queue_order(state: &AppState) -> Result<Vec<String>> {
        Ok(state.tasks.queue_order())
    }

    /// What to tell a person when the application is closing (FR-086).
    ///
    /// The difference between the kinds of task is not cosmetic here: a transfer carries on
    /// from where it got to, while a paused preparation is held by a living process and will
    /// not survive the closing. A person must learn of that **before** it happens.
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
                    Detail::new(DetailCode::OnCloseResumesFrom).with("percent", percent),
                ),
                PauseKind::SuspendedProcess => (
                    "restarts",
                    Detail::new(DetailCode::OnCloseRestartsLosing).with("percent", percent),
                ),
                PauseKind::NotPausable => {
                    if t.state == TaskState::Queued {
                        ("resumes", Detail::new(DetailCode::OnCloseNotStartedYet))
                    } else {
                        ("restarts", Detail::new(DetailCode::OnCloseMustRunAgain))
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

    /// Check the FFmpeg bundled with the application (FR-112, T115).
    ///
    /// Called at start-up and before a preparation. The bundled file may fail to run: an
    /// antivirus cut it out, the installer unpacked only halfway, the file has no execute
    /// permission. Learning that at the start means telling a person what to fix; learning
    /// it halfway through a two-hour preparation means taking those two hours away.
    pub async fn ffmpeg_probe_self() -> Result<crate::media::ffmpeg::FfmpegInfo> {
        let info = crate::media::ffmpeg::probe_self().await.map_err(|e| {
            AppError::new(ErrorCode::FfmpegBroken)
                .detail(DetailCode::FfmpegSelfBroken)
                .with_cause(e.to_string())
        })?;

        // Without a software encoder, preparing is impossible on a machine without a
        // suitable graphics card — that is, for some people the application would quietly
        // turn out to be useless.
        if !info.has_x264 {
            return Err(AppError::new(ErrorCode::FfmpegBroken)
                .detail(DetailCode::FfmpegNoX264)
                .with_cause(info.version));
        }
        Ok(info)
    }

    /// Examine a source file: what is it we have been given (FR-020).
    ///
    /// A quick operation rather than a task: examining takes fractions of a second and a
    /// person is waiting for the answer right now. Making a queue entry for it would litter
    /// the task list with something that ends before it manages to appear there.
    pub async fn source_probe(path: &str) -> Result<crate::domain::source::SourceFile> {
        use crate::media::probe::ProbeError;

        crate::media::probe::probe(std::path::Path::new(path))
            .await
            .map_err(|e| match e {
                ProbeError::Ffmpeg(_) => AppError::new(ErrorCode::FfmpegBroken)
                    .detail(DetailCode::FfmpegSelfBroken)
                    .with_cause(e.to_string()),
                ProbeError::NoVideo => AppError::new(ErrorCode::InvalidInput)
                    .detail(DetailCode::ProbeNoVideo)
                    .with_cause(path),
                ProbeError::Unreadable(_) => AppError::new(ErrorCode::InvalidInput)
                    .detail(DetailCode::ProbeUnreadable)
                    // The parser's own complaint is kept as it stands: "moov atom not
                    // found" means nothing to most people, but it can be searched for,
                    // and "bad file" cannot.
                    .with_cause(e.to_string()),
            })
    }

    /// Learn a server's fingerprint without presenting it anything (FR-092).
    pub async fn server_probe_fingerprint(host: &str, port: u16) -> Result<String> {
        let addr = crate::ssh::ServerAddress::new(host, port);
        Ok(crate::ssh::fingerprint::probe(&addr).await?)
    }
}

/// The thin wrappers the shell exposes to the interface.
///
/// There must be no logic here — only converting the state and calling `api`.
pub mod ipc {
    use super::*;
    use tauri::State;

    #[tauri::command]
    pub async fn app_versions(
        state: State<'_, AppState>,
        server_id: Option<String>,
    ) -> Result<Versions> {
        api::app_versions(&state, server_id.as_deref()).await
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
