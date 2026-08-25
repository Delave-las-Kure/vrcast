//! T016, T019 — the task queue, the lanes by resource, cancelling and pausing.
//!
//! The design answers three requirements, and each one shapes it:
//!
//! - **The interface stays responsive** (FR-080, SC-009): all the work runs in the task
//!   runner, and what goes outside is a stream of events rather than state queries.
//! - **Tasks survive a restart** (FR-081): the moves that matter are written to the
//!   database, and those caught running become paused at the next start, never completed
//!   (constitution, principle III).
//! - **A cancellation does not count as done while the process tree is alive**
//!   (principle III): the state is written only once no processes are left.

use super::progress::ProgressThrottle;
use super::state::{Lane, LaneLimits, TaskKind, TaskState};
use super::store::{self, TaskRecord};
use crate::domain::wording::DetailCode;
use crate::error::AppError;
use crate::store::db::{Db, DbError};
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, Notify};
use tokio_util::sync::CancellationToken;

#[derive(Debug, thiserror::Error)]
pub enum TaskError {
    #[error("task {0} not found")]
    NotFound(String),

    #[error("transition {from} -> {to} is not allowed for task {id}")]
    BadTransition {
        id: String,
        from: &'static str,
        to: &'static str,
    },

    #[error("a task of this kind cannot be paused")]
    NotPausable,

    #[error("task cancelled")]
    Cancelled,

    #[error("{0}")]
    Failed(String),

    #[error(transparent)]
    Db(#[from] DbError),
}

pub type Result<T> = std::result::Result<T, TaskError>;

/// How often progress reaches the database.
///
/// Three seconds is about how much work one is willing to lose after a restart, not about
/// how smooth the display looks: the smoothness comes from the stream of events.
const PROGRESS_PERSIST_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3);

/// An event about a task, on its way to the interface.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum TaskEvent {
    Progress {
        id: String,
        state: TaskState,
        progress: f64,
        stage: Option<DetailCode>,
        speed_bps: Option<i64>,
        eta_s: Option<i64>,
    },
    Done {
        id: String,
        state: TaskState,
        error: Option<AppError>,
    },
}

/// The controls of a live task.
struct LiveTask {
    kind: TaskKind,
    state: TaskState,
    cancel: CancellationToken,
    /// Raised while the task is paused.
    paused: Arc<Mutex<bool>>,
    resume: Arc<Notify>,
    throttle: Arc<ProgressThrottle>,
    /// The place in the queue: lower runs sooner (FR-083).
    position: i64,
}

/// What a running task sees.
///
/// Through it the task reports its progress and learns whether it is time to stop. Nothing
/// else is any of the task's business — not the database, not the queue.
#[derive(Clone)]
pub struct TaskContext {
    pub id: String,
    cancel: CancellationToken,
    paused: Arc<Mutex<bool>>,
    resume: Arc<Notify>,
    throttle: Arc<ProgressThrottle>,
    /// A separate valve — for writing progress to the database.
    ///
    /// Not the same one the events use: the events go into memory four times a second,
    /// and writing to disk that often for the sake of a number serves nobody.
    persist_throttle: Arc<ProgressThrottle>,
    events: broadcast::Sender<TaskEvent>,
    db: Arc<Db>,
}

impl TaskContext {
    /// Whether the task was cancelled. Check it where stopping does no harm.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// The cancellation token — to hand into a wait on input-output.
    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    /// Whether the task is paused — **without waiting** for it to carry on.
    ///
    /// It differs from `wait_while_paused` in not blocking. Needed where the work cannot
    /// simply stand and wait: encoding is done by somebody else's program, and that has to
    /// be frozen rather than abandoned halfway. Without this a "pause" would free a place
    /// in the lane while stopping nothing (debt T067, FR-083a).
    pub fn is_paused(&self) -> bool {
        *self.paused.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Break off if the task was cancelled.
    pub fn bail_if_cancelled(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(TaskError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Wait until the task carries on, if it is paused.
    ///
    /// Call it between units of work: between pieces of a transfer, between steps of a
    /// setup. A pause takes effect at the nearest such point. A cancellation wakes it too:
    /// after it returns, the caller must check `is_cancelled`.
    pub async fn wait_while_paused(&self) {
        loop {
            // The subscription to "carry on" is taken out BEFORE the flag is read:
            // notify_waiters keeps no permit for those not yet subscribed, and otherwise a
            // "carry on" pressed in the gap between reading the flag and falling asleep
            // would be lost forever.
            let resumed = self.resume.notified();

            if self.is_cancelled() || !*self.paused.lock().unwrap_or_else(|e| e.into_inner()) {
                return;
            }

            // Waiting for "carry on" alone will not do: a cancellation does not clear the
            // pause flag, and a task woken by one would fall asleep again at once.
            tokio::select! {
                _ = resumed => {}
                _ = self.cancel.cancelled() => return,
            }
        }
    }

    /// Report progress. The rate is capped (T020).
    pub fn report(&self, progress: f64, stage: DetailCode) {
        self.report_full(progress, Some(stage), None, None, false);
    }

    /// Report progress along with the transfer's figures.
    pub fn report_transfer(&self, progress: f64, speed_bps: i64, eta_s: i64) {
        self.report_full(progress, None, Some(speed_bps), Some(eta_s), false);
    }

    /// A message that must get through regardless of the rate cap: a change of stage, the
    /// end of the work.
    pub fn report_important(&self, progress: f64, stage: DetailCode) {
        self.report_full(progress, Some(stage), None, None, true);
    }

    fn report_full(
        &self,
        progress: f64,
        stage: Option<DetailCode>,
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

    /// Remember the progress so that it survives the application closing.
    ///
    /// As events, progress spreads through the interface four times a second, but it lives
    /// only in memory. It reaches the database far less often — once every few seconds:
    /// accuracy to the second serves nobody here, and there is no point keeping the disk
    /// busy for it. The task itself decides when to call: the task layer does not know what
    /// counts as progress.
    ///
    /// A failed write is swallowed deliberately: a number is not the work, and a transfer
    /// running for hours must not be brought down by one.
    pub fn save_progress(&self, progress: f64) {
        if !self.persist_throttle.allow(false) {
            return;
        }
        if let Err(e) = store::save_progress(&self.db, &self.id, progress) {
            tracing::debug!(id = %self.id, error = %e, "the progress was not written");
        }
    }

    /// Write the resume position.
    ///
    /// This is the one thing worth writing to the database often: without it an interrupted
    /// transfer starts over. The write is pointed — see `store::save_resume_token`.
    pub fn save_resume_token(&self, token: &str) -> Result<()> {
        store::save_resume_token(&self.db, &self.id, token)?;
        Ok(())
    }

    /// Read the resume position left by the previous run.
    pub fn resume_token(&self) -> Result<Option<String>> {
        Ok(store::get(&self.db, &self.id)?.and_then(|r| r.resume_token))
    }
}

/// How an attempt to take a place in a lane ended.
enum ClaimOutcome {
    /// The place was taken; the task is now running.
    Started,
    /// The lane is full — wait and try again.
    Busy,
    /// The task is no longer among the living (cancelled or finished) — do not start it.
    Gone,
}

/// The task machinery.
#[derive(Clone)]
pub struct TaskEngine {
    db: Arc<Db>,
    live: Arc<Mutex<HashMap<String, LiveTask>>>,
    limits: LaneLimits,
    events: broadcast::Sender<TaskEvent>,
    /// The place the next task submitted will get.
    next_position: Arc<std::sync::atomic::AtomicI64>,
}

impl TaskEngine {
    pub fn new(db: Arc<Db>) -> Self {
        let (events, _) = broadcast::channel(256);
        // The count carries on from where the previous run left off: otherwise a new task
        // would get a place already taken by one sitting in the database, and would cut
        // into the middle of somebody else's queue.
        let next = store::max_queue_order(&db).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "could not read the queue order — starting from zero");
            0
        }) + 1;
        Self {
            db,
            live: Arc::new(Mutex::new(HashMap::new())),
            limits: LaneLimits::default(),
            events,
            next_position: Arc::new(std::sync::atomic::AtomicI64::new(next)),
        }
    }

    pub fn with_limits(mut self, limits: LaneLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Subscribe to task events.
    pub fn subscribe(&self) -> broadcast::Receiver<TaskEvent> {
        self.events.subscribe()
    }

    /// Sort out the state after the application starts (T017).
    pub fn recover_after_start(&self) -> Result<store::RecoveryReport> {
        Ok(store::recover_after_start(&self.db)?)
    }

    /// How many tasks take up the lane right now.
    pub fn running_in_lane(&self, lane: Lane) -> usize {
        let live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        live.values()
            .filter(|t| t.state.occupies_lane() && t.kind.lane() == lane)
            .count()
    }

    /// Whether the lane has room for a task of this kind.
    pub fn has_room_for(&self, kind: TaskKind) -> bool {
        let lane = kind.lane();
        self.running_in_lane(lane) < self.limits.for_lane(lane)
    }

    /// Submit a task and start it once there is room in its lane.
    ///
    /// It returns the identifier at once: a command does not block on long work (FR-080,
    /// the command layer's contract).
    pub async fn submit<F, Fut>(
        &self,
        kind: TaskKind,
        server_id: Option<String>,
        work: F,
    ) -> Result<String>
    where
        F: FnOnce(TaskContext) -> Fut + Send + 'static,
        Fut: Future<Output = std::result::Result<(), AppError>> + Send + 'static,
    {
        let id = uuid::Uuid::new_v4().to_string();
        let mut record = TaskRecord::new(id.clone(), kind, server_id);
        record.queue_order = self
            .next_position
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        store::upsert(&self.db, &record)?;
        self.start(
            id.clone(),
            kind,
            TaskState::Queued,
            record.queue_order,
            work,
        );
        Ok(id)
    }

    /// Bring a task from the previous run back to life without creating a new one.
    ///
    /// Needed for FR-031: an upload must carry on after the application is closed and
    /// started again. Without it the task shows in the list as paused, but there is nothing
    /// to carry it on with — the working part lives only in memory and dies along with the
    /// application. The identifier stays the same: it has the same resume position, the same
    /// record in the database, and the same place in a person's eyes.
    ///
    /// The task comes back **paused**: it waits until a person says "carry on". Resuming a
    /// transfer that runs for hours unbidden at start-up will not do — a person may have
    /// closed the application precisely to stop it.
    pub fn resubmit_paused<F, Fut>(&self, id: &str, work: F) -> Result<()>
    where
        F: FnOnce(TaskContext) -> Fut + Send + 'static,
        Fut: Future<Output = std::result::Result<(), AppError>> + Send + 'static,
    {
        let record = store::get(&self.db, id)?.ok_or_else(|| TaskError::NotFound(id.to_owned()))?;
        if record.state.is_final() {
            return Err(TaskError::BadTransition {
                id: id.to_owned(),
                from: record.state.as_str(),
                to: TaskState::Paused.as_str(),
            });
        }
        if self
            .live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(id)
        {
            // Already alive — raising it a second time will not do: that would give two
            // pieces of work under one identifier.
            return Ok(());
        }

        store::save_state(&self.db, id, TaskState::Paused, None)?;
        self.start(
            id.to_owned(),
            record.kind,
            TaskState::Paused,
            record.queue_order,
            work,
        );
        Ok(())
    }

    /// The part both ways of submitting share: create the live task and start its work.
    fn start<F, Fut>(&self, id: String, kind: TaskKind, initial: TaskState, position: i64, work: F)
    where
        F: FnOnce(TaskContext) -> Fut + Send + 'static,
        Fut: Future<Output = std::result::Result<(), AppError>> + Send + 'static,
    {
        let cancel = CancellationToken::new();
        let paused = Arc::new(Mutex::new(initial == TaskState::Paused));
        let resume = Arc::new(Notify::new());
        let throttle = Arc::new(ProgressThrottle::default());

        {
            let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
            live.insert(
                id.clone(),
                LiveTask {
                    kind,
                    state: initial,
                    cancel: cancel.clone(),
                    paused: paused.clone(),
                    resume: resume.clone(),
                    throttle: throttle.clone(),
                    position,
                },
            );
        }

        let ctx = TaskContext {
            id: id.clone(),
            cancel: cancel.clone(),
            paused,
            resume,
            throttle,
            persist_throttle: Arc::new(ProgressThrottle::new(PROGRESS_PERSIST_INTERVAL)),
            events: self.events.clone(),
            db: self.db.clone(),
        };

        let engine = self.clone();
        let task_id = id.clone();
        let paused_flag = ctx.paused.clone();
        let resume_signal = ctx.resume.clone();
        tokio::spawn(async move {
            // A task raised after a restart waits for a person and **takes up no lane**:
            // otherwise it would hold a place while doing nothing, and a second one like it
            // could not start. Carrying on a transfer that runs for hours unbidden at
            // start-up will not do either — the application may have been closed precisely
            // to stop it.
            loop {
                let resumed = resume_signal.notified();
                if cancel.is_cancelled() {
                    engine.finish(&task_id, TaskState::Cancelled, None);
                    return;
                }
                if !*paused_flag.lock().unwrap_or_else(|e| e.into_inner()) {
                    break;
                }
                tokio::select! {
                    _ = resumed => {}
                    _ = cancel.cancelled() => {
                        engine.finish(&task_id, TaskState::Cancelled, None);
                        return;
                    }
                }
            }

            // Waiting for room in the lane. Cancelling works here too: a task standing in
            // the queue can be dropped without waiting for it to start.
            loop {
                if cancel.is_cancelled() {
                    engine.finish(&task_id, TaskState::Cancelled, None);
                    return;
                }
                match engine.try_claim_lane(&task_id) {
                    ClaimOutcome::Started => break,
                    ClaimOutcome::Busy => {
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                    ClaimOutcome::Gone => return,
                }
            }

            let outcome = work(ctx).await;

            // A cancellation outweighs the work's outcome: a task a person dropped did
            // not "fail".
            if cancel.is_cancelled() {
                engine.finish(&task_id, TaskState::Cancelled, None);
                return;
            }

            match outcome {
                Ok(()) => engine.finish(&task_id, TaskState::Completed, None),
                Err(e) => engine.finish(&task_id, TaskState::Failed, Some(e)),
            }
        });
    }

    /// Reorder the tasks in the queue (FR-083).
    ///
    /// `ordered` holds the task identifiers in the wanted order, as a person sees them in
    /// the list. Only those **waiting their turn** are moved: a running task is left alone,
    /// because breaking it off for the sake of a reordering would throw away work already
    /// done. The places taken are redistributed among themselves, so tasks not in the list
    /// stay where they stood.
    ///
    /// Tasks that managed to start or finish between the list being shown and the button
    /// being pressed are skipped quietly: the list on a person's screen always lags a
    /// little, and refusing the whole reordering over one such task would punish them for
    /// somebody else's speed. It returns how many tasks were really moved.
    pub fn reorder_queue(&self, ordered: &[String]) -> Result<usize> {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());

        // Only those still waiting are taken, keeping the order from the request.
        let waiting: Vec<&String> = ordered
            .iter()
            .filter(|id| {
                live.get(id.as_str())
                    .is_some_and(|t| t.state == TaskState::Queued)
            })
            .collect();
        if waiting.len() < 2 {
            // There is nothing to reorder: one task or none.
            return Ok(0);
        }

        // The places they hold right now are the ones handed out in the new order. That way
        // tasks left out of the request do not move a single step.
        let mut places: Vec<i64> = waiting
            .iter()
            .filter_map(|id| live.get(id.as_str()).map(|t| t.position))
            .collect();
        places.sort_unstable();

        let mut to_write: Vec<(String, i64)> = Vec::with_capacity(waiting.len());
        for (id, place) in waiting.iter().zip(places.iter()) {
            to_write.push(((*id).clone(), *place));
        }
        for (id, place) in &to_write {
            if let Some(t) = live.get_mut(id.as_str()) {
                t.position = *place;
            }
        }
        drop(live);

        // And to the database, so the order survives a restart of the application.
        for (id, place) in &to_write {
            if let Err(e) = store::save_queue_order(&self.db, id, *place) {
                tracing::warn!(id, error = %e, "the queue order was not written");
            }
        }
        Ok(to_write.len())
    }

    /// The waiting tasks' identifiers, in the order they will be taken up.
    pub fn queue_order(&self) -> Vec<String> {
        let live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        let mut waiting: Vec<(&String, i64)> = live
            .iter()
            .filter(|(_, t)| t.state == TaskState::Queued)
            .map(|(id, t)| (id, t.position))
            .collect();
        waiting.sort_by_key(|(_, position)| *position);
        waiting.into_iter().map(|(id, _)| id.clone()).collect()
    }

    /// Cancel a task.
    ///
    /// The token is raised at once, but the state is written only when the work has really
    /// stopped — the process tree included (principle III).
    pub fn cancel(&self, id: &str) -> Result<()> {
        let token = {
            let live = self.live.lock().unwrap_or_else(|e| e.into_inner());
            match live.get(id) {
                Some(t) => {
                    // A paused task will not wake by itself — it is woken so that it sees
                    // the cancellation.
                    t.resume.notify_waiters();
                    Some(t.cancel.clone())
                }
                None => None,
            }
        };

        match token {
            Some(token) => {
                token.cancel();
                Ok(())
            }
            // The task is not among the living: it was left over from the previous run and
            // nobody raised it. There is nothing to stop, but the person's decision must be
            // written down — otherwise it stays in the list as paused forever, with nothing
            // to drop it with.
            None => {
                let record =
                    store::get(&self.db, id)?.ok_or_else(|| TaskError::NotFound(id.to_owned()))?;
                if record.state.is_final() {
                    // Repeating is safe (constitution, principle V).
                    return Ok(());
                }
                store::save_state(&self.db, id, TaskState::Cancelled, None)?;
                let _ = self.events.send(TaskEvent::Done {
                    id: id.to_owned(),
                    state: TaskState::Cancelled,
                    error: None,
                });
                Ok(())
            }
        }
    }

    /// Pause a task.
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

    /// Carry on a paused task.
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

    /// The list of tasks from the database — finished ones and leftovers from previous
    /// runs included.
    pub fn list(&self) -> Result<Vec<TaskRecord>> {
        Ok(store::list(&self.db)?)
    }

    pub fn get(&self, id: &str) -> Result<Option<TaskRecord>> {
        Ok(store::get(&self.db, id)?)
    }

    /// Take a place in the lane and become running — atomically, under one lock.
    ///
    /// Checking for room and changing the state are inseparable on purpose: two tasks that
    /// wake at the same moment would otherwise both see one free place — and both would
    /// start, two preparations in a lane meant for one.
    fn try_claim_lane(&self, id: &str) -> ClaimOutcome {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        let Some(t) = live.get(id) else {
            return ClaimOutcome::Gone;
        };
        let lane = t.kind.lane();
        // We do not count ourselves. A task raised after a restart and carried on by a
        // person already counts as running — and, counting itself, would never see a free
        // place in its own lane.
        let used = live
            .iter()
            .filter(|(other_id, x)| {
                other_id.as_str() != id && x.state.occupies_lane() && x.kind.lane() == lane
            })
            .count();
        if used >= self.limits.for_lane(lane) {
            return ClaimOutcome::Busy;
        }

        // The queue is honoured: the place goes to the task standing first in the lane.
        // Without this check the order would be "whoever grabbed the lock first", and
        // reordering (FR-083) would give nothing — there would be nothing to reorder.
        //
        // Only those standing in the queue are compared. A paused task is not waiting for
        // the lane, it is waiting for a person, and has no claim to hold the queue; one
        // carried on by a person goes at once — they have just said it is the one they want.
        if t.state == TaskState::Queued {
            let position = t.position;
            let someone_is_ahead = live.values().any(|x| {
                x.state == TaskState::Queued && x.kind.lane() == lane && x.position < position
            });
            if someone_is_ahead {
                return ClaimOutcome::Busy;
            }
        }

        let Some(t) = live.get_mut(id) else {
            return ClaimOutcome::Gone;
        };
        if !t.state.can_transition_to(TaskState::Running) {
            tracing::warn!(
                id,
                from = t.state.as_str(),
                "the task cannot become running"
            );
            return ClaimOutcome::Gone;
        }
        t.state = TaskState::Running;
        drop(live);
        self.persist_state(id, TaskState::Running, None);
        ClaimOutcome::Started
    }

    fn finish(&self, id: &str, state: TaskState, error: Option<AppError>) {
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

    fn persist_state(&self, id: &str, state: TaskState, error: Option<AppError>) {
        match store::save_state(&self.db, id, state, error.as_ref()) {
            Ok(true) => {}
            Ok(false) => tracing::warn!(
                id,
                "nowhere to save the state: there is no record of the task"
            ),
            Err(e) => tracing::error!(id, error = %e, "could not save the task's state"),
        }
    }
}

impl From<TaskError> for crate::error::AppError {
    fn from(e: TaskError) -> Self {
        use crate::error::{AppError, ErrorCode};
        use TaskError as T;
        let code = match &e {
            T::NotFound(_) => ErrorCode::TaskNotFound,
            T::BadTransition { .. } => ErrorCode::TaskBadTransition,
            T::NotPausable => ErrorCode::TaskNotPausable,
            T::Cancelled => ErrorCode::TaskCancelled,
            T::Db(_) => ErrorCode::StorageFailed,
            T::Failed(_) => ErrorCode::Internal,
        };
        AppError::new(code).with_cause(e)
    }
}
