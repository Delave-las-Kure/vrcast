//! T016, T019 — очередь задач, полосы по ресурсам, отмена и приостановка.
//!
//! Устройство подчинено трём требованиям, и каждое влияет на форму:
//!
//! - **Интерфейс остаётся отзывчивым** (FR-080, SC-009): вся работа идёт в исполнителе,
//!   а наружу уходит поток событий, а не запросы состояния.
//! - **Задачи переживают перезапуск** (FR-081): значимые сдвиги пишутся в базу, а
//!   застигнутые в работе при следующем старте становятся приостановленными, но никогда
//!   завершёнными (конституция, принцип III).
//! - **Отмена не считается выполненной, пока живо дерево процессов** (принцип III):
//!   состояние записывается только после того, как процессов не осталось.

use super::progress::ProgressThrottle;
use super::state::{Lane, LaneLimits, TaskKind, TaskState};
use super::store::{self, TaskRecord};
use crate::store::db::{now_rfc3339, Db, DbError};
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, Notify};
use tokio_util::sync::CancellationToken;

#[derive(Debug, thiserror::Error)]
pub enum TaskError {
    #[error("задача {0} не найдена")]
    NotFound(String),

    #[error("переход {from} → {to} для задачи {id} недопустим")]
    BadTransition {
        id: String,
        from: &'static str,
        to: &'static str,
    },

    #[error("задачу этого вида нельзя приостановить")]
    NotPausable,

    #[error("задача отменена")]
    Cancelled,

    #[error("{0}")]
    Failed(String),

    #[error(transparent)]
    Db(#[from] DbError),
}

pub type Result<T> = std::result::Result<T, TaskError>;

/// Событие о задаче, уходящее в интерфейс.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum TaskEvent {
    Progress {
        id: String,
        state: TaskState,
        progress: f64,
        stage: Option<String>,
        speed_bps: Option<i64>,
        eta_s: Option<i64>,
    },
    Done {
        id: String,
        state: TaskState,
        error: Option<String>,
    },
}

/// Ручки управления живой задачей.
struct LiveTask {
    kind: TaskKind,
    state: TaskState,
    cancel: CancellationToken,
    /// Взведён, пока задача приостановлена.
    paused: Arc<Mutex<bool>>,
    resume: Arc<Notify>,
    throttle: Arc<ProgressThrottle>,
}

/// То, что видит выполняющаяся задача.
///
/// Через него она сообщает о продвижении и узнаёт, не пора ли остановиться. Ничего
/// другого задаче знать не нужно — ни про базу, ни про очередь.
#[derive(Clone)]
pub struct TaskContext {
    pub id: String,
    cancel: CancellationToken,
    paused: Arc<Mutex<bool>>,
    resume: Arc<Notify>,
    throttle: Arc<ProgressThrottle>,
    events: broadcast::Sender<TaskEvent>,
    db: Arc<Db>,
}

impl TaskContext {
    /// Отменена ли задача. Проверять в местах, где можно остановиться без вреда.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Токен отмены — чтобы передать его в ожидание ввода-вывода.
    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    /// Прерваться, если задачу отменили.
    pub fn bail_if_cancelled(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(TaskError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Дождаться продолжения, если задача приостановлена.
    ///
    /// Вызывать между единицами работы: между кусками передачи, между шагами
    /// развёртывания. Приостановка вступит в силу на ближайшей такой точке.
    pub async fn wait_while_paused(&self) {
        loop {
            let paused = *self.paused.lock().unwrap_or_else(|e| e.into_inner());
            if !paused {
                return;
            }
            self.resume.notified().await;
        }
    }

    /// Сообщить о продвижении. Частота ограничена (T020).
    pub fn report(&self, progress: f64, stage: impl Into<String>) {
        self.report_full(progress, Some(stage.into()), None, None, false);
    }

    /// Сообщить о продвижении с показателями передачи.
    pub fn report_transfer(&self, progress: f64, speed_bps: i64, eta_s: i64) {
        self.report_full(progress, None, Some(speed_bps), Some(eta_s), false);
    }

    /// Сообщение, которое обязано пройти независимо от частоты: смена этапа, конец работы.
    pub fn report_important(&self, progress: f64, stage: impl Into<String>) {
        self.report_full(progress, Some(stage.into()), None, None, true);
    }

    fn report_full(
        &self,
        progress: f64,
        stage: Option<String>,
        speed_bps: Option<i64>,
        eta_s: Option<i64>,
        important: bool,
    ) {
        if !self.throttle.allow(important) {
            return;
        }
        let _ = self.events.send(TaskEvent::Progress {
            id: self.id.clone(),
            state: TaskState::Running,
            progress: progress.clamp(0.0, 1.0),
            stage,
            speed_bps,
            eta_s,
        });
    }

    /// Записать позицию возобновления.
    ///
    /// Это единственное, что имеет смысл писать в базу часто: без него прерванная
    /// передача начнётся заново.
    pub fn save_resume_token(&self, token: &str) -> Result<()> {
        if let Some(mut rec) = store::get(&self.db, &self.id)? {
            rec.resume_token = Some(token.to_owned());
            rec.updated_at = now_rfc3339();
            store::upsert(&self.db, &rec)?;
        }
        Ok(())
    }

    /// Прочитать позицию возобновления, оставленную прошлым запуском.
    pub fn resume_token(&self) -> Result<Option<String>> {
        Ok(store::get(&self.db, &self.id)?.and_then(|r| r.resume_token))
    }
}

/// Механизм задач.
#[derive(Clone)]
pub struct TaskEngine {
    db: Arc<Db>,
    live: Arc<Mutex<HashMap<String, LiveTask>>>,
    limits: LaneLimits,
    events: broadcast::Sender<TaskEvent>,
}

impl TaskEngine {
    pub fn new(db: Arc<Db>) -> Self {
        let (events, _) = broadcast::channel(256);
        Self {
            db,
            live: Arc::new(Mutex::new(HashMap::new())),
            limits: LaneLimits::default(),
            events,
        }
    }

    pub fn with_limits(mut self, limits: LaneLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Подписаться на события задач.
    pub fn subscribe(&self) -> broadcast::Receiver<TaskEvent> {
        self.events.subscribe()
    }

    /// Разобрать состояние после запуска приложения (T017).
    pub fn recover_after_start(&self) -> Result<store::RecoveryReport> {
        Ok(store::recover_after_start(&self.db)?)
    }

    /// Сколько задач сейчас занимает полосу.
    pub fn running_in_lane(&self, lane: Lane) -> usize {
        let live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        live.values()
            .filter(|t| t.state.occupies_lane() && t.kind.lane() == lane)
            .count()
    }

    /// Есть ли место в полосе для задачи этого вида.
    pub fn has_room_for(&self, kind: TaskKind) -> bool {
        let lane = kind.lane();
        self.running_in_lane(lane) < self.limits.for_lane(lane)
    }

    /// Поставить задачу и запустить её, когда освободится место в полосе.
    ///
    /// Возвращает идентификатор сразу: команда не блокируется на длительной работе
    /// (FR-080, договор слоя команд).
    pub async fn submit<F, Fut>(
        &self,
        kind: TaskKind,
        server_id: Option<String>,
        work: F,
    ) -> Result<String>
    where
        F: FnOnce(TaskContext) -> Fut + Send + 'static,
        Fut: Future<Output = std::result::Result<(), String>> + Send + 'static,
    {
        let id = uuid::Uuid::new_v4().to_string();
        let record = TaskRecord::new(id.clone(), kind, server_id);
        store::upsert(&self.db, &record)?;

        let cancel = CancellationToken::new();
        let paused = Arc::new(Mutex::new(false));
        let resume = Arc::new(Notify::new());
        let throttle = Arc::new(ProgressThrottle::default());

        {
            let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
            live.insert(
                id.clone(),
                LiveTask {
                    kind,
                    state: TaskState::Queued,
                    cancel: cancel.clone(),
                    paused: paused.clone(),
                    resume: resume.clone(),
                    throttle: throttle.clone(),
                },
            );
        }

        let ctx = TaskContext {
            id: id.clone(),
            cancel: cancel.clone(),
            paused,
            resume,
            throttle,
            events: self.events.clone(),
            db: self.db.clone(),
        };

        let engine = self.clone();
        let task_id = id.clone();
        tokio::spawn(async move {
            // Ждём места в полосе. Отмена работает и здесь: стоящую в очереди задачу
            // можно снять, не дожидаясь её запуска.
            loop {
                if cancel.is_cancelled() {
                    engine.finish(&task_id, TaskState::Cancelled, None);
                    return;
                }
                if engine.has_room_for(kind) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }

            if !engine.set_state(&task_id, TaskState::Running) {
                return;
            }

            let outcome = work(ctx).await;

            // Отмена важнее исхода работы: задача, снятая пользователем, не «упала».
            if cancel.is_cancelled() {
                engine.finish(&task_id, TaskState::Cancelled, None);
                return;
            }

            match outcome {
                Ok(()) => engine.finish(&task_id, TaskState::Completed, None),
                Err(e) => engine.finish(&task_id, TaskState::Failed, Some(e)),
            }
        });

        Ok(id)
    }

    /// Отменить задачу.
    ///
    /// Токен взводится сразу, но состояние записывается только когда работа
    /// действительно прекратилась — включая дерево процессов (принцип III).
    pub fn cancel(&self, id: &str) -> Result<()> {
        let token = {
            let live = self.live.lock().unwrap_or_else(|e| e.into_inner());
            let t = live
                .get(id)
                .ok_or_else(|| TaskError::NotFound(id.to_owned()))?;
            // Приостановленная задача не проснётся сама — будим, чтобы она увидела отмену.
            t.resume.notify_waiters();
            t.cancel.clone()
        };
        token.cancel();
        Ok(())
    }

    /// Приостановить задачу.
    pub fn pause(&self, id: &str) -> Result<()> {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        let t = live
            .get_mut(id)
            .ok_or_else(|| TaskError::NotFound(id.to_owned()))?;

        if t.kind.pause_kind() == super::state::PauseKind::NotPausable {
            return Err(TaskError::NotPausable);
        }
        if !t.state.can_transition_to(TaskState::Paused) {
            return Err(TaskError::BadTransition {
                id: id.to_owned(),
                from: t.state.as_str(),
                to: TaskState::Paused.as_str(),
            });
        }

        *t.paused.lock().unwrap_or_else(|e| e.into_inner()) = true;
        t.state = TaskState::Paused;
        drop(live);
        self.persist_state(id, TaskState::Paused, None);
        Ok(())
    }

    /// Продолжить приостановленную задачу.
    pub fn resume(&self, id: &str) -> Result<()> {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        let t = live
            .get_mut(id)
            .ok_or_else(|| TaskError::NotFound(id.to_owned()))?;

        if !t.state.can_transition_to(TaskState::Running) {
            return Err(TaskError::BadTransition {
                id: id.to_owned(),
                from: t.state.as_str(),
                to: TaskState::Running.as_str(),
            });
        }

        *t.paused.lock().unwrap_or_else(|e| e.into_inner()) = false;
        t.state = TaskState::Running;
        t.throttle.reset();
        t.resume.notify_waiters();
        drop(live);
        self.persist_state(id, TaskState::Running, None);
        Ok(())
    }

    /// Список задач из базы — включая завершённые и оставшиеся от прошлых запусков.
    pub fn list(&self) -> Result<Vec<TaskRecord>> {
        Ok(store::list(&self.db)?)
    }

    pub fn get(&self, id: &str) -> Result<Option<TaskRecord>> {
        Ok(store::get(&self.db, id)?)
    }

    fn set_state(&self, id: &str, next: TaskState) -> bool {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        let Some(t) = live.get_mut(id) else {
            return false;
        };
        if !t.state.can_transition_to(next) {
            tracing::warn!(
                id,
                from = t.state.as_str(),
                to = next.as_str(),
                "недопустимый переход"
            );
            return false;
        }
        t.state = next;
        drop(live);
        self.persist_state(id, next, None);
        true
    }

    fn finish(&self, id: &str, state: TaskState, error: Option<String>) {
        {
            let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(t) = live.get_mut(id) {
                t.state = state;
            }
            live.remove(id);
        }
        self.persist_state(id, state, error.clone());
        let _ = self.events.send(TaskEvent::Done {
            id: id.to_owned(),
            state,
            error,
        });
    }

    fn persist_state(&self, id: &str, state: TaskState, error: Option<String>) {
        match store::get(&self.db, id) {
            Ok(Some(mut rec)) => {
                rec.state = state;
                if state == TaskState::Completed {
                    rec.progress = 1.0;
                }
                if let Some(e) = error {
                    // Ошибка проходит вырезание секретов: она может прийти от чужой
                    // библиотеки, которая о наших правилах не знает (принцип IV).
                    rec.error = Some(crate::store::redact::redact(&e).into_owned());
                }
                rec.updated_at = now_rfc3339();
                if let Err(e) = store::upsert(&self.db, &rec) {
                    tracing::error!(id, error = %e, "не удалось сохранить состояние задачи");
                }
            }
            Ok(None) => tracing::warn!(id, "состояние сохранять некуда: записи о задаче нет"),
            Err(e) => tracing::error!(id, error = %e, "не удалось прочитать задачу"),
        }
    }
}
