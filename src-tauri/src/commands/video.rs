//! T672 — one video, the whole way, in one place (the owner's decision of 2026-10-01).
//!
//! Contract: `contracts/ipc-commands.md`, the videos section (described in the T672 report).
//!
//! **Not a second pipeline.** A video is a film on its way and what is remembered about it —
//! the source, the medium in the library, the stage, the problem it stopped on. The work is
//! done by what already does it: the measurement with its chain onwards (`MeasureRequest.
//! then_build`, T438) and `ladder_build`, which recognises the rungs already on the server and
//! sends only what is missing. Every task a video starts carries the video's id as its batch,
//! so the build the measurement chains onto — created inside the measurement's own task, where
//! nothing here can be told about it — is still known to be this video's.
//!
//! **Where a video is, is read off its tasks' own stage codes** (`domain::video::stage_of`),
//! through the same event stream the task list listens to. Nothing here reports progress of
//! its own; it translates.
//!
//! **And it carries on after a restart** ([`api::restore_videos`]): a video that was going
//! starts its stage again — the measurement keeps every point it took, the build finds every
//! rung already on the server — and one a person paused stays paused.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::error::{AppError, DetailCode, ErrorCode, Result};
use super::AppState;
use crate::commands::library::DiskUsage;
use crate::domain::ladder::{Rung, SourceFacts};
use crate::domain::links::Links;
use crate::domain::source::SourceFile;
use crate::domain::video::{self, Act, VideoAction, VideoStage, VideoState};
use crate::domain::wording::Detail;
use crate::store::videos::{self as rows, VideoRow};
use crate::tasks::engine::TaskEvent;
use crate::tasks::state::{TaskKind, TaskState};
use crate::tasks::store::Batch;

// ---------- what the interface sees ----------

/// Where a plan's rungs come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanSource {
    /// Measured on this film: these are the rungs that will be built.
    Measured,
    /// Lent from another episode's measurement (FR-146).
    Borrowed,
    /// The formula's preview: the measurement after «Start» decides the real ones.
    Formula,
    /// Set by a person in the rung editor.
    Edited,
}

/// What the time to encode rests on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncodeEstimate {
    /// What this machine's encoder has actually done on earlier rungs.
    ThisMachine,
    /// A rough model: nothing has been encoded here yet.
    Model,
}

/// Whether there is room, with the numbers it rests on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SpaceCheck {
    Fits {
        needed_bytes: u64,
        free_bytes: u64,
    },
    Short {
        needed_bytes: u64,
        free_bytes: u64,
        short_by: u64,
    },
    /// Could not be asked (the server did not answer, the film's length is unknown). Not a
    /// refusal: the build asks again before a byte is made.
    Unknown,
}

/// The plan as shown before «Start».
#[derive(Debug, Clone, Serialize)]
pub struct VideoPlan {
    /// The rungs, heaviest first: measured, borrowed, the formula's preview, or edited.
    pub rungs: Vec<Rung>,
    pub from: PlanSource,
    /// Whether «Start» begins with measuring.
    pub needs_measuring: bool,
    /// Roughly how long measuring will take, in seconds. Nought when it is not needed.
    pub measure_s: u64,
    /// Roughly how long encoding the rungs will take, in seconds. `None` when the film's
    /// length is unknown.
    pub encode_s: Option<u64>,
    pub encode_estimate: EncodeEstimate,
    /// The encoder that will do it, as FFmpeg names it.
    pub encoder: String,
    /// What the set will take on the server, in bytes.
    pub server_bytes: u64,
    /// What one rung takes here while it is made, in bytes (one at a time).
    pub local_bytes: u64,
    pub server_space: SpaceCheck,
    pub local_space: SpaceCheck,
    /// Whether the short name is already a medium's in the library. `None` when the
    /// library could not be read.
    pub name_taken: Option<bool>,
    /// What the checker says about these rungs (the same codes as the ladder screen).
    pub objections: Vec<Detail>,
    pub notices: Vec<Detail>,
}

/// What a video stopped on, and what can be pressed about it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoProblem {
    pub error: AppError,
    pub actions: Vec<VideoAction>,
}

/// How far the current stage has got.
#[derive(Debug, Clone, Serialize)]
pub struct VideoProgress {
    /// The state of the task doing the stage: `queued` while it waits for its turn.
    pub task_state: TaskState,
    /// From 0 to 1, within the stage — within the rung for encoding and sending.
    pub progress: f64,
    /// Bytes a second while a rung is being sent; `null` otherwise or while unknown.
    pub speed_bps: Option<i64>,
    /// Seconds left in the stage, when it can be said.
    pub eta_s: Option<i64>,
    /// Which rung the build is at, counting from one. `null` before the build.
    pub rung: Option<u32>,
    /// How many rungs there are.
    pub rungs: u32,
}

/// One video, as the screen shows it.
#[derive(Debug, Clone, Serialize)]
pub struct VideoView {
    pub id: String,
    pub server_id: String,
    pub source_path: String,
    pub title: String,
    pub slug: String,
    pub audio_track: usize,
    pub stage: VideoStage,
    pub state: VideoState,
    /// Paused by a person — stays paused across a restart.
    pub paused_by_person: bool,
    /// «Start» was pressed while the plan was still being made.
    pub start_requested: bool,
    /// What the source turned out to be, with its audio tracks.
    pub source: Option<SourceFile>,
    pub plan: Option<VideoPlan>,
    pub progress: Option<VideoProgress>,
    pub task_id: Option<String>,
    pub media_id: Option<String>,
    pub problem: Option<VideoProblem>,
    /// The link to the set — once it is served (`done`).
    pub link: Option<Links>,
    pub created_at: String,
    pub updated_at: String,
}

/// A file `video_add` would not take, and why.
#[derive(Debug, Clone, Serialize)]
pub struct VideoRefusal {
    pub path: String,
    pub error: AppError,
}

/// What `video_add` did, file by file (as T665).
#[derive(Debug, Clone, Serialize)]
pub struct VideoAdded {
    pub added: Vec<VideoView>,
    pub refused: Vec<VideoRefusal>,
}

/// What `video_start` did about one video.
#[derive(Debug, Clone, Serialize)]
pub struct VideoStarted {
    pub id: String,
    /// `null` — started, or will start the moment its plan is ready.
    pub error: Option<AppError>,
}

// ---------- what is kept ----------

/// The plan as worked out, before a person's choices are applied to it. What is shown is
/// worked out from this every time, so a changed audio track or an edited rung changes the
/// sizes, the time and the room at once rather than leaving them describing another plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanBasis {
    rungs: Vec<Rung>,
    from: PlanSource,
    needs_measuring: bool,
    measure_s: u64,
    encoder: String,
    pixels_per_s: f64,
    encode_estimate: EncodeEstimate,
    server_disk: Option<DiskUsage>,
    local_disk: Option<DiskUsage>,
    name_taken: Option<bool>,
    notices: Vec<Detail>,
}

// ---------- the hub: what this run holds about videos ----------

/// What this run holds about the videos, beside what is stored: the progress of the stage in
/// hand, the plans being made, and the one watcher of the task stream.
///
/// Shared between clones of [`AppState`], like the task engine.
#[derive(Clone, Default)]
pub struct VideoHub {
    inner: Arc<Hub>,
}

#[derive(Default)]
struct Hub {
    /// Held around every read-change-write of a video row, so the watcher and a command never
    /// write over each other. Never held across an `await`.
    rows: Mutex<()>,
    watching: AtomicBool,
    /// Which video a task belongs to, and its kind; `None` for a task that is nobody's.
    owners: Mutex<HashMap<String, Option<(String, TaskKind)>>>,
    live: Mutex<HashMap<String, Live>>,
    planning: Mutex<HashMap<String, CancellationToken>>,
    /// Videos whose next task is being put on the queue right now (the medium being made,
    /// the build's quick refusals being asked).
    starting: Mutex<HashSet<String>>,
}

struct Live {
    code: Option<DetailCode>,
    since: Instant,
    from: f64,
    progress: VideoProgress,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Run something in the background, on whichever runtime is there.
fn spawn<F>(work: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            handle.spawn(work);
        }
        Err(_) => {
            tauri::async_runtime::spawn(work);
        }
    }
}

fn not_found(id: &str) -> AppError {
    AppError::new(ErrorCode::VideoNotFound).with_cause(id)
}

fn not_now(row: &VideoRow) -> AppError {
    AppError::new(ErrorCode::VideoNotNow).with_cause(format!(
        "{}: {} at {}",
        row.id,
        row.state.as_str(),
        row.stage.as_str()
    ))
}

fn storage(e: crate::store::db::DbError) -> AppError {
    AppError::new(ErrorCode::StorageFailed).with_cause(e)
}

fn load(state: &AppState, id: &str) -> Result<VideoRow> {
    rows::get(&state.db, id)
        .map_err(storage)?
        .ok_or_else(|| not_found(id))
}

/// Read, change and write one video under the rows lock, then say so.
///
/// `f` returning an error leaves the row as it was.
fn change<T>(state: &AppState, id: &str, f: impl FnOnce(&mut VideoRow) -> Result<T>) -> Result<T> {
    let out = {
        let _held = lock(&state.videos.inner.rows);
        let mut row = load(state, id)?;
        let out = f(&mut row)?;
        rows::save(&state.db, &row).map_err(storage)?;
        out
    };
    emit(state, id);
    Ok(out)
}

fn emit(state: &AppState, id: &str) {
    if let Ok(row) = load(state, id) {
        let _ = state
            .events
            .send(super::AppEvent::VideoUpdate(Box::new(view_of(state, &row))));
    }
}

fn batch_of(row: &VideoRow) -> Batch {
    Batch {
        id: row.id.clone(),
        label: row.title.clone(),
    }
}

fn set_problem(row: &mut VideoRow, error: AppError) {
    let problem = VideoProblem {
        actions: video::actions_for(&error),
        error,
    };
    row.state = VideoState::Problem;
    row.paused_by_person = false;
    row.problem_json = serde_json::to_string(&problem).ok();
}

fn source_of(row: &VideoRow) -> Option<SourceFile> {
    row.source_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
}

fn basis_of(row: &VideoRow) -> Option<PlanBasis> {
    row.plan_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
}

fn custom_rungs(row: &VideoRow) -> Option<Vec<Rung>> {
    row.rungs_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
}

fn facts_of(source: &SourceFile) -> SourceFacts {
    SourceFacts {
        width: source.width,
        height: source.height,
        fps: source.fps,
        bitrate_bps: source.bitrate_bps,
        heavier_codec: source.video_codec.eq_ignore_ascii_case("hevc"),
        native_height: None,
    }
}

fn space(disk: Option<DiskUsage>, needed: u64) -> SpaceCheck {
    let Some(disk) = disk else {
        return SpaceCheck::Unknown;
    };
    if needed == 0 {
        return SpaceCheck::Unknown;
    }
    match crate::server::free_space::check(&disk, needed, 0) {
        crate::server::free_space::SpaceVerdict::Fits => SpaceCheck::Fits {
            needed_bytes: needed,
            free_bytes: disk.free_bytes,
        },
        crate::server::free_space::SpaceVerdict::NotEnough {
            needed,
            free,
            short_by,
        } => SpaceCheck::Short {
            needed_bytes: needed,
            free_bytes: free,
            short_by,
        },
    }
}

/// The plan as it stands with this video's own choices — its audio track, its edited rungs —
/// applied. Worked out every time, from the basis and the source.
fn effective_plan(row: &VideoRow, basis: &PlanBasis, source: &SourceFile) -> VideoPlan {
    use crate::domain::ladder_size::{bytes_for_rung, bytes_for_set, AUDIO_BUDGET_BPS};

    let custom = custom_rungs(row);
    let edited = custom.is_some();
    let rungs = custom.unwrap_or_else(|| basis.rungs.clone());
    let audio_bps = source
        .audio_tracks
        .get(row.audio_track)
        .and_then(|t| t.bitrate_bps)
        .unwrap_or(AUDIO_BUDGET_BPS)
        .max(AUDIO_BUDGET_BPS);
    let bitrates: Vec<u64> = rungs.iter().map(|r| r.bitrate_bps).collect();
    let server_bytes = bytes_for_set(&bitrates, audio_bps, source.duration_s);
    let local_bytes = bytes_for_rung(
        bitrates.iter().copied().max().unwrap_or(0),
        audio_bps,
        source.duration_s,
    );
    let objections = crate::domain::ladder::validate(&rungs, &facts_of(source), source.fps)
        .iter()
        .map(|o| o.detail())
        .collect();
    VideoPlan {
        encode_s: video::encode_seconds(&rungs, source, basis.pixels_per_s),
        from: if edited {
            PlanSource::Edited
        } else {
            basis.from
        },
        needs_measuring: !edited && basis.needs_measuring && !row.measured,
        measure_s: if !edited && basis.needs_measuring && !row.measured {
            basis.measure_s
        } else {
            0
        },
        encode_estimate: basis.encode_estimate,
        encoder: basis.encoder.clone(),
        server_space: space(basis.server_disk, server_bytes),
        local_space: space(basis.local_disk, local_bytes),
        server_bytes,
        local_bytes,
        name_taken: if row.media_id.is_some() {
            Some(false)
        } else {
            basis.name_taken
        },
        objections,
        notices: basis.notices.clone(),
        rungs,
    }
}

fn link_of(state: &AppState, row: &VideoRow) -> Option<Links> {
    if row.stage != VideoStage::Done {
        return None;
    }
    let profile = crate::store::profiles::get(&state.db, &row.server_id)
        .ok()
        .flatten()?;
    Some(crate::domain::links::for_path(
        &profile.domain,
        profile.cdn_base.as_deref(),
        &format!("{}/master.m3u8", row.slug),
    ))
}

fn view_of(state: &AppState, row: &VideoRow) -> VideoView {
    let source = source_of(row);
    let plan = match (basis_of(row), source.as_ref()) {
        (Some(basis), Some(source)) => Some(effective_plan(row, &basis, source)),
        _ => None,
    };
    let progress = if matches!(
        row.state,
        VideoState::Working | VideoState::Paused | VideoState::Cancelling
    ) {
        lock(&state.videos.inner.live)
            .get(&row.id)
            .map(|l| l.progress.clone())
    } else {
        None
    };
    VideoView {
        id: row.id.clone(),
        server_id: row.server_id.clone(),
        source_path: row.source_path.clone(),
        title: row.title.clone(),
        slug: row.slug.clone(),
        audio_track: row.audio_track,
        stage: row.stage,
        state: row.state,
        paused_by_person: row.paused_by_person,
        start_requested: row.start_requested,
        source,
        plan,
        progress,
        task_id: row.task_id.clone(),
        media_id: row.media_id.clone(),
        problem: row
            .problem_json
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok()),
        link: link_of(state, row),
        created_at: row.created_at.clone(),
        updated_at: row.updated_at.clone(),
    }
}

/// Every task of this video that has not finished, alive in this run or not.
fn unfinished(state: &AppState, id: &str) -> Vec<String> {
    crate::tasks::store::unfinished_in_batch(&state.db, id).unwrap_or_default()
}

fn alive(state: &AppState, id: &str) -> Vec<String> {
    unfinished(state, id)
        .into_iter()
        .filter(|t| state.tasks.is_alive(t))
        .collect()
}

// ---------- watching the tasks ----------

/// Start the one watcher of the task stream, if it is not running yet.
fn ensure_watching(state: &AppState) {
    if state.videos.inner.watching.swap(true, Ordering::SeqCst) {
        return;
    }
    let state = state.clone();
    let mut rx = state.tasks.subscribe();
    spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        loop {
            tokio::select! {
                got = rx.recv() => match got {
                    Ok(event) => on_task_event(&state, event),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => reconcile(&state),
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                },
                _ = tick.tick() => reconcile(&state),
            }
        }
    });
}

/// Which video a task is, by its batch — remembered once found.
fn owner_of(state: &AppState, task_id: &str) -> Option<(String, TaskKind)> {
    if let Some(known) = lock(&state.videos.inner.owners).get(task_id) {
        return known.clone();
    }
    let found = crate::tasks::store::get(&state.db, task_id)
        .ok()
        .flatten()
        .and_then(|t| {
            let batch = t.batch?;
            rows::get(&state.db, &batch.id)
                .ok()
                .flatten()
                .map(|_| (batch.id, t.kind))
        });
    lock(&state.videos.inner.owners).insert(task_id.to_owned(), found.clone());
    found
}

fn on_task_event(state: &AppState, event: TaskEvent) {
    match event {
        TaskEvent::Progress {
            id,
            state: task_state,
            progress,
            stage,
            speed_bps,
            eta_s,
        } => on_progress(state, &id, task_state, progress, stage, speed_bps, eta_s),
        TaskEvent::Done {
            id,
            state: task_state,
            error,
            ..
        } => on_done(state, &id, task_state, error),
    }
}

fn rungs_now(row: &VideoRow) -> u32 {
    custom_rungs(row)
        .map(|r| r.len())
        .or_else(|| basis_of(row).map(|b| b.rungs.len()))
        .unwrap_or(0) as u32
}

fn on_progress(
    state: &AppState,
    task_id: &str,
    task_state: TaskState,
    progress: f64,
    code: Option<DetailCode>,
    speed_bps: Option<i64>,
    eta_s: Option<i64>,
) {
    let Some((vid, kind)) = owner_of(state, task_id) else {
        return;
    };
    let mut pause_it = false;
    let mut refresh = false;
    let rungs;
    {
        let _held = lock(&state.videos.inner.rows);
        let Ok(mut row) = load(state, &vid) else {
            return;
        };
        if !matches!(
            row.state,
            VideoState::Working | VideoState::Paused | VideoState::Cancelling
        ) {
            return;
        }
        let mut dirty = false;
        // The build the measurement chained onto becomes the video's task the moment it is
        // heard from; a measurement never takes the place back from a build.
        let current_is_live = row
            .task_id
            .as_deref()
            .is_some_and(|t| state.tasks.is_alive(t));
        if row.task_id.as_deref() != Some(task_id)
            && !task_state.is_final()
            && (kind == TaskKind::BuildLadder || !current_is_live)
        {
            row.task_id = Some(task_id.to_owned());
            dirty = true;
        }
        if row.task_id.as_deref() != Some(task_id) {
            return;
        }
        if kind == TaskKind::MeasureQuality && code == Some(DetailCode::StageDone) && !row.measured
        {
            row.measured = true;
            refresh = true;
            dirty = true;
        }
        if let Some(stage) = code.and_then(video::stage_of) {
            if stage != row.stage {
                row.stage = stage;
                dirty = true;
            }
        }
        // A pause pressed while the task was still waiting for its turn is carried out the
        // moment it starts: a queued task cannot be paused, a running one can.
        if row.paused_by_person && task_state == TaskState::Running {
            pause_it = true;
        }
        rungs = rungs_now(&row);
        if dirty {
            let _ = rows::save(&state.db, &row);
        }
    }
    if pause_it {
        let _ = state.tasks.pause(task_id);
    }
    note_progress(
        state, &vid, task_state, progress, code, speed_bps, eta_s, rungs,
    );
    if refresh {
        refresh_rungs(state, &vid);
    }
    emit(state, &vid);
}

#[allow(clippy::too_many_arguments)]
fn note_progress(
    state: &AppState,
    vid: &str,
    task_state: TaskState,
    progress: f64,
    code: Option<DetailCode>,
    speed_bps: Option<i64>,
    eta_s: Option<i64>,
    rungs: u32,
) {
    let mut live = lock(&state.videos.inner.live);
    let entry = live.entry(vid.to_owned()).or_insert_with(|| Live {
        code: None,
        since: Instant::now(),
        from: 0.0,
        progress: VideoProgress {
            task_state,
            progress: 0.0,
            speed_bps: None,
            eta_s: None,
            rung: None,
            rungs,
        },
    });
    entry.progress.task_state = task_state;
    entry.progress.rungs = rungs.max(entry.progress.rungs);
    let Some(code) = code else {
        // A change of the task's state: where the bar stands is not news.
        return;
    };
    if code == DetailCode::StageBuildingLadder {
        // The build taking up its next rung: its share is of the whole set, and it is read
        // back as which rung this is.
        entry.progress.rung = Some(video::rung_at(progress, rungs as usize));
        entry.code = Some(code);
        entry.since = Instant::now();
        entry.from = 0.0;
        entry.progress.progress = 0.0;
        entry.progress.speed_bps = None;
        entry.progress.eta_s = None;
        return;
    }
    if entry.code != Some(code) {
        entry.code = Some(code);
        entry.since = Instant::now();
        entry.from = progress;
    }
    entry.progress.progress = progress.clamp(0.0, 1.0);
    entry.progress.speed_bps = speed_bps;
    entry.progress.eta_s = eta_s.or_else(|| {
        video::eta_s(
            entry.since.elapsed().as_secs_f64(),
            entry.from,
            entry.progress.progress,
        )
    });
}

fn on_done(state: &AppState, task_id: &str, task_state: TaskState, error: Option<AppError>) {
    let owner = owner_of(state, task_id);
    lock(&state.videos.inner.owners).remove(task_id);
    let Some((vid, kind)) = owner else {
        return;
    };
    let mut refresh = false;
    let mut carry = false;
    let ended;
    {
        let _held = lock(&state.videos.inner.rows);
        let Ok(mut row) = load(state, &vid) else {
            return;
        };
        let before = row.clone();
        let current = row.task_id.as_deref() == Some(task_id);
        let others: Vec<String> = alive(state, &vid)
            .into_iter()
            .filter(|t| t != task_id)
            .collect();

        if row.state == VideoState::Cancelling {
            if others.is_empty() && !lock(&state.videos.inner.starting).contains(&vid) {
                row.state = VideoState::Cancelled;
                row.task_id = None;
            }
        } else if matches!(row.state, VideoState::Working | VideoState::Paused) && current {
            match (kind, task_state) {
                (_, TaskState::Cancelled) => {
                    // Stopped from the task list rather than from the video: the video stops
                    // with it, and «Retry» carries on from here.
                    row.state = VideoState::Cancelled;
                    row.paused_by_person = false;
                    row.task_id = None;
                }
                (TaskKind::MeasureQuality, TaskState::Completed) => {
                    if !row.measured {
                        row.measured = true;
                        refresh = true;
                    }
                    if let Some(build) = others.first() {
                        row.task_id = Some(build.clone());
                        if row.stage < VideoStage::Encoding {
                            row.stage = VideoStage::Encoding;
                        }
                    } else {
                        // The measurement ended without a build behind it — it may have been
                        // cut off between the two. The build is put on the queue from here.
                        row.task_id = None;
                        carry = true;
                    }
                }
                (TaskKind::MeasureQuality, _) => {
                    let measured = crate::tasks::store::get(&state.db, task_id)
                        .ok()
                        .flatten()
                        .is_some_and(|t| t.stage == Some(DetailCode::StageDone));
                    if measured && !row.measured {
                        row.measured = true;
                        refresh = true;
                    }
                    if let Some(build) = others.first() {
                        row.task_id = Some(build.clone());
                    } else {
                        let error = error
                            .clone()
                            .unwrap_or_else(|| AppError::new(ErrorCode::Internal));
                        if measured && row.stage == VideoStage::Measuring {
                            // The chain stopped after the measurement (an objection, a viewer):
                            // the video is past measuring, and «Retry» goes to the build.
                            row.stage = VideoStage::Encoding;
                        }
                        set_problem(&mut row, error);
                        row.task_id = None;
                    }
                }
                (TaskKind::BuildLadder, TaskState::Completed) => {
                    row.stage = VideoStage::Done;
                    row.state = VideoState::Done;
                    row.problem_json = None;
                    row.paused_by_person = false;
                    if let Some(result) = crate::tasks::store::get(&state.db, task_id)
                        .ok()
                        .flatten()
                        .and_then(|t| t.result)
                    {
                        row.media_id = Some(result.media_id);
                    }
                }
                (TaskKind::BuildLadder, _) => {
                    let error = error
                        .clone()
                        .unwrap_or_else(|| AppError::new(ErrorCode::Internal));
                    set_problem(&mut row, error);
                    row.task_id = None;
                }
                _ => {}
            }
        }
        ended = !matches!(
            row.state,
            VideoState::Working | VideoState::Paused | VideoState::Cancelling
        );
        if row != before {
            let _ = rows::save(&state.db, &row);
        }
    }
    if ended {
        lock(&state.videos.inner.live).remove(&vid);
    }
    if refresh {
        refresh_rungs(state, &vid);
    }
    emit(state, &vid);
    if carry {
        let state = state.clone();
        spawn(async move { carry_on(&state, &vid).await });
    }
}

/// Catch what the stream did not say: a task that ended while nobody listened (the stream
/// fell behind), a stop that has no task left to wait for.
fn reconcile(state: &AppState) {
    let Ok(all) = rows::list(&state.db) else {
        return;
    };
    for row in all {
        if !matches!(
            row.state,
            VideoState::Working | VideoState::Paused | VideoState::Cancelling
        ) {
            continue;
        }
        if lock(&state.videos.inner.starting).contains(&row.id) {
            continue;
        }
        if let Some(task_id) = row.task_id.clone() {
            if state.tasks.is_alive(&task_id) {
                continue;
            }
            if let Ok(Some(task)) = state.tasks.get(&task_id) {
                if task.state.is_final() {
                    on_done(state, &task_id, task.state, task.error);
                }
            }
        } else if row.state == VideoState::Cancelling && alive(state, &row.id).is_empty() {
            let _ = change(state, &row.id, |r| {
                if r.state == VideoState::Cancelling {
                    r.state = VideoState::Cancelled;
                }
                Ok(())
            });
        }
    }
}

/// After the measurement the plan's rungs are the measured ones, not the formula's preview.
fn refresh_rungs(state: &AppState, vid: &str) {
    let state = state.clone();
    let vid = vid.to_owned();
    spawn(async move {
        let Ok(row) = load(&state, &vid) else { return };
        let Ok(preview) = super::ladder::api::ladder_plan_until(
            &state,
            &ladder_request(&row.source_path),
            Some(&CancellationToken::new()),
        )
        .await
        else {
            return;
        };
        let from = match preview.from {
            super::ladder::LadderSource::Measured => PlanSource::Measured,
            super::ladder::LadderSource::Borrowed => PlanSource::Borrowed,
            super::ladder::LadderSource::Formula => return,
        };
        let _ = change(&state, &vid, |r| {
            if let Some(mut basis) = basis_of(r) {
                basis.rungs = preview.plan.rungs.clone();
                basis.from = from;
                basis.needs_measuring = false;
                basis.measure_s = 0;
                r.plan_json = serde_json::to_string(&basis).ok();
            }
            Ok(())
        });
    });
}

fn ladder_request(path: &str) -> super::ladder::LadderRequest {
    super::ladder::LadderRequest {
        path: path.to_owned(),
        codec: String::from("h264"),
        native_height: None,
        declared_layout: None,
        measured_peak_bps: None,
        prefer_hardware: true,
    }
}

// ---------- planning ----------

fn begin_planning(state: &AppState, id: &str) {
    let cancel = CancellationToken::new();
    if let Some(old) = lock(&state.videos.inner.planning).insert(id.to_owned(), cancel.clone()) {
        old.cancel();
    }
    let state = state.clone();
    let id = id.to_owned();
    spawn(async move {
        let made = make_basis(&state, &id, &cancel).await;
        {
            // Stopped, or replaced by a newer planning of the same video: this answer is
            // nobody's any more.
            if cancel.is_cancelled() {
                return;
            }
            let mut planning = lock(&state.videos.inner.planning);
            if planning.get(&id).is_some_and(|t| t.is_cancelled()) {
                return;
            }
            planning.remove(&id);
        }
        let start = change(&state, &id, |row| {
            if row.state != VideoState::Planning {
                return Ok(false);
            }
            match made {
                Ok(basis) => {
                    row.plan_json = serde_json::to_string(&basis).ok();
                    row.problem_json = None;
                    if row.start_requested {
                        row.state = VideoState::Working;
                        Ok(true)
                    } else {
                        row.state = VideoState::Ready;
                        Ok(false)
                    }
                }
                Err(e) => {
                    set_problem(row, e);
                    Ok(false)
                }
            }
        })
        .unwrap_or(false);
        if start {
            carry_on(&state, &id).await;
        }
    });
}

async fn make_basis(state: &AppState, id: &str, cancel: &CancellationToken) -> Result<PlanBasis> {
    let row = load(state, id)?;
    let profile = super::library::api::profile_of(state, &row.server_id)?;

    let preview = super::ladder::api::ladder_plan_until(
        state,
        &ladder_request(&row.source_path),
        Some(cancel),
    )
    .await?;
    let from = match preview.from {
        super::ladder::LadderSource::Measured => PlanSource::Measured,
        super::ladder::LadderSource::Borrowed => PlanSource::Borrowed,
        super::ladder::LadderSource::Formula => PlanSource::Formula,
    };
    let needs_measuring = from == PlanSource::Formula;
    let measure_s = if needs_measuring {
        measure_estimate(state, &row.source_path, &preview)
    } else {
        0
    };

    let (encoder, _) = super::ladder::pick_encoder(true).await?;
    let (pixels_per_s, encode_estimate) = match rows::encode_speed(&state.db, encoder.ffmpeg_name())
    {
        Ok(Some(speed)) => (speed, EncodeEstimate::ThisMachine),
        _ => (
            match encoder {
                crate::media::encoders::Encoder::Hardware { .. } => {
                    video::MODEL_PIXELS_PER_S_HARDWARE
                }
                crate::media::encoders::Encoder::Software => video::MODEL_PIXELS_PER_S_SOFTWARE,
            },
            EncodeEstimate::Model,
        ),
    };

    // The server is asked once, briefly: how much room, and whether the name is taken. A
    // server that does not answer is not a refusal — the build asks again before a byte.
    let (server_disk, name_taken) = match tokio::time::timeout(
        Duration::from_secs(30),
        look_at_server(state, &profile, &row.slug),
    )
    .await
    {
        Ok(Ok((disk, taken))) => (Some(disk), Some(taken)),
        _ => (None, None),
    };

    let chosen = crate::store::settings::load(&state.db)
        .map(|s| s.work_dir)
        .unwrap_or_default();
    let work_dir = crate::domain::work_dir::for_source(
        chosen.as_deref(),
        std::path::Path::new(&row.source_path),
    );
    let local_disk = crate::media::local_disk::usage(&work_dir);

    Ok(PlanBasis {
        rungs: preview.plan.rungs,
        from,
        needs_measuring,
        measure_s,
        encoder: encoder.ffmpeg_name().to_owned(),
        pixels_per_s,
        encode_estimate,
        server_disk,
        local_disk,
        name_taken,
        notices: preview.notices,
    })
}

fn measure_estimate(state: &AppState, path: &str, preview: &super::ladder::LadderPreview) -> u64 {
    use crate::domain::measure_grid::{grid, seconds_per_point};
    let facts = preview.source;
    let anchor = preview
        .anchor_mbps
        .unwrap_or(crate::domain::ladder::FALLBACK_MBPS);
    let points = grid(&facts, anchor).len();
    let already = crate::store::measurements::key_for(std::path::Path::new(path))
        .ok()
        .and_then(|key| crate::store::measurements::points(&state.db, &key, "h264").ok())
        .map(|p| p.len())
        .unwrap_or(0);
    let per_point = seconds_per_point(
        facts.width,
        facts.height,
        facts.fps,
        crate::domain::chunks::CHUNK_S as u64,
        3,
    );
    let factor = crate::store::measurements::machine_factor(&state.db)
        .ok()
        .flatten()
        .map(|m| m.factor)
        .unwrap_or(1.0);
    (points.saturating_sub(already) as f64 * per_point * factor).round() as u64
}

async fn look_at_server(
    state: &AppState,
    profile: &crate::domain::server_profile::ServerProfile,
    slug: &str,
) -> Result<(DiskUsage, bool)> {
    let conn = crate::server::gate::open(
        state.secrets.as_ref(),
        profile,
        crate::server::gate::Intent::Read,
    )
    .await?
    .conn;
    let disk = crate::server::disk::usage(&conn, &profile.video_dir).await;
    let manifest = crate::server::manifest_io::read(&conn, &profile.video_dir).await;
    conn.close().await;
    Ok((disk?, !manifest?.slug_available(slug, None)))
}

// ---------- going ----------

/// Put the next task of this video on the queue, from the stage it is at.
///
/// Called for «Start», «Continue» with nothing left alive, «Retry», and at start-up for every
/// video that was going. The caller has set the video `working`.
async fn carry_on(state: &AppState, id: &str) {
    let ready = {
        let _held = lock(&state.videos.inner.rows);
        match load(state, id) {
            Ok(mut row) if row.state == VideoState::Working => {
                // Whatever task it had belongs to a run that is over, or has ended: it is let go
                // of here, so its ending cannot be taken for this video's.
                row.task_id = None;
                let _ = rows::save(&state.db, &row);
                lock(&state.videos.inner.starting).insert(id.to_owned());
                true
            }
            _ => false,
        }
    };
    if !ready {
        return;
    }
    // Rows of tasks from a run that is over have no work behind them; they are closed so that
    // nothing — the build's own «already running» guard included — takes them for live work.
    for task in unfinished(state, id) {
        if !state.tasks.is_alive(&task) {
            let _ = state.tasks.cancel(&task);
        }
    }

    let outcome = next_task(state, id).await;
    lock(&state.videos.inner.starting).remove(id);

    let mut stop_it = None;
    let mut pause_it = None;
    let _ = change(state, id, |row| {
        match outcome {
            Ok((task_id, stage)) => {
                match row.state {
                    VideoState::Cancelling | VideoState::Cancelled => {
                        row.state = VideoState::Cancelling;
                        stop_it = Some(task_id.clone());
                    }
                    _ => {
                        if row.paused_by_person {
                            pause_it = Some(task_id.clone());
                        }
                    }
                }
                // Only when the watcher has not already heard from a later task of this video
                // (the build the measurement chained onto may already be going).
                if row.task_id.is_none() {
                    row.task_id = Some(task_id);
                }
                if stage > row.stage {
                    row.stage = stage;
                }
            }
            Err(error) => match row.state {
                VideoState::Cancelling => row.state = VideoState::Cancelled,
                VideoState::Cancelled => {}
                _ => set_problem(row, error),
            },
        }
        Ok(())
    });
    if let Some(task) = stop_it {
        let _ = state.tasks.cancel(&task);
    }
    if let Some(task) = pause_it {
        let _ = state.tasks.pause(&task);
    }
}

/// The task that does this video's next stage: the measurement (chained on to the build), or
/// the build itself.
async fn next_task(state: &AppState, id: &str) -> Result<(String, VideoStage)> {
    let row = load(state, id)?;
    let source = source_of(&row).ok_or_else(|| AppError::new(ErrorCode::Internal))?;

    // **Room is asked before anything is made** — the plan's own answer, which carries the
    // person's choices. Only at the very start: past it the build asks again for itself, and
    // credits what is already on the server.
    if row.stage == VideoStage::Planned {
        if let Some(basis) = basis_of(&row) {
            let plan = effective_plan(&row, &basis, &source);
            if let SpaceCheck::Short {
                needed_bytes,
                free_bytes,
                short_by,
            } = plan.server_space
            {
                return Err(AppError::new(ErrorCode::RemoteDiskFull)
                    .with_detail(
                        Detail::new(DetailCode::LadderNotEnoughSpace)
                            .with("short_by", short_by)
                            .with("needed", needed_bytes)
                            .with("free", free_bytes)
                            .with("rungs", plan.rungs.len() as u64),
                    )
                    .with_cause(format!("short_by={short_by}")));
            }
            if let SpaceCheck::Short {
                needed_bytes,
                free_bytes,
                short_by,
            } = plan.local_space
            {
                return Err(AppError::new(ErrorCode::LocalDiskFull)
                    .with_detail(
                        Detail::new(DetailCode::LadderNoRoomHere)
                            .with("short_by", short_by)
                            .with("needed", needed_bytes)
                            .with("free", free_bytes)
                            .with("at", row.source_path.clone()),
                    )
                    .with_cause(format!("short_by={short_by}")));
            }
        }
    }

    // **The medium in the library, made before the first byte.** A taken name is a problem
    // with a choice — take that medium over, or another name — and never a quiet overwrite.
    let row = if row.media_id.is_none() {
        let media_id =
            super::library::api::media_create(state, &row.server_id, &row.title, Some(&row.slug))
                .await?;
        change(state, id, |r| {
            r.media_id = Some(media_id.clone());
            r.own_medium = true;
            Ok(r.clone())
        })?
    } else {
        row
    };

    // A medium made by this video a moment ago cannot be being watched: the viewer guard
    // (T571) counts every connection to the server, not to this medium, and would stop the
    // build for somebody watching something else.
    let past_viewers = row.own_medium || row.confirmed;
    let custom = custom_rungs(&row);
    let measure_first = custom.is_none()
        && !row.measured
        && (basis_of(&row).is_some_and(|b| b.needs_measuring)
            || row.stage == VideoStage::Measuring);

    if measure_first {
        let task = super::quality::api::quality_measure_start(
            state,
            super::quality::MeasureRequest {
                path: row.source_path.clone(),
                codec: String::from("h264"),
                native_height: None,
                prefer_hardware: true,
                then_build: Some(super::quality::ThenBuild {
                    server_id: row.server_id.clone(),
                    slug: row.slug.clone(),
                    audio_track: row.audio_track,
                    confirmed: past_viewers,
                    accept_objections: row.confirmed,
                }),
                batch: Some(batch_of(&row)),
            },
        )
        .await?;
        return Ok((task, VideoStage::Measuring));
    }

    let rungs = match custom {
        Some(rungs) => rungs,
        None => {
            let preview = super::ladder::api::ladder_plan_until(
                state,
                &ladder_request(&row.source_path),
                None,
            )
            .await?;
            if preview.from == super::ladder::LadderSource::Formula {
                return Err(AppError::new(ErrorCode::LadderNotMeasured));
            }
            if !row.confirmed
                && !crate::domain::ladder::may_build_unasked(&preview.verdict.objections)
            {
                return Err(AppError::new(ErrorCode::LadderObjection)
                    .with_details(preview.verdict.objections.iter().map(|o| o.detail()))
                    .detail(DetailCode::ChainStoppedByObjection));
            }
            preview.plan.rungs
        }
    };
    let task = super::ladder::api::ladder_build(
        state,
        super::ladder::BuildRequest {
            server_id: row.server_id.clone(),
            path: row.source_path.clone(),
            slug: row.slug.clone(),
            rungs,
            audio_track: row.audio_track,
            prefer_hardware: true,
            batch: Some(batch_of(&row)),
            confirmed: past_viewers,
        },
    )
    .await?;
    Ok((task, VideoStage::Encoding))
}

fn start_going(state: &AppState, id: &str) {
    let state = state.clone();
    let id = id.to_owned();
    spawn(async move { carry_on(&state, &id).await });
}

pub mod api {
    use super::*;

    /// Add files to be shown as videos with a plan. Each file on its own: one that is not a
    /// film is refused with its reason, and the rest are added (as T665).
    pub async fn video_add(
        state: &AppState,
        server_id: &str,
        paths: &[String],
    ) -> Result<VideoAdded> {
        ensure_watching(state);
        // The server has to exist; a list of videos for nowhere helps nobody.
        super::super::library::api::profile_of(state, server_id)?;

        let existing = rows::list(&state.db).map_err(storage)?;
        let mut added = Vec::new();
        let mut refused = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for path in paths {
            // A file already on its way here is not added a second time: two videos building
            // one set would write the same directory. One finished or stopped may be.
            let listed = existing.iter().any(|v| {
                v.server_id == server_id && v.source_path == *path && v.state.is_unfinished()
            });
            if listed || !seen.insert(path.clone()) {
                refused.push(VideoRefusal {
                    path: path.clone(),
                    error: AppError::new(ErrorCode::InvalidInput)
                        .detail(DetailCode::VideoAlreadyListed)
                        .with_cause(path),
                });
                continue;
            }
            let source = match super::super::api::source_probe(path).await {
                Ok(source) => source,
                Err(error) => {
                    refused.push(VideoRefusal {
                        path: path.clone(),
                        error,
                    });
                    continue;
                }
            };
            if source.audio_tracks.is_empty() {
                refused.push(VideoRefusal {
                    path: path.clone(),
                    error: AppError::new(ErrorCode::NoAudioTracks).with_cause(path),
                });
                continue;
            }
            let id = format!("v_{}", uuid::Uuid::new_v4().simple());
            let title = video::title_of(path);
            let slug = video::slug_for(&title, &id);
            let mut row = VideoRow::new(&id, server_id, path, &title, &slug);
            row.audio_track = video::default_audio(&source);
            row.source_json = serde_json::to_string(&source).ok();
            rows::save(&state.db, &row).map_err(storage)?;
            begin_planning(state, &id);
            added.push(view_of(state, &row));
            emit(state, &id);
        }
        Ok(VideoAdded { added, refused })
    }

    /// Every video, in the order they were added.
    pub fn video_list(state: &AppState) -> Result<Vec<VideoView>> {
        ensure_watching(state);
        Ok(rows::list(&state.db)
            .map_err(storage)?
            .iter()
            .map(|r| view_of(state, r))
            .collect())
    }

    /// One video.
    pub fn video_get(state: &AppState, id: &str) -> Result<VideoView> {
        Ok(view_of(state, &load(state, id)?))
    }

    /// Choose the audio track. Before anything is encoded only.
    pub fn video_set_audio(state: &AppState, id: &str, track: usize) -> Result<VideoView> {
        change(state, id, |row| {
            if !video::allowed(Act::SetAudio, row.state, row.stage, row.media_id.is_some()) {
                return Err(not_now(row));
            }
            let available = source_of(row).map(|s| s.audio_tracks.len()).unwrap_or(0);
            if track >= available {
                return Err(AppError::new(ErrorCode::InvalidInput).with_detail(
                    Detail::new(DetailCode::PlanNoSuchTrack)
                        .with("number", track + 1)
                        .with("available", available),
                ));
            }
            row.audio_track = track;
            Ok(())
        })?;
        video_get(state, id)
    }

    /// Change the title and the short name. Before the medium is in the library only.
    pub fn video_set_name(
        state: &AppState,
        id: &str,
        title: &str,
        slug: Option<&str>,
    ) -> Result<VideoView> {
        let title = title.trim();
        if title.is_empty() {
            return Err(AppError::new(ErrorCode::InvalidInput).detail(DetailCode::MediaTitleEmpty));
        }
        let slug = match slug.map(str::trim).filter(|s| !s.is_empty()) {
            Some(s) => s.to_owned(),
            None => crate::domain::media::slugify(title).ok_or_else(|| {
                AppError::new(ErrorCode::InvalidInput).detail(DetailCode::SlugUnmakeable)
            })?,
        };
        crate::domain::media::validate_slug(&slug)
            .map_err(|e| AppError::new(ErrorCode::InvalidInput).with_detail(e.detail()))?;
        change(state, id, |row| {
            if !video::allowed(Act::SetName, row.state, row.stage, row.media_id.is_some()) {
                return Err(not_now(row));
            }
            row.title = title.to_owned();
            if row.slug != slug {
                row.slug = slug.clone();
                if let Some(mut basis) = basis_of(row) {
                    // Not known for the new name until the library is read again.
                    basis.name_taken = None;
                    row.plan_json = serde_json::to_string(&basis).ok();
                }
            }
            Ok(())
        })?;
        video_get(state, id)
    }

    /// Set the rungs by hand (`Some`), or go back to the planned ones (`None`).
    pub fn video_set_rungs(
        state: &AppState,
        id: &str,
        rungs: Option<Vec<Rung>>,
    ) -> Result<VideoView> {
        if let Some(rungs) = &rungs {
            crate::domain::ladder::buildable(rungs).map_err(|why| match why {
                crate::domain::ladder::NotBuildable::NoRungs => {
                    AppError::new(ErrorCode::InvalidInput)
                }
                crate::domain::ladder::NotBuildable::RungsNotMeasured { indexes } => {
                    AppError::new(ErrorCode::LadderNotMeasured).with_cause(format!(
                        "rungs {}",
                        indexes
                            .iter()
                            .map(|i| (i + 1).to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                }
            })?;
        }
        change(state, id, |row| {
            if !video::allowed(Act::SetRungs, row.state, row.stage, row.media_id.is_some()) {
                return Err(not_now(row));
            }
            row.rungs_json = rungs.as_ref().and_then(|r| serde_json::to_string(r).ok());
            Ok(())
        })?;
        video_get(state, id)
    }

    /// «Start» — each video on its own. One still being planned starts the moment its plan is
    /// ready.
    pub fn video_start(state: &AppState, ids: &[String]) -> Vec<VideoStarted> {
        ensure_watching(state);
        ids.iter()
            .map(|id| {
                let outcome = change(state, id, |row| {
                    if !video::allowed(Act::Start, row.state, row.stage, row.media_id.is_some()) {
                        return Err(not_now(row));
                    }
                    row.start_requested = true;
                    if row.state == VideoState::Ready {
                        row.state = VideoState::Working;
                        Ok(true)
                    } else {
                        Ok(false)
                    }
                });
                match outcome {
                    Ok(go) => {
                        if go {
                            start_going(state, id);
                        }
                        VideoStarted {
                            id: id.clone(),
                            error: None,
                        }
                    }
                    Err(error) => VideoStarted {
                        id: id.clone(),
                        error: Some(error),
                    },
                }
            })
            .collect()
    }

    /// Pause — the task of the current stage. Stays a pause across a restart.
    pub fn video_pause(state: &AppState, id: &str) -> Result<VideoView> {
        change(state, id, |row| {
            if !video::allowed(Act::Pause, row.state, row.stage, row.media_id.is_some()) {
                return Err(not_now(row));
            }
            row.state = VideoState::Paused;
            row.paused_by_person = true;
            Ok(())
        })?;
        // A queued task cannot be paused: the watcher pauses it the moment it starts.
        for task in alive(state, id) {
            let _ = state.tasks.pause(&task);
        }
        video_get(state, id)
    }

    /// Continue after a pause — the paused task, or, with nothing left alive (a restart), the
    /// stage from where it was.
    pub fn video_resume(state: &AppState, id: &str) -> Result<VideoView> {
        change(state, id, |row| {
            if !video::allowed(Act::Resume, row.state, row.stage, row.media_id.is_some()) {
                return Err(not_now(row));
            }
            row.state = VideoState::Working;
            row.paused_by_person = false;
            Ok(())
        })?;
        let living = alive(state, id);
        if living.is_empty() {
            if !lock(&state.videos.inner.starting).contains(id) {
                start_going(state, id);
            }
        } else {
            for task in living {
                let _ = state.tasks.resume(&task);
            }
        }
        video_get(state, id)
    }

    /// Stop. The tasks are cancelled; the video is `cancelling` until they have really
    /// stopped (the server's cutting included), then `cancelled`. «Retry» carries on.
    pub fn video_cancel(state: &AppState, id: &str) -> Result<VideoView> {
        if let Some(planning) = lock(&state.videos.inner.planning).remove(id) {
            planning.cancel();
        }
        let living = alive(state, id);
        let busy = !living.is_empty() || lock(&state.videos.inner.starting).contains(id);
        change(state, id, |row| {
            if !video::allowed(Act::Cancel, row.state, row.stage, row.media_id.is_some()) {
                return Err(not_now(row));
            }
            row.state = if busy {
                VideoState::Cancelling
            } else {
                VideoState::Cancelled
            };
            row.paused_by_person = false;
            row.start_requested = false;
            Ok(())
        })?;
        let _ = state.tasks.cancel_batch(id);
        if !busy {
            lock(&state.videos.inner.live).remove(id);
        }
        video_get(state, id)
    }

    /// Carry on from the stage where it stopped. `confirmed` is «build anyway»: past viewers
    /// on the server and past the checker's objections, once a person has seen them.
    pub fn video_retry(state: &AppState, id: &str, confirmed: bool) -> Result<VideoView> {
        ensure_watching(state);
        let replan = change(state, id, |row| {
            if !video::allowed(Act::Retry, row.state, row.stage, row.media_id.is_some()) {
                return Err(not_now(row));
            }
            if confirmed {
                row.confirmed = true;
            }
            let problem: Option<VideoProblem> = row
                .problem_json
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok());
            row.problem_json = None;
            row.paused_by_person = false;
            let space_was_short = problem.as_ref().is_some_and(|p| {
                matches!(
                    p.error.code,
                    ErrorCode::RemoteDiskFull | ErrorCode::LocalDiskFull
                )
            });
            if row.stage == VideoStage::Planned && (row.plan_json.is_none() || space_was_short) {
                row.state = VideoState::Planning;
                Ok(true)
            } else {
                row.state = VideoState::Working;
                row.start_requested = true;
                Ok(false)
            }
        })?;
        if replan {
            begin_planning(state, id);
        } else {
            start_going(state, id);
        }
        video_get(state, id)
    }

    /// The name is taken: build into the medium that has it (its other files stay).
    pub async fn video_replace(state: &AppState, id: &str) -> Result<VideoView> {
        let row = load(state, id)?;
        if !video::allowed(Act::Replace, row.state, row.stage, row.media_id.is_some()) {
            return Err(not_now(&row));
        }
        let profile = super::super::library::api::profile_of(state, &row.server_id)?;
        let conn = crate::server::gate::open(
            state.secrets.as_ref(),
            &profile,
            crate::server::gate::Intent::Read,
        )
        .await?
        .conn;
        let manifest = crate::server::manifest_io::read(&conn, &profile.video_dir).await;
        conn.close().await;
        let media_id = manifest?.find_by_slug(&row.slug).map(|m| m.id.clone());
        change(state, id, |r| {
            if !video::allowed(Act::Replace, r.state, r.stage, r.media_id.is_some()) {
                return Err(not_now(r));
            }
            r.media_id = media_id.clone();
            r.own_medium = false;
            r.problem_json = None;
            r.state = VideoState::Working;
            r.start_requested = true;
            Ok(())
        })?;
        start_going(state, id);
        video_get(state, id)
    }

    /// Take a video off the list. Nothing on the server is touched (T577, part b). Not while it is
    /// going: stop it first.
    pub fn video_remove(state: &AppState, id: &str) -> Result<()> {
        let _held = lock(&state.videos.inner.rows);
        let row = load(state, id)?;
        if !video::allowed(Act::Remove, row.state, row.stage, row.media_id.is_some()) {
            return Err(not_now(&row));
        }
        if let Some(planning) = lock(&state.videos.inner.planning).remove(id) {
            planning.cancel();
        }
        rows::remove(&state.db, id).map_err(storage)?;
        lock(&state.videos.inner.live).remove(id);
        Ok(())
    }

    /// Carry on after the application starts (T672, the owner's decision of 2026-10-01).
    ///
    /// Every video that was going starts its stage again: the measurement takes only the
    /// points it does not have yet, the build finds every rung already on the server. A plan
    /// that was being made is made again. One a person paused stays paused; one waiting for
    /// «Start» or for a choice about a problem waits. Returns how many were carried on.
    pub fn restore_videos(state: &AppState) -> Result<usize> {
        ensure_watching(state);
        let mut carried = 0;
        for row in rows::list(&state.db).map_err(storage)? {
            match video::after_restart(row.state) {
                video::AfterRestart::PlanAgain => begin_planning(state, &row.id),
                video::AfterRestart::CarryOn => {
                    carried += 1;
                    start_going(state, &row.id);
                }
                video::AfterRestart::NowCancelled => {
                    for task in unfinished(state, &row.id) {
                        if !state.tasks.is_alive(&task) {
                            let _ = state.tasks.cancel(&task);
                        }
                    }
                    let _ = change(state, &row.id, |r| {
                        r.state = VideoState::Cancelled;
                        r.task_id = None;
                        Ok(())
                    });
                }
                video::AfterRestart::Leave => {}
            }
        }
        Ok(carried)
    }
}

pub mod ipc {
    use super::*;
    use tauri::State;

    #[tauri::command]
    pub async fn video_add(
        state: State<'_, AppState>,
        server_id: String,
        paths: Vec<String>,
    ) -> Result<VideoAdded> {
        api::video_add(&state, &server_id, &paths).await
    }

    #[tauri::command]
    pub fn video_list(state: State<'_, AppState>) -> Result<Vec<VideoView>> {
        api::video_list(&state)
    }

    #[tauri::command]
    pub fn video_set_audio(
        state: State<'_, AppState>,
        id: String,
        track: usize,
    ) -> Result<VideoView> {
        api::video_set_audio(&state, &id, track)
    }

    #[tauri::command]
    pub fn video_set_name(
        state: State<'_, AppState>,
        id: String,
        title: String,
        slug: Option<String>,
    ) -> Result<VideoView> {
        api::video_set_name(&state, &id, &title, slug.as_deref())
    }

    #[tauri::command]
    pub fn video_set_rungs(
        state: State<'_, AppState>,
        id: String,
        rungs: Option<Vec<Rung>>,
    ) -> Result<VideoView> {
        api::video_set_rungs(&state, &id, rungs)
    }

    #[tauri::command]
    pub fn video_start(state: State<'_, AppState>, ids: Vec<String>) -> Vec<VideoStarted> {
        api::video_start(&state, &ids)
    }

    #[tauri::command]
    pub fn video_pause(state: State<'_, AppState>, id: String) -> Result<VideoView> {
        api::video_pause(&state, &id)
    }

    #[tauri::command]
    pub fn video_resume(state: State<'_, AppState>, id: String) -> Result<VideoView> {
        api::video_resume(&state, &id)
    }

    #[tauri::command]
    pub fn video_cancel(state: State<'_, AppState>, id: String) -> Result<VideoView> {
        api::video_cancel(&state, &id)
    }

    #[tauri::command]
    pub fn video_retry(
        state: State<'_, AppState>,
        id: String,
        confirmed: bool,
    ) -> Result<VideoView> {
        api::video_retry(&state, &id, confirmed)
    }

    #[tauri::command]
    pub async fn video_replace(state: State<'_, AppState>, id: String) -> Result<VideoView> {
        api::video_replace(&state, &id).await
    }

    #[tauri::command]
    pub fn video_remove(state: State<'_, AppState>, id: String) -> Result<()> {
        api::video_remove(&state, &id)
    }
}
