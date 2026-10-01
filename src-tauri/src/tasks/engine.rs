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
use crate::domain::wording::{Detail, DetailCode};
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

    /// "Forget everything" is removing this application's data right now (T643): no task is
    /// admitted until it is over.
    #[error("the application's data is being removed — no task can start now")]
    Forgetting,
}

/// Why [`TaskEngine::try_claim`] did not give the key (T643).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimRefused {
    /// Somebody holds it already (T621).
    Taken,
    /// "Forget everything" is running (T643).
    Forgetting,
}

/// What stood in the way of "forget everything" (T643): the work still alive in the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StillWorking {
    /// Tasks in the living map — queued, running, or paused with their work still held —
    /// by kind.
    pub tasks: Vec<TaskKind>,
    /// Claims held by commands whose task does not exist yet (a deployment still connecting,
    /// T621). Such a command is about to become a task, and counts as one.
    pub claims: usize,
}

impl std::fmt::Display for StillWorking {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kinds: Vec<&str> = self.tasks.iter().map(|k| k.as_str()).collect();
        write!(
            f,
            "{} task(s) alive [{}], {} starting",
            self.tasks.len(),
            kinds.join(", "),
            self.claims
        )
    }
}

/// The engine closed to new work while "forget everything" runs (T643).
///
/// Given by [`TaskEngine::close_for_forgetting`]; the engine opens again when this is dropped —
/// on every way out of the removal, a failed one included.
pub struct Closed {
    gate: Arc<Mutex<bool>>,
}

impl Drop for Closed {
    fn drop(&mut self) {
        *self.gate.lock().unwrap_or_else(|e| e.into_inner()) = false;
    }
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
        /// What the task had to say that is not a failure (T415). Empty for most tasks.
        notices: Vec<Detail>,
    },
}

/// The controls of a live task.
struct LiveTask {
    kind: TaskKind,
    state: TaskState,
    cancel: CancellationToken,
    /// Raised while the task may not do its work: paused by a person, or waiting for its
    /// place in the lane (T650). Lowered only by [`TaskEngine::try_claim_lane`], under the
    /// same lock that makes it running.
    paused: Arc<Mutex<bool>>,
    resume: Arc<Notify>,
    /// Wakes the task's placer: the task has become `Queued` and wants a place (T650).
    place: Arc<Notify>,
    throttle: Arc<ProgressThrottle>,
    /// What the running task has said so far.
    ///
    /// Held here as well as in the context because `finish` is what ends the task, and by
    /// then the context has gone with the work. Shared, not copied: the task goes on adding
    /// to it while it runs.
    notices: Arc<Mutex<Vec<Detail>>>,
    /// The place in the queue: lower runs sooner (FR-083).
    position: i64,
    /// The progress last reported, for the event a change of state sends (T652): a pause
    /// or a carry-on is announced with the bar where it stands, not at zero.
    progress: Arc<Mutex<f64>>,
}

/// The living map, shared by the engine and every task's context.
type LiveMap = Arc<Mutex<HashMap<String, LiveTask>>>;

/// What to do when a task raised after a restart is dropped before its work ran (T653).
type Undo =
    Box<dyn FnOnce(TaskContext) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send>> + Send>;

/// What [`TaskEngine::start`] needs to put a task among the living, besides its work.
struct Starting {
    id: String,
    kind: TaskKind,
    /// `Queued` for a new task, `Paused` for one raised after a restart.
    initial: TaskState,
    position: i64,
    /// Where the bar stands — what a change of state is announced with (T652).
    progress: f64,
    undo: Option<Undo>,
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
    /// The stage last reported, so that a change of it can be told from a repeat.
    ///
    /// The difference is the whole of T412 and T414. A change has to get past the rate cap,
    /// because a stage that lasts less than a quarter of a second is still a stage somebody
    /// wants to see, and it has to be written down, because a task outlives the screen that
    /// was watching it. A repeat has to do neither: a task reporting the same stage four
    /// hundred times would otherwise write to the disk four hundred times and fill the
    /// interface with one word.
    last_stage: Arc<Mutex<Option<DetailCode>>>,
    /// What the task has to say that is not a failure — see `add_notice`.
    notices: Arc<Mutex<Vec<Detail>>>,
    /// The last progress reported — see [`LiveTask::progress`].
    progress: Arc<Mutex<f64>>,
    /// The engine's living map, where the task's state is kept (T652). A progress report
    /// takes the state from here, under the lock every change of state is made under, so an
    /// event never says `Running` about a task already paused. `None` for a detached context.
    live: Option<LiveMap>,
    events: broadcast::Sender<TaskEvent>,
    db: Arc<Db>,
}

impl TaskContext {
    /// A context attached to nothing, for checking work that takes one.
    ///
    /// **Why this exists at all.** The constitution counts logic that can only be exercised
    /// through a server as unchecked, and the same applies to logic that can only be reached
    /// through a private constructor. `tasks::deploy::run` takes a context, and what it does
    /// with one — copying the server's settings aside before touching them (FR-095) — cannot
    /// be seen from `server::deploy::run`, which is a layer below and does none of it. A check
    /// written against that lower layer would pass whatever the runner did, and did: it was
    /// written first, the defect was put back, and it stayed green.
    ///
    /// **It is not a task.** Nothing is in the engine's living map under this identifier, so
    /// nothing here can be cancelled, paused, listed or persisted as a task — progress goes
    /// into a channel with no listener and notices into a vector nobody reads. That is the
    /// whole of what makes it safe to hand out: it cannot be mistaken for the real thing,
    /// because it does not behave like one.
    pub fn detached(db: std::sync::Arc<Db>) -> Self {
        let (events, _) = broadcast::channel(1);
        Self {
            id: String::from("detached"),
            cancel: CancellationToken::new(),
            paused: Arc::new(Mutex::new(false)),
            resume: Arc::new(Notify::new()),
            throttle: Arc::new(ProgressThrottle::default()),
            persist_throttle: Arc::new(ProgressThrottle::new(PROGRESS_PERSIST_INTERVAL)),
            last_stage: Arc::new(Mutex::new(None)),
            notices: Arc::new(Mutex::new(Vec::new())),
            progress: Arc::new(Mutex::new(0.0)),
            live: None,
            events,
            db,
        }
    }

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

    /// Report progress along with the transfer's figures. `None` is "not known yet" and goes
    /// out as `null` — never as a zero, which would say nothing is moving (T659).
    pub fn report_transfer(&self, progress: f64, speed_bps: Option<i64>, eta_s: Option<i64>) {
        self.report_full(progress, None, speed_bps, eta_s, false);
    }

    /// The same, under a stage of its own (T672): a variant on its way to the server says how
    /// fast it is going, as an upload does, without losing the stage that says what it is.
    pub fn report_stage_transfer(
        &self,
        progress: f64,
        stage: DetailCode,
        speed_bps: Option<i64>,
        eta_s: Option<i64>,
    ) {
        self.report_full(progress, Some(stage), speed_bps, eta_s, false);
    }

    /// Say something that is not progress and is not a failure.
    ///
    /// **Why the context and not the return value.** A task returns `Ok(())` or an error,
    /// and widening that would mean touching every kind of task for the sake of the three
    /// that have something to add. Through the context every kind gets it at once, and a
    /// notice can be raised in the middle of the work rather than only at the end — which
    /// matters most for the task that then fails: what it managed to do before it stopped
    /// is exactly what tells a person how much is still standing.
    pub fn add_notice(&self, notice: Detail) {
        self.notices
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(notice);
    }

    /// Say what this task produced, for a person to go and look at (T519(3)).
    ///
    /// Written to the database at once rather than held until the task ends, unlike
    /// [`Self::add_notice`]: only `Upload` and `BuildLadder` ever call this, each exactly
    /// once, near the very end of their own work, and by the time they do the tie to the
    /// medium is already known to hold. There is nothing later to wait for, and holding it
    /// in memory would only risk losing it if the process were killed between here and
    /// `finish`.
    ///
    /// Failures are swallowed and logged, the same choice as `note_stage`: a result is a
    /// link to somewhere else, and a task that otherwise succeeded must not be reported as
    /// failed for the sake of one write that could not land.
    pub fn set_result(&self, result: super::store::TaskResult) {
        if let Err(e) = store::save_result(&self.db, &self.id, &result) {
            tracing::warn!(id = %self.id, error = %e, "the task's result was not written down");
        }
    }

    /// A message that must get through regardless of the rate cap: a change of stage, the
    /// end of the work.
    pub fn report_important(&self, progress: f64, stage: DetailCode) {
        self.report_full(progress, Some(stage), None, None, true);
    }

    /// Take note of the stage, and say whether it is a new one.
    ///
    /// The writing to the database happens here, on the change and nowhere else. It is
    /// swallowed on failure for the same reason `save_progress` swallows its own: a stage is
    /// a label, and a transfer running for hours must not be brought down by one.
    fn note_stage(&self, stage: Option<DetailCode>) -> bool {
        let Some(stage) = stage else {
            return false;
        };
        {
            let mut last = self.last_stage.lock().unwrap_or_else(|e| e.into_inner());
            if *last == Some(stage) {
                return false;
            }
            *last = Some(stage);
        }
        if let Err(e) = store::save_stage(&self.db, &self.id, stage) {
            tracing::debug!(id = %self.id, error = %e, "the stage was not written");
        }
        true
    }

    fn report_full(
        &self,
        progress: f64,
        stage: Option<DetailCode>,
        speed_bps: Option<i64>,
        eta_s: Option<i64>,
        important: bool,
    ) {
        // A change of stage carries itself: the caller does not have to remember to mark it
        // important, and cannot mark a repeat important by mistake. Reporting a stage is the
        // ordinary way to report progress — `convert` names its stage on every line ffmpeg
        // prints — so the decision belongs here rather than at four hundred call sites.
        let changed = self.note_stage(stage);
        if !self.throttle.allow(important || changed) {
            return;
        }
        let progress = progress.clamp(0.0, 1.0);
        *self.progress.lock().unwrap_or_else(|e| e.into_inner()) = progress;
        let event = |state| TaskEvent::Progress {
            id: self.id.clone(),
            state,
            progress,
            stage,
            speed_bps,
            eta_s,
        };
        // **A report of bytes is not a change of state** (T652, QA-24A №3). The state the
        // event carries is the one the engine holds right now, read and sent under the lock
        // every change of state is made under — so the events go out in the order the
        // changes happened. A window that was already being written when "pause" was pressed
        // finishes and reports its bytes as `Paused`, not `Running`.
        let Some(live) = &self.live else {
            let _ = self.events.send(event(TaskState::Running));
            return;
        };
        let live = live.lock().unwrap_or_else(|e| e.into_inner());
        // A task no longer among the living has ended, and its `Done` has gone out: a late
        // report must not come after it.
        if let Some(t) = live.get(&self.id) {
            let _ = self.events.send(event(t.state));
        }
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

    /// The database, for a task that keeps a fact of its own beside its progress — how fast
    /// the encoder ran (T672). Not for the task's own record: that is the engine's.
    pub(crate) fn db(&self) -> &Db {
        &self.db
    }
}

/// How an attempt to take a place in a lane ended.
enum ClaimOutcome {
    /// The place was taken; the task is now running and its work may move.
    Started,
    /// The lane is full, or somebody stands ahead — wait and try again.
    Busy,
    /// The task is not waiting for a place: it is running, or paused and waiting for a
    /// person. Nothing to do until it becomes `Queued` again.
    NotWaiting,
    /// The task is no longer among the living (cancelled or finished).
    Gone,
}

/// The task machinery.
#[derive(Clone)]
pub struct TaskEngine {
    db: Arc<Db>,
    live: Arc<Mutex<HashMap<String, LiveTask>>>,
    /// Behind a lock, and shared rather than owned by value, for the same reason as
    /// `live`: `TaskEngine` is `Clone` and handed out to every command, so a limit changed
    /// on one handle (T546 — `concurrent_heavy_tasks`, changed through `settings_set`
    /// without a restart) must be seen by every other handle already given out, not just
    /// the one that changed it.
    limits: Arc<std::sync::RwLock<LaneLimits>>,
    events: broadcast::Sender<TaskEvent>,
    /// The place the next task submitted will get.
    next_position: Arc<std::sync::atomic::AtomicI64>,
    /// Keys taken by [`TaskEngine::claim`] (T621). Shared by every clone, like `live`.
    claims: Arc<Mutex<std::collections::HashSet<String>>>,
    /// Raised while "forget everything" runs (T643) — see [`TaskEngine::close_for_forgetting`].
    ///
    /// **One lock that every way in takes first.** Claiming a key and creating a task (and
    /// raising one after a restart) check it and do their work while holding it; closing takes
    /// it, then looks at the claims and the living, and raises it — all under it. So "nothing
    /// is running" and "nothing may start" are one step: there is no moment between the look
    /// and the closing in which a task could slip in. Order: this one, then `claims`, then
    /// `live` — never the other way round.
    closed: Arc<Mutex<bool>>,
}

/// The right to do one thing, held by one caller at a time (T621).
///
/// Taken by [`TaskEngine::claim`], given back when dropped — on every way out of the command
/// that took it before its task exists, and, once moved into the task's work, when that work
/// ends.
pub struct Claim {
    claims: Arc<Mutex<std::collections::HashSet<String>>>,
    key: String,
}

impl Drop for Claim {
    fn drop(&mut self) {
        let mut claims = self.claims.lock().unwrap_or_else(|e| e.into_inner());
        claims.remove(&self.key);
    }
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
            limits: Arc::new(std::sync::RwLock::new(LaneLimits::default())),
            events,
            next_position: Arc::new(std::sync::atomic::AtomicI64::new(next)),
            claims: Arc::new(Mutex::new(std::collections::HashSet::new())),
            closed: Arc::new(Mutex::new(false)),
        }
    }

    /// Take `key` for this caller, or `None` when somebody holds it already (T621) — or when
    /// "forget everything" is running (T643); [`TaskEngine::try_claim`] says which.
    ///
    /// Check-and-take under one lock: of two callers at the same instant exactly one gets it.
    /// This is what a scan of the task list cannot give a command that does slow work (a
    /// connection, a DNS wait) between its check and its `submit`.
    pub fn claim(&self, key: &str) -> Option<Claim> {
        self.try_claim(key).ok()
    }

    /// The same as [`TaskEngine::claim`], saying why it was refused (T643).
    pub fn try_claim(&self, key: &str) -> std::result::Result<Claim, ClaimRefused> {
        let closed = self.closed.lock().unwrap_or_else(|e| e.into_inner());
        if *closed {
            return Err(ClaimRefused::Forgetting);
        }
        let mut claims = self.claims.lock().unwrap_or_else(|e| e.into_inner());
        if !claims.insert(key.to_owned()) {
            return Err(ClaimRefused::Taken);
        }
        drop(claims);
        drop(closed);
        Ok(Claim {
            claims: self.claims.clone(),
            key: key.to_owned(),
        })
    }

    /// Close the engine to new work for the length of "forget everything" (T643, the owner's
    /// decision of 2026-09-30).
    ///
    /// **Refused while any work is alive**: a task in the living map — queued, running, or
    /// paused with its work held in memory (a paused one can be carried on at any moment, and
    /// a carried-on one needs no permission from here) — or a claim held by a command whose
    /// task does not exist yet. The removal must not race a task that writes a secret back
    /// (QA-22 №3: a deployment's `key_keeper` re-created the key after the report said it was
    /// gone), and a lock around the removal alone would not stop that: the task would write
    /// once the lock was let go.
    ///
    /// **When it is given, nothing starts until it is dropped**: [`TaskEngine::try_claim`]
    /// answers `Forgetting`, and creating or raising a task answers [`TaskError::Forgetting`].
    /// The look and the closing are one step under [`TaskEngine::closed`]'s lock.
    ///
    /// Two removals at once: the second is refused as `Err(None)`.
    pub fn close_for_forgetting(&self) -> std::result::Result<Closed, Option<StillWorking>> {
        let mut closed = self.closed.lock().unwrap_or_else(|e| e.into_inner());
        if *closed {
            return Err(None);
        }
        let claims = self.claims.lock().unwrap_or_else(|e| e.into_inner()).len();
        let tasks: Vec<TaskKind> = self
            .live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .map(|t| t.kind)
            .collect();
        if claims > 0 || !tasks.is_empty() {
            return Err(Some(StillWorking { tasks, claims }));
        }
        *closed = true;
        Ok(Closed {
            gate: self.closed.clone(),
        })
    }

    /// Whether "forget everything" is running right now (T643).
    pub fn is_closed_for_forgetting(&self) -> bool {
        *self.closed.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Set the per-lane limits at construction — used by `AppState::with_db` (T546) to
    /// carry over `concurrent_heavy_tasks` from `Settings`, and by tests that want fixed,
    /// small lanes to make contention easy to provoke.
    pub fn with_limits(self, limits: LaneLimits) -> Self {
        self.set_limits(limits);
        self
    }

    /// Change the per-lane limits on an engine already handed out (T546).
    ///
    /// Every clone of this `TaskEngine` shares the same `Arc<RwLock<..>>`, so this is seen
    /// at once by every command holding one — no restart needed, the way `settings_set`
    /// already tells a running viewer watch about a new threshold without one.
    pub fn set_limits(&self, limits: LaneLimits) {
        match self.limits.write() {
            Ok(mut guard) => *guard = limits,
            Err(e) => tracing::error!(error = %e, "the lane limits lock was poisoned"),
        }
    }

    /// The limits currently in force.
    ///
    /// Not `#[cfg(test)]`: `tests/unit/engine.rs` is a separate integration-test crate that
    /// links the library built without `cfg(test)`, so a test-gated method would not exist
    /// for it to call. Read-only and harmless outside tests too — commands still go through
    /// `has_room_for`/the claim path to actually use a limit.
    pub fn limits(&self) -> LaneLimits {
        self.limits.read().map(|g| *g).unwrap_or_else(|e| {
            tracing::error!(error = %e, "the lane limits lock was poisoned — using defaults");
            LaneLimits::default()
        })
    }

    /// Subscribe to task events.
    pub fn subscribe(&self) -> broadcast::Receiver<TaskEvent> {
        self.events.subscribe()
    }

    /// Stop every task of one batch that has not finished (T445).
    ///
    /// **Reads the database rather than the living.** A batch's later tasks are queued, not
    /// running, and a cancel that only reached what was running would stop the film in hand
    /// and let the next nine start — which is the opposite of what the button says.
    ///
    /// Returns how many were stopped. Cancelling one that has already finished is quiet, so a
    /// batch half over is stopped without a fuss (constitution, principle V).
    pub fn cancel_batch(&self, batch_id: &str) -> Result<usize> {
        let ids = store::unfinished_in_batch(&self.db, batch_id)?;
        let mut stopped = 0usize;
        for id in ids {
            // One that finished between the listing and here is not a failure of this: the
            // listing on any screen lags, and so does this one.
            match self.cancel(&id) {
                Ok(()) => stopped += 1,
                Err(e) => tracing::debug!(id, error = %e, "a task of the batch would not stop"),
            }
        }
        Ok(stopped)
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
        self.running_in_lane(lane) < self.limits().for_lane(lane)
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
        self.submit_in_batch(kind, server_id, None, work).await
    }

    /// The same, and say which batch it belongs to (T445).
    ///
    /// A separate way in rather than a fourth argument on `submit`: every task in the
    /// application goes through that one, and almost none of them is part of a batch. Making
    /// them all name it would be forty call sites saying `None`.
    pub async fn submit_in_batch<F, Fut>(
        &self,
        kind: TaskKind,
        server_id: Option<String>,
        batch: Option<store::Batch>,
        work: F,
    ) -> Result<String>
    where
        F: FnOnce(TaskContext) -> Fut + Send + 'static,
        Fut: Future<Output = std::result::Result<(), AppError>> + Send + 'static,
    {
        let id = uuid::Uuid::new_v4().to_string();
        let mut record = TaskRecord::new(id.clone(), kind, server_id);
        record.batch = batch;
        // T643: checked and held until the task is among the living, so "forget everything"
        // either sees it or it sees the engine closed — never neither.
        let closed = self.closed.lock().unwrap_or_else(|e| e.into_inner());
        if *closed {
            return Err(TaskError::Forgetting);
        }
        record.queue_order = self
            .next_position
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        store::upsert(&self.db, &record)?;
        self.start(
            Starting {
                id: id.clone(),
                kind,
                initial: TaskState::Queued,
                position: record.queue_order,
                progress: 0.0,
                undo: None,
            },
            work,
        );
        drop(closed);
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
        self.raise(id, work, None)
    }

    /// The same, with what to do if the task is dropped before its work ever ran (T653).
    ///
    /// **Tidying up after a task from the previous run is an action of its own, not a side
    /// effect of its work.** A raised upload may have left a part-file on the server, and the
    /// only thing that ever removed one was the transfer itself, on its way out after a
    /// cancellation. A task cancelled before its work was called — which is exactly a person
    /// deciding, after a restart, not to carry on — never went that way, and the part-file
    /// stayed on the server for good (QA-24A №4, FR-038).
    ///
    /// `undo` runs in place of the work, when a cancellation arrives while the task is still
    /// waiting for a person or for its place; the task is written down as cancelled once it
    /// has returned, as with the work (principle III). Once the work has begun, undoing is
    /// the work's own business, and `undo` is not called.
    pub fn resubmit_paused_with_undo<F, Fut, U, UFut>(
        &self,
        id: &str,
        work: F,
        undo: U,
    ) -> Result<()>
    where
        F: FnOnce(TaskContext) -> Fut + Send + 'static,
        Fut: Future<Output = std::result::Result<(), AppError>> + Send + 'static,
        U: FnOnce(TaskContext) -> UFut + Send + 'static,
        UFut: Future<Output = ()> + Send + 'static,
    {
        self.raise(id, work, Some(Box::new(move |ctx| Box::pin(undo(ctx)))))
    }

    fn raise<F, Fut>(&self, id: &str, work: F, undo: Option<Undo>) -> Result<()>
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
        // T643: the same gate as `submit_in_batch`, held until the task is among the living.
        let closed = self.closed.lock().unwrap_or_else(|e| e.into_inner());
        if *closed {
            return Err(TaskError::Forgetting);
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
            Starting {
                id: id.to_owned(),
                kind: record.kind,
                initial: TaskState::Paused,
                position: record.queue_order,
                progress: record.progress,
                undo,
            },
            work,
        );
        Ok(())
    }

    /// The part both ways of submitting share: create the live task and start its work.
    ///
    /// **One way into the lane, for every task and every time** (T650). A new task, a task
    /// raised after a restart and carried on, and a task paused mid-work and carried on all
    /// go the same road: they become `Queued`, and the task's *placer* — a loop living as long
    /// as the task — takes a place for them through [`TaskEngine::try_claim_lane`], which is
    /// the only thing that makes a task `Running` and lowers the flag its work waits on.
    /// Carrying on used to set `Running` directly, so a carried-on task either took no place
    /// at all (two transfers at once under a limit of one) or counted as running while it
    /// waited for one — and two such tasks each saw the other and waited for ever (QA-24A №1).
    fn start<F, Fut>(&self, starting: Starting, work: F)
    where
        F: FnOnce(TaskContext) -> Fut + Send + 'static,
        Fut: Future<Output = std::result::Result<(), AppError>> + Send + 'static,
    {
        let Starting {
            id,
            kind,
            initial,
            position,
            progress,
            undo,
        } = starting;
        let cancel = CancellationToken::new();
        // Raised for everyone at the start: a queued task has no place yet, a raised one is
        // waiting for a person. Only a place in the lane lowers it.
        let paused = Arc::new(Mutex::new(true));
        let resume = Arc::new(Notify::new());
        let place = Arc::new(Notify::new());
        let throttle = Arc::new(ProgressThrottle::default());
        let notices: Arc<Mutex<Vec<Detail>>> = Arc::new(Mutex::new(Vec::new()));
        let progress = Arc::new(Mutex::new(progress));

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
                    place: place.clone(),
                    throttle: throttle.clone(),
                    notices: notices.clone(),
                    position,
                    progress: progress.clone(),
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
            last_stage: Arc::new(Mutex::new(None)),
            notices,
            progress,
            live: Some(self.live.clone()),
            events: self.events.clone(),
            db: self.db.clone(),
        };

        let engine = self.clone();
        let task_id = id.clone();
        let paused_flag = ctx.paused.clone();
        let resume_signal = ctx.resume.clone();
        let placer = self.clone().placer(id, place);
        let run = async move {
            // Waiting for the first place — or, for a task raised after a restart, for a
            // person and then a place. Such a task **takes up no lane** while it waits:
            // otherwise it would hold a place while doing nothing. Cancelling works here too:
            // a task standing in the queue can be dropped without waiting for it to start —
            // and one raised after a restart is tidied up after (`undo`, T653).
            let dropped = |ctx: TaskContext| async move {
                if let Some(undo) = undo {
                    undo(ctx).await;
                }
            };
            loop {
                let resumed = resume_signal.notified();
                if cancel.is_cancelled() {
                    dropped(ctx).await;
                    engine.finish(&task_id, TaskState::Cancelled, None);
                    return;
                }
                if !*paused_flag.lock().unwrap_or_else(|e| e.into_inner()) {
                    break;
                }
                tokio::select! {
                    _ = resumed => {}
                    _ = cancel.cancelled() => {
                        dropped(ctx).await;
                        engine.finish(&task_id, TaskState::Cancelled, None);
                        return;
                    }
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
        };
        tokio::spawn(async move {
            // The placer never ends by itself; it goes when the task does.
            tokio::select! {
                _ = run => {}
                _ = placer => {}
            }
        });
    }

    /// The loop that takes a place in the lane whenever the task stands `Queued` (T650).
    ///
    /// It lives beside the task's work for the task's whole life, so a task paused and
    /// carried on in the middle of its work finds its place exactly as a new one does. Asleep
    /// while the task is running or paused; woken by [`TaskEngine::resume`] through `place`
    /// (a permit is kept, so a wake that comes before the sleep is not lost).
    async fn placer(self, id: String, place: Arc<Notify>) {
        loop {
            match self.try_claim_lane(&id) {
                ClaimOutcome::Busy => {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                ClaimOutcome::Started | ClaimOutcome::NotWaiting => place.notified().await,
                // The task's own work is what ends the task; the placer only stops trying.
                ClaimOutcome::Gone => std::future::pending::<()>().await,
            }
        }
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
                    // A task cancelled while it was only ever in the database never ran, so
                    // it never said anything.
                    notices: Vec::new(),
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
        // Under the lock, for the same reason as in `resume`: the order the states reach the
        // database is the order they happened in.
        self.persist_state(id, TaskState::Paused, None);
        self.announce(id, t);
        drop(live);
        Ok(())
    }

    /// Carry on a paused task.
    ///
    /// **Carrying on asks for a place; it does not take one** (T650). The task becomes
    /// `Queued` and its placer is woken; it becomes `Running`, and its work moves, only when
    /// [`TaskEngine::try_claim_lane`] finds room in its lane and nobody standing ahead of it.
    /// A task already waiting or already running is left as it is — pressing twice is not an
    /// error (constitution, principle V). So is one already being stopped: it is on its way
    /// out, tidying up after itself (T653), and must not be given a place for that.
    pub fn resume(&self, id: &str) -> Result<()> {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        let t = live
            .get_mut(id)
            .ok_or_else(|| TaskError::NotFound(id.to_owned()))?;
        if t.cancel.is_cancelled() {
            return Ok(());
        }

        match t.state {
            TaskState::Queued | TaskState::Running => return Ok(()),
            TaskState::Paused => {}
            other => {
                return Err(TaskError::BadTransition {
                    id: id.to_owned(),
                    from: other.as_str(),
                    to: TaskState::Queued.as_str(),
                })
            }
        }

        t.state = TaskState::Queued;
        // One permit, kept if the placer is not asleep yet.
        t.place.notify_one();
        // Written before the lock is let go: the placer may take the place the moment it is,
        // and its `Running` must not be overwritten by this `Queued` arriving late.
        self.persist_state(id, TaskState::Queued, None);
        self.announce(id, t);
        drop(live);
        // With room in the lane the answer to "carry on" is already `Running` — the same
        // atomic claim the placer makes, only sooner; without room the placer keeps trying.
        let _ = self.try_claim_lane(id);
        Ok(())
    }

    /// The list of tasks from the database — finished ones and leftovers from previous
    /// runs included.
    pub fn list(&self) -> Result<Vec<TaskRecord>> {
        let mut out = store::list(&self.db)?;
        for task in &mut out {
            task.can_resume = self.could_carry_on(task);
        }
        Ok(out)
    }

    pub fn get(&self, id: &str) -> Result<Option<TaskRecord>> {
        Ok(store::get(&self.db, id)?.map(|mut task| {
            task.can_resume = self.could_carry_on(&task);
            task
        }))
    }

    /// Whether this task's work is held by this run — queued, running or paused (T672).
    ///
    /// After a restart a task of the previous run is a row and nothing more: no work behind
    /// it, nothing to pause, carry on or wait for. A video in work asks this to tell its own
    /// task from such a row before deciding to start the stage again.
    pub fn is_alive(&self, id: &str) -> bool {
        self.live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(id)
    }

    /// Whether "carry on" would do anything for this task (T515).
    ///
    /// **Membership in the living map, which is what `resume` needs and nothing else is.**
    /// After a restart only an upload is raised into it; a measurement, a build and a
    /// deployment are rows and nothing more, and `resume` answers `TaskNotFound` — a phrase
    /// about an identifier, offered to somebody looking at the task on their screen.
    ///
    /// The state is checked too, so the answer is about this moment rather than about the
    /// kind: a running task is not resumed, it is already going.
    fn could_carry_on(&self, task: &TaskRecord) -> bool {
        if task.state != TaskState::Paused {
            return false;
        }
        self.live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&task.id)
    }

    /// Take a place in the lane and become running — atomically, under one lock.
    ///
    /// Checking for room and changing the state are inseparable on purpose: two tasks that
    /// wake at the same moment would otherwise both see one free place — and both would
    /// start, two preparations in a lane meant for one.
    ///
    /// **The only way into `Running`** (T650): only a task standing `Queued` is placed —
    /// whether it was just submitted or carried on after a pause or a restart — and only
    /// here is the flag its work waits on lowered. So a task never counts as running while it
    /// waits, and never works without a place.
    fn try_claim_lane(&self, id: &str) -> ClaimOutcome {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        let Some(t) = live.get(id) else {
            return ClaimOutcome::Gone;
        };
        if t.state != TaskState::Queued {
            return ClaimOutcome::NotWaiting;
        }
        let lane = t.kind.lane();
        let position = t.position;
        let used = live
            .values()
            .filter(|x| x.state.occupies_lane() && x.kind.lane() == lane)
            .count();
        if used >= self.limits().for_lane(lane) {
            return ClaimOutcome::Busy;
        }

        // The queue is honoured: the place goes to the task standing first in the lane.
        // Without this check the order would be "whoever grabbed the lock first", and
        // reordering (FR-083) would give nothing — there would be nothing to reorder. A task
        // carried on keeps the place it was given when it was first submitted.
        let someone_is_ahead = live.iter().any(|(other, x)| {
            other.as_str() != id
                && x.state == TaskState::Queued
                && x.kind.lane() == lane
                && x.position < position
        });
        if someone_is_ahead {
            return ClaimOutcome::Busy;
        }

        let Some(t) = live.get_mut(id) else {
            return ClaimOutcome::Gone;
        };
        t.state = TaskState::Running;
        t.throttle.reset();
        *t.paused.lock().unwrap_or_else(|e| e.into_inner()) = false;
        t.resume.notify_waiters();
        self.persist_state(id, TaskState::Running, None);
        self.announce(id, t);
        drop(live);
        ClaimOutcome::Started
    }

    fn finish(&self, id: &str, state: TaskState, error: Option<AppError>) {
        let notices = {
            let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(t) = live.get_mut(id) {
                t.state = state;
            }
            // Taken before the task leaves the living, which is the last moment anything
            // knows where to find them.
            let said = live
                .get(id)
                .map(|t| t.notices.lock().unwrap_or_else(|e| e.into_inner()).clone())
                .unwrap_or_default();
            live.remove(id);
            said
        };
        self.persist_state(id, state, error.clone());
        // After the state, not before: `save_notices` writes to a record that is already
        // over, and writing them first would put them on a task the crash-recovery pass
        // could still take for running.
        if let Err(e) = store::save_notices(&self.db, id, &notices) {
            tracing::warn!(id, error = %e, "what the task had to say was not written down");
        }
        let _ = self.events.send(TaskEvent::Done {
            id: id.to_owned(),
            state,
            error,
            notices,
        });
    }

    /// Tell the interface a task changed state (T652).
    ///
    /// Called by every change of state the engine makes short of the end — paused, waiting
    /// for its turn, running — with the living map's lock held (the borrowed `LiveTask` is the
    /// proof), the same lock a progress report sends under. So the interface learns the state
    /// from the engine's own transition, in the order the transitions happened, and a report
    /// of bytes that was already on its way can no longer overtake a pause.
    fn announce(&self, id: &str, t: &LiveTask) {
        let _ = self.events.send(TaskEvent::Progress {
            id: id.to_owned(),
            state: t.state,
            progress: *t.progress.lock().unwrap_or_else(|e| e.into_inner()),
            // Nothing is said about the stage; the speed and time left are unknown from here.
            stage: None,
            speed_bps: None,
            eta_s: None,
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
            T::Forgetting => ErrorCode::ForgetInProgress,
        };
        AppError::new(code).with_cause(e)
    }
}
