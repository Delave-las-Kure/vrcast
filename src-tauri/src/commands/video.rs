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
    /// What the set will take on the server once checked, in bytes: the segments alone
    /// (T693 — the prepared files are removed then). `server_space` asks for more: while the
    /// set is built the prepared files are there too.
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
    /// T695 — whether the sound is chosen. `false` for a film with more than one track
    /// until a person picks one: «Start» is refused meanwhile, and `audio_track` is only what
    /// the plan's sizes are reckoned with.
    pub audio_chosen: bool,
    /// The subtitle track drawn into every rung (T696, owner's decision B3); `null` — none,
    /// the default. The tracks themselves are in `source.subtitle_tracks`.
    pub subtitle_track: Option<usize>,
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
    /// T694 — a link per quality of the set, the heaviest first; empty until `done`.
    pub quality_links: Vec<crate::domain::links::QualityLink>,
    pub created_at: String,
    pub updated_at: String,
    /// The version of this view (T687): higher is newer, for every change — the progress
    /// of the stage included, which `updated_at` does not follow. A screen keeps the view with
    /// the higher one, whichever arrived first.
    pub rev: u64,
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

impl VideoHub {
    /// The next version of a view of a video (T687): it grows with every view this run
    /// makes — a change of progress included, which writes nothing to the row — and starts
    /// from the clock, in microseconds, so that a run started later is above every view an
    /// earlier one gave out. Well inside what a JavaScript number holds exactly.
    pub(crate) fn next_rev(&self) -> u64 {
        let base = *self.inner.rev_base.get_or_init(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_micros() as u64)
                .unwrap_or(0)
        });
        base + self.inner.rev.fetch_add(1, Ordering::SeqCst) + 1
    }
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
    /// What each video last said about its medium's set (T677), so the library is told only
    /// when that changes, not on every tick of a bar.
    set_work: Mutex<HashMap<String, (String, String, video::SetWorkState)>>,
    /// Servers being removed (T683): nothing of theirs is put on the queue meanwhile.
    closing: Mutex<HashSet<String>>,
    /// Videos whose stop on the server is being confirmed after a restart (T682): a cutting
    /// a run that is over started and left `cancelling`.
    stopping: Mutex<HashSet<String>>,
    /// The versions of the views (T687): see [`VideoHub::next_rev`].
    rev_base: std::sync::OnceLock<u64>,
    rev: std::sync::atomic::AtomicU64,
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

/// T695 — nothing goes ahead on a sound nobody chose: a film with more than one track
/// waits for the person to pick one, and is refused, changed in nothing, meanwhile.
fn audio_not_chosen(row: &VideoRow) -> Result<()> {
    if row.audio_chosen {
        return Ok(());
    }
    Err(AppError::new(ErrorCode::InvalidInput)
        .detail(DetailCode::AudioNotChosen)
        .with_cause(&row.id))
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
    // The version is taken before the row is read (T687): a view with a higher one has seen
    // every change written before a lower one was taken.
    let rev = state.videos.next_rev();
    if let Ok(row) = load(state, id) {
        // «Remove» on a video whose work was alive (T683): it goes off the list the moment
        // the work has stopped, whichever way the news of that arrived.
        if row.remove_requested && row.state == VideoState::Cancelled {
            take_off_list(state, id);
            return;
        }
        tell_library(state, id, Some(&row));
        let _ = state
            .events
            .send(super::AppEvent::VideoUpdate(Box::new(view_at(
                state, &row, rev,
            ))));
    }
}

/// Send every video as it is now (T687): after the stream of events fell behind, a change
/// may never have reached the screen — a build that ended, a stop confirmed.
pub(crate) fn republish(state: &AppState) {
    let Ok(all) = rows::list(&state.db) else {
        return;
    };
    for row in all {
        emit(state, &row.id);
    }
}

/// Take a video off the list now — the row, what this run holds about it — and say so.
/// Nothing on the server is touched (T577, part b), but for an empty medium the video made
/// itself (T700, `drop_medium_left_empty`). Only ever called once nothing of the video's work
/// is alive.
fn take_off_list(state: &AppState, id: &str) {
    let left = {
        let _held = lock(&state.videos.inner.rows);
        if let Some(planning) = lock(&state.videos.inner.planning).remove(id) {
            planning.cancel();
        }
        let row = load(state, id).ok();
        let _ = rows::remove(&state.db, id);
        row
    };
    if let Some(row) = left {
        drop_medium_left_empty(state, &row);
    }
    forget_video(state, id);
}

/// A video removed before it was done leaves no empty medium of its own making behind
/// (T700): the one it made at «Start» goes too — only if it made it, and only if nothing of
/// it is on the server (`media_drop_if_empty`). Best effort, in the background: a server that
/// does not answer leaves the medium as it is, for a person to delete in the library.
fn drop_medium_left_empty(state: &AppState, row: &VideoRow) {
    let (true, Some(media_id)) = (row.made_medium, row.media_id.clone()) else {
        return;
    };
    if row.state == VideoState::Done {
        return;
    }
    let state = state.clone();
    let server_id = row.server_id.clone();
    let vid = row.id.clone();
    spawn(async move {
        match super::library::api::media_drop_if_empty(&state, &server_id, &media_id).await {
            Ok(_) => {}
            Err(e) => {
                tracing::info!(video = %vid, media = %media_id, error = %e, "the empty medium of a removed video was left as it is");
            }
        }
    });
}

/// What this run holds about a video that is gone from the list, and the news of it.
fn forget_video(state: &AppState, id: &str) {
    lock(&state.videos.inner.live).remove(id);
    tell_library(state, id, None);
    let _ = state
        .events
        .send(super::AppEvent::VideoRemoved { id: id.to_owned() });
}

// ---------- a server going away (T683) ----------

/// The work alive in this run on a server: its own tasks, and every task of its videos (a
/// measurement names no server of its own, only the video's batch), plus a video whose next
/// task is being put on the queue right now. What removing the server would have to stop.
pub(crate) fn work_on_server(state: &AppState, server_id: &str) -> Vec<String> {
    let videos: HashSet<String> = rows::list(&state.db)
        .unwrap_or_default()
        .into_iter()
        .filter(|v| v.server_id == server_id)
        .map(|v| v.id)
        .collect();
    let mut out: Vec<String> = state
        .tasks
        .list()
        .unwrap_or_default()
        .into_iter()
        .filter(|t| !t.state.is_final() && state.tasks.is_alive(&t.id))
        .filter(|t| {
            t.server_id.as_deref() == Some(server_id)
                || t.batch.as_ref().is_some_and(|b| videos.contains(&b.id))
        })
        .map(|t| t.id)
        .collect();
    let starting = lock(&state.videos.inner.starting);
    out.extend(
        videos
            .iter()
            .filter(|v| starting.contains(*v))
            .map(|v| format!("video:{v}")),
    );
    let stopping = lock(&state.videos.inner.stopping);
    out.extend(
        videos
            .iter()
            .filter(|v| stopping.contains(*v))
            .map(|v| format!("stop:{v}")),
    );
    out
}

/// Stop everything on a server that is being removed (T683): its videos' plans, their tasks
/// (each video `cancelling` until they have stopped, as after «Cancel»), and its own tasks.
/// Safe to repeat: a cancellation already under way is left to finish.
pub(crate) fn stop_server_work(state: &AppState, server_id: &str) {
    ensure_watching(state);
    let videos: Vec<VideoRow> = rows::list(&state.db)
        .unwrap_or_default()
        .into_iter()
        .filter(|v| v.server_id == server_id)
        .collect();
    for v in &videos {
        if let Some(planning) = lock(&state.videos.inner.planning).remove(&v.id) {
            planning.cancel();
        }
        let busy =
            !alive(state, &v.id).is_empty() || lock(&state.videos.inner.starting).contains(&v.id);
        if busy {
            let _ = change(state, &v.id, |r| {
                r.state = VideoState::Cancelling;
                r.paused_by_person = false;
                r.start_requested = false;
                Ok(())
            });
        }
        cancel_tasks_of(state, &v.id);
    }
    for task in work_on_server(state, server_id) {
        let _ = state.tasks.cancel(&task);
    }
}

/// The server's videos are gone with it (its rows cascade): what this run holds about them
/// goes too, and the screen is told (T683).
pub(crate) fn forget_server_videos(state: &AppState, ids: &[String]) {
    for id in ids {
        if let Some(planning) = lock(&state.videos.inner.planning).remove(id) {
            planning.cancel();
        }
        forget_video(state, id);
    }
}

/// The videos of a server, by id.
pub(crate) fn videos_of_server(state: &AppState, server_id: &str) -> Vec<String> {
    rows::list(&state.db)
        .unwrap_or_default()
        .into_iter()
        .filter(|v| v.server_id == server_id)
        .map(|v| v.id)
        .collect()
}

/// While held, nothing of this server's videos is put on the queue (T683): «Start», «Retry»,
/// «Continue» pressed while the server is being removed do not start what is being stopped.
pub(crate) struct Closing {
    state: AppState,
    server_id: String,
}

pub(crate) fn close_server(state: &AppState, server_id: &str) -> Closing {
    lock(&state.videos.inner.closing).insert(server_id.to_owned());
    Closing {
        state: state.clone(),
        server_id: server_id.to_owned(),
    }
}

impl Drop for Closing {
    fn drop(&mut self) {
        lock(&self.state.videos.inner.closing).remove(&self.server_id);
    }
}

/// Tell the library when what a video says about its medium's set changes (T677) — it shows
/// «building» or «stopped» beside the medium (`library::api::with_set_work`). `None` for a
/// video taken off the list.
fn tell_library(state: &AppState, id: &str, row: Option<&VideoRow>) {
    let now = row.and_then(|r| {
        let (media, work) = set_work_of_row(r)?;
        Some((r.server_id.clone(), media, work))
    });
    let was = {
        let mut known = lock(&state.videos.inner.set_work);
        let was = known.get(id).cloned();
        if was == now {
            return;
        }
        match &now {
            Some(n) => known.insert(id.to_owned(), n.clone()),
            None => known.remove(id),
        };
        was
    };
    for (server, _, _) in [was, now].into_iter().flatten() {
        state.notify_library_changed(&server);
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

fn problem_of(row: &VideoRow) -> Option<VideoProblem> {
    row.problem_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
}

/// The confirmed «Replace» this video is carrying out, if any (T686).
fn replacing_of(row: &VideoRow) -> Option<video::Replacing> {
    row.replacing_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
}

/// Which medium's set this video says something about, and what (T677) — `building` from
/// the moment a «Replace» is confirmed until it is on its way (T686), whatever the video's
/// own state reads meanwhile. Shared with the library (`library::api::with_set_work`).
pub(crate) fn set_work_of_row(row: &VideoRow) -> Option<(String, video::SetWorkState)> {
    if let Some(replacing) = replacing_of(row) {
        if let Some(media) = replacing.media_id.or_else(|| row.media_id.clone()) {
            return Some((media, video::SetWorkState::Building));
        }
    }
    let media = row.media_id.clone()?;
    let work = video::set_work_of(row.state, row.stage, row.start_requested)?;
    Some((media, work))
}

/// A video for a medium, added stopped on a set of the medium's name that nobody claims
/// (T677): it waits for «Replace», and nothing — its plan finishing included — moves it on.
fn held_for_replace(row: &VideoRow) -> bool {
    row.state == VideoState::Problem
        && row.stage == VideoStage::Planned
        && problem_of(row).is_some_and(|p| video::is_old_set_problem(&p.error))
}

/// Whether «Replace» may be pressed on this video: as `video::may_replace` says, or — T700 —
/// on the plan itself when it already says the name is taken. «Start» is not offered there
/// (it would only stop on SLUG_TAKEN); «Another name» and «Replace» are.
fn may_replace_row(row: &VideoRow) -> bool {
    video::may_replace(
        row.state,
        row.stage,
        row.media_id.is_some(),
        problem_of(row).as_ref().map(|p| &p.error),
    ) || (row.state == VideoState::Ready
        && row.stage == VideoStage::Planned
        && row.media_id.is_none()
        && basis_of(row).and_then(|b| b.name_taken) == Some(true))
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

/// The person's rungs with what this film already has measured for them (T680): the
/// film's own measurement (measured here, or lent), and the points measured for rungs edited
/// before (`quality::edited_codec`). Only a point at exactly the rung's bitrate and height
/// counts (`ladder::with_measured`); nothing is marked measured otherwise.
fn with_measured_here(state: &AppState, path: &str, rungs: &[Rung]) -> Vec<Rung> {
    use crate::store::measurements;
    let Ok(key) = measurements::key_for(std::path::Path::new(path)) else {
        return rungs.to_vec();
    };
    let codec = "h264";
    let mut out = rungs.to_vec();
    if let Ok(Some(run)) = measurements::run(&state.db, &key, codec) {
        if !run.check_pending {
            let points = measurements::points(&state.db, &key, codec).unwrap_or_default();
            out = crate::domain::ladder::with_measured(&out, &points, run.borrowed_from.is_some());
        }
    }
    let edited = super::quality::edited_codec(codec);
    let points = measurements::points(&state.db, &key, &edited).unwrap_or_default();
    crate::domain::ladder::with_measured(&out, &points, false)
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
    use crate::domain::ladder_size::bytes_for_set;

    let custom = custom_rungs(row);
    let edited = custom.is_some();
    let rungs = custom.unwrap_or_else(|| basis.rungs.clone());
    // What of the person's rungs is still to be measured before the build (T680): «Start»
    // measures those points first. Preliminary: a point's time on this machine's model.
    let edited_cells = if edited {
        crate::domain::ladder::cells_to_measure(&rungs).len()
    } else {
        0
    };
    let edited_measure_s = (edited_cells as f64
        * crate::domain::measure_grid::seconds_per_point(
            source.width,
            source.height,
            source.fps,
            crate::domain::chunks::CHUNK_S as u64,
            3,
        ))
    .round() as u64;
    // The sound the prepared files will carry — the output's, not the source's (T699).
    let audio_bps = crate::domain::ladder_size::audio_out_bps(source, row.audio_track);
    let bitrates: Vec<u64> = rungs.iter().map(|r| r.bitrate_bps).collect();
    // What the set leaves on the server once checked — the segments alone (T693) — and what
    // the build needs there while it runs: the prepared files too, until the check.
    let server_bytes =
        crate::domain::ladder_size::served_bytes_for_set(&bitrates, audio_bps, source.duration_s);
    let server_peak = bytes_for_set(&bitrates, audio_bps, source.duration_s);
    // What this computer holds at its fullest: one prepared file, the heaviest (T699). The
    // segments are cut on the server.
    let ceilings: Vec<u64> = rungs
        .iter()
        .map(|r| r.maxrate_bps.max(r.bitrate_bps))
        .collect();
    let local_bytes =
        crate::domain::ladder_size::local_peak_bytes(&ceilings, audio_bps, source.duration_s);
    let objections = crate::domain::ladder::validate(&rungs, &facts_of(source), source.fps)
        .iter()
        .map(|o| o.detail())
        .collect();
    VideoPlan {
        // T696: subtitles drawn in make every rung an encode, the top one included.
        encode_s: video::encode_seconds_with(
            &rungs,
            source,
            basis.pixels_per_s,
            row.subtitle_track.is_some(),
        ),
        from: if edited {
            PlanSource::Edited
        } else {
            basis.from
        },
        needs_measuring: if edited {
            edited_cells > 0
        } else {
            basis.needs_measuring && !row.measured
        },
        measure_s: if edited {
            edited_measure_s
        } else if basis.needs_measuring && !row.measured {
            basis.measure_s
        } else {
            0
        },
        encode_estimate: basis.encode_estimate,
        encoder: basis.encoder.clone(),
        server_space: space(basis.server_disk, server_peak),
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

/// T694 — a link per quality of the finished set, the heaviest first: each rung's own
/// playlist (`{slug}/v9/stream.m3u8`), named the way the build names its directory. Empty
/// until the video is done.
fn quality_links_of(state: &AppState, row: &VideoRow) -> Vec<crate::domain::links::QualityLink> {
    if row.stage != VideoStage::Done {
        return Vec::new();
    }
    let Some(profile) = crate::store::profiles::get(&state.db, &row.server_id)
        .ok()
        .flatten()
    else {
        return Vec::new();
    };
    let Some(rungs) = custom_rungs(row).or_else(|| basis_of(row).map(|b| b.rungs)) else {
        return Vec::new();
    };
    let mut out: Vec<crate::domain::links::QualityLink> = rungs
        .iter()
        .map(|rung| {
            let sub = crate::domain::ladder_build::sub_name(rung);
            let links = crate::domain::links::for_path(
                &profile.domain,
                profile.cdn_base.as_deref(),
                &format!("{}/{sub}/stream.m3u8", row.slug),
            );
            crate::domain::links::QualityLink {
                width: rung.width,
                height: rung.height,
                bitrate_bps: (rung.bitrate_bps / 1_000_000).max(1) * 1_000_000,
                origin: links.origin,
                cdn: links.cdn,
            }
        })
        .collect();
    out.sort_by_key(|q| std::cmp::Reverse(q.bitrate_bps));
    out.dedup_by(|a, b| a.origin == b.origin);
    out
}

/// A view of a video at a version taken now — after the row was read. Fine where nothing
/// else can be changing the video meanwhile (it was just written under the rows lock, and
/// its event follows with a higher one); a reader racing the watcher takes its version
/// before reading, with [`view_at`].
fn view_of(state: &AppState, row: &VideoRow) -> VideoView {
    view_at(state, row, state.videos.next_rev())
}

/// A view of a video, at the version `rev` taken **before** `row` was read (T687).
fn view_at(state: &AppState, row: &VideoRow, rev: u64) -> VideoView {
    let source = source_of(row);
    let plan = match (basis_of(row), source.as_ref()) {
        (Some(basis), Some(source)) => Some(effective_plan(row, &basis, source)),
        _ => None,
    };
    let progress = if matches!(
        row.state,
        VideoState::Working | VideoState::Paused | VideoState::Cancelling | VideoState::Planning
    ) {
        // While planning, only the wait for a place for the plan's trial encodes (T688):
        // `task_state: queued`, nothing else.
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
        audio_chosen: row.audio_chosen,
        subtitle_track: row.subtitle_track,
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
        quality_links: quality_links_of(state, row),
        created_at: row.created_at.clone(),
        updated_at: row.updated_at.clone(),
        rev,
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

/// Stop every task of this video — except a row of a run that is over whose work on the
/// server is not confirmed stopped yet (T682): that one stays unfinished, and its set busy,
/// until [`stop_after_restart`] has the server's word.
fn cancel_tasks_of(state: &AppState, id: &str) {
    for task in unfinished(state, id) {
        let waiting_for_server = !state.tasks.is_alive(&task)
            && crate::store::remote_runs::get(&state.db, &task)
                .ok()
                .flatten()
                .is_some();
        if !waiting_for_server {
            let _ = state.tasks.cancel(&task);
        }
    }
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
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        // Task events were missed, so a video may have changed with nothing
                        // sent about it (T687): every one is sent again as it now is.
                        reconcile(&state);
                        republish(&state);
                    }
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
    // T700 — a new task took the video over, or the stage moved on: what the bar said was
    // about the stage before (a measurement's «100%» shown under «Encoding»).
    let mut fresh = false;
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
        let mut taken_over = false;
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
            taken_over = true;
            fresh = true;
            dirty = true;
        }
        if row.task_id.as_deref() != Some(task_id) {
            return;
        }
        if kind == TaskKind::MeasureQuality
            && code == Some(DetailCode::StageDone)
            && !row.measured
            && row.rungs_json.is_none()
        {
            row.measured = true;
            refresh = true;
            dirty = true;
        }
        if let Some(stage) = code.and_then(video::stage_of) {
            if stage != row.stage {
                row.stage = stage;
                dirty = true;
                fresh = true;
            }
        }
        // **A pause is the video's, wherever it was pressed** (T685, QA-25 №6). «Pause» in
        // «Tasks» pauses the task of the video's current stage, and that is the video paused
        // by a person — kept across a restart as such, with «Continue» on the card. «Continue»
        // in «Tasks» on a task that was paused is the video going again. Told apart from a
        // task only waiting for its place (`queued` from the start) by what the task was
        // last: a task carried on goes `paused` → `queued` → `running`. What was last is the
        // same task's only while it did not just take the place over (a build chained on).
        let was = lock(&state.videos.inner.live)
            .get(&vid)
            .map(|l| l.progress.task_state)
            .filter(|_| !taken_over);
        match (row.state, task_state) {
            (VideoState::Working, TaskState::Paused) => {
                row.state = VideoState::Paused;
                row.paused_by_person = true;
                dirty = true;
            }
            (VideoState::Paused, TaskState::Queued | TaskState::Running)
                if was == Some(TaskState::Paused) =>
            {
                row.state = VideoState::Working;
                row.paused_by_person = false;
                dirty = true;
            }
            _ => {}
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
        state, &vid, task_state, progress, code, speed_bps, eta_s, rungs, fresh,
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
    fresh: bool,
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
    if fresh {
        // Begun afresh: nothing of the stage before is carried over — not its share, not
        // its speed, not its time left. Which rung the build is at stays: it is still true.
        entry.code = None;
        entry.since = Instant::now();
        entry.from = 0.0;
        entry.progress.progress = 0.0;
        entry.progress.speed_bps = None;
        entry.progress.eta_s = None;
    }
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
    let progress = video::bar_of(code, progress);
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
                    if !row.measured && row.rungs_json.is_none() {
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
                    if measured && !row.measured && row.rungs_json.is_none() {
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
                    let filed = crate::tasks::store::get(&state.db, task_id)
                        .ok()
                        .flatten()
                        .and_then(|t| t.result);
                    row.paused_by_person = false;
                    match filed {
                        Some(result) => {
                            row.stage = VideoStage::Done;
                            row.state = VideoState::Done;
                            row.problem_json = None;
                            row.media_id = Some(result.media_id);
                        }
                        // **No medium to file the set under** (T684, QA-25 №5): it was deleted
                        // meanwhile — by another copy of the application; this one refuses
                        // while the video is on its way — or the catalogue could not be read.
                        // Not «Done» with a set the catalogue does not know: a problem, and
                        // «Retry» makes the medium again and files the set (its rungs are
                        // found done by the set's own record, T681).
                        None => {
                            set_problem(&mut row, AppError::new(ErrorCode::VideoMediumGone));
                            row.media_id = None;
                            row.own_medium = false;
                            row.made_medium = false;
                            row.task_id = None;
                        }
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
        // A stop on the server being confirmed after a restart (T682) is over when it says.
        if lock(&state.videos.inner.stopping).contains(&row.id) {
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
            // Added stopped on an old set (T677): the plan is kept for «Replace», and the
            // video stays on its problem until then.
            let held = held_for_replace(row);
            if row.state != VideoState::Planning && !held {
                return Ok(false);
            }
            match made {
                Ok(basis) => {
                    row.plan_json = serde_json::to_string(&basis).ok();
                    if held {
                        return Ok(false);
                    }
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
                    // The plan is made again after «Replace» and says so then.
                    if !held {
                        set_problem(row, e);
                    }
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

/// Say on the card whether its plan waits for a place for its trial encodes (T688): a
/// `queued` progress while it waits, none once it has the place.
fn plan_waiting(state: &AppState, id: &str, queued: bool) {
    let changed = {
        let mut live = lock(&state.videos.inner.live);
        if queued {
            live.insert(
                id.to_owned(),
                Live {
                    code: None,
                    since: Instant::now(),
                    from: 0.0,
                    progress: VideoProgress {
                        task_state: TaskState::Queued,
                        progress: 0.0,
                        speed_bps: None,
                        eta_s: None,
                        rung: None,
                        rungs: 0,
                    },
                },
            );
            true
        } else {
            match live.get(id) {
                Some(l) if l.code.is_none() && l.progress.task_state == TaskState::Queued => {
                    live.remove(id);
                    true
                }
                _ => false,
            }
        }
    };
    if changed {
        emit(state, id);
    }
}

async fn make_basis(state: &AppState, id: &str, cancel: &CancellationToken) -> Result<PlanBasis> {
    let row = load(state, id)?;
    let profile = super::library::api::profile_of(state, &row.server_id)?;

    // **The trial encodes wait for a place among the heavy work** (T688, QA-25 №9): with a
    // limit of one, ten videos added at once encode one plan's pieces at a time, and the card
    // says «in the queue» while it waits. «Remove» cancels the wait and the encodes.
    let waiting = {
        let state = state.clone();
        let id = id.to_owned();
        move |queued: bool| plan_waiting(&state, &id, queued)
    };
    let preview = super::ladder::api::ladder_plan_under_limit(
        state,
        &ladder_request(&row.source_path),
        cancel,
        &waiting,
    )
    .await;
    plan_waiting(state, id, false);
    let preview = preview?;
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
            Ok(row) if lock(&state.videos.inner.closing).contains(&row.server_id) => false,
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

/// How many times a video tries to make its medium when the catalogue keeps changing under it.
const MEDIUM_ATTEMPTS: u32 = 5;

/// Make this video's medium in the library.
///
/// **A catalogue changed meanwhile is read again, not a problem** (found on a real run,
/// 2026-10-02). «Start» on a selection starts every video at once, and each makes its medium
/// at the same moment: every one reads the catalogue at the same generation, the first write
/// wins, and the rest were stopped on `MANIFEST_CONFLICT` with «Retry» although nothing was
/// wrong. A conflict writes nothing (`manifest_io::write`), so trying again with the catalogue
/// read afresh is exactly what «Retry» would do — done here, a few times, before a person is
/// asked. A name that has become taken meanwhile is still `SLUG_TAKEN`, with its choice.
async fn make_medium(state: &AppState, row: &VideoRow) -> Result<String> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        match super::library::api::media_create(state, &row.server_id, &row.title, Some(&row.slug))
            .await
        {
            Err(e) if e.code == ErrorCode::ManifestConflict && attempt < MEDIUM_ATTEMPTS => {
                tracing::debug!(video = %row.id, attempt, "the catalogue changed while the medium was made; reading it again");
                tokio::time::sleep(Duration::from_millis(150 * u64::from(attempt))).await;
            }
            other => return other,
        }
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
        let media_id = make_medium(state, &row).await?;
        change(state, id, |r| {
            r.media_id = Some(media_id.clone());
            r.own_medium = true;
            r.made_medium = true;
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
                    subtitle_track: row.subtitle_track,
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
        // **The person's rungs: what is not measured yet is measured first** (T680, the
        // owner's decision of 2026-10-02). Only the points still missing — a point measured
        // before, on the grid or for an earlier edit, is taken as it is — and then on to
        // the build.
        Some(rungs) => {
            let rungs = with_measured_here(state, &row.source_path, &rungs);
            if custom_rungs(&row).as_ref() != Some(&rungs) {
                let kept = rungs.clone();
                change(state, id, |r| {
                    r.rungs_json = serde_json::to_string(&kept).ok();
                    Ok(())
                })?;
            }
            let cells = crate::domain::ladder::cells_to_measure(&rungs);
            if !cells.is_empty() {
                let task = super::quality::api::quality_measure_cells_start(
                    state,
                    super::quality::MeasureRequest {
                        path: row.source_path.clone(),
                        codec: String::from("h264"),
                        native_height: None,
                        prefer_hardware: true,
                        then_build: None,
                        batch: Some(batch_of(&row)),
                    },
                    cells,
                )
                .await?;
                return Ok((task, VideoStage::Measuring));
            }
            // The person's own rungs are built as they are, as before T680: what they chose
            // was theirs to choose, and the measurement only fills in the score.
            rungs
        }
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
            subtitle_track: row.subtitle_track,
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

// ---------- a stop left unconfirmed by a run that is over (T682) ----------

/// A video found `cancelling` at start-up: «Cancel» was pressed and the application went
/// away before every part of the work had stopped (T682, QA-25 №3).
///
/// **Local work died with the application** — a task of the previous run is a row and
/// nothing more (`ManagedProcess`'s sweep at start-up has ended any program it left). **Work
/// on the server did not:** a cutting runs there apart from us, and the start of it is written
/// down (`remote_runs`) until its end is confirmed. So:
/// - a task with no such note is closed at once, as before;
/// - a task with one stays unfinished — its set stays busy for the library and for
///   «Replace» (`running_build_for` sees it) — while the stop of **that start's mark** is
///   tried again, through a fresh connection to the server and account it ran on, until the
///   server confirms nothing carrying it is alive; only then is the task closed and the
///   video `cancelled`;
/// - a task another copy of the application is running (still `running` after
///   `recover_after_start`) is not ours: nothing of it is stopped or closed, and the video
///   waits for it.
///
/// The mark is unique to one start, so a cutting of the same set started by another copy
/// carries another mark and is never reached by this stop.
fn stop_after_restart(state: &AppState, row: &VideoRow) {
    let mut pending: Vec<(String, crate::store::remote_runs::RemoteRun)> = Vec::new();
    let mut foreign: Option<String> = None;
    for task in unfinished(state, &row.id) {
        if state.tasks.is_alive(&task) {
            continue;
        }
        let record = crate::tasks::store::get(&state.db, &task).ok().flatten();
        if record
            .as_ref()
            .is_some_and(|t| t.state == TaskState::Running)
        {
            foreign = Some(task);
            continue;
        }
        match crate::store::remote_runs::get(&state.db, &task)
            .ok()
            .flatten()
        {
            Some(run) if run.var == crate::domain::hls_package::JOB_VAR => {
                pending.push((task, run));
            }
            _ => {
                let _ = state.tasks.cancel(&task);
            }
        }
    }
    if pending.is_empty() {
        let _ = change(state, &row.id, |r| {
            match &foreign {
                // Another copy's work: its ending, when the watcher hears of it, ends this.
                Some(task) => r.task_id = Some(task.clone()),
                None => {
                    r.state = VideoState::Cancelled;
                    r.task_id = None;
                }
            }
            Ok(())
        });
        return;
    }
    lock(&state.videos.inner.stopping).insert(row.id.clone());
    let first = pending[0].0.clone();
    let _ = change(state, &row.id, |r| {
        r.task_id = Some(first.clone());
        Ok(())
    });
    let state = state.clone();
    let vid = row.id.clone();
    let server_id = row.server_id.clone();
    spawn(async move {
        for (_, run) in &pending {
            confirm_remote_stop(&state, &server_id, run).await;
        }
        lock(&state.videos.inner.stopping).remove(&vid);
        for (task, _) in &pending {
            let _ = crate::store::remote_runs::clear(&state.db, task);
            // Its ending reaches the watcher, which makes the video `cancelled` — or takes it
            // off the list, when «Remove» was what had been pressed.
            let _ = state.tasks.cancel(task);
        }
        let _ = change(&state, &vid, |r| {
            if r.state == VideoState::Cancelling && foreign.is_none() {
                r.state = VideoState::Cancelled;
                r.task_id = None;
            }
            Ok(())
        });
    });
}

/// Try the stop of one start on the server until the server confirms it (T682). Does not
/// return before then: what it protects is exactly the files that start may be writing.
async fn confirm_remote_stop(
    state: &AppState,
    server_id: &str,
    run: &crate::store::remote_runs::RemoteRun,
) {
    let attempt = || remote_stop_once(state, server_id, run);
    match attempt().await {
        Ok(how) => {
            tracing::info!(mark = %run.mark, ?how, "the stop left from the last run is confirmed");
        }
        Err(why) => {
            crate::server::marked::retry_until_confirmed("cutting", &run.mark, why, attempt).await;
        }
    }
}

/// One attempt: a fresh connection to the server and account the work ran on — not to
/// wherever the profile points by now (the lesson of T647) — and the stop of that mark.
async fn remote_stop_once(
    state: &AppState,
    server_id: &str,
    run: &crate::store::remote_runs::RemoteRun,
) -> std::result::Result<crate::server::hls_package::Stopped, String> {
    let profile = crate::store::profiles::get(&state.db, server_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| String::from("the server's profile is gone"))?;
    let mut at = profile.clone();
    let same_place = profile.host == run.host && profile.port == run.port;
    at.host = run.host.clone();
    at.port = run.port;
    at.user = run.user.clone();
    if !same_place {
        // The profile was pointed elsewhere meanwhile: the run's own machine is checked by
        // the fingerprint confirmed for it, and never by the profile's new one.
        at.host_fingerprint = crate::ssh::fingerprint::stored(
            &state.db,
            &crate::ssh::ServerAddress::new(&run.host, run.port),
        )
        .ok()
        .flatten();
    }
    let target = crate::server::gate::StopTarget::new(
        &at,
        super::deploy::own_credentials(state.secrets.as_ref(), &profile),
    )
    .ok_or_else(|| String::from("no confirmed fingerprint for the server the work ran on"))?;
    let opened = crate::server::gate::open_to_stop(&target, None)
        .await
        .map_err(|refusal| format!("the gate would not open: {refusal}"))?;
    let stopped = crate::server::marked::stop_confirmed(
        &opened.conn,
        &run.var,
        &run.mark,
        crate::server::marked::Patience::NONE,
    )
    .await;
    opened.conn.close().await;
    stopped.map_err(|problem| problem.to_string())
}

pub mod api {
    use super::*;

    /// Add files to be shown as videos with a plan. Each file on its own: one that is not a
    /// film is refused with its reason, and the rest are added (as T665).
    ///
    /// **With `media_id`** (T675): exactly one file, built into that medium of the library —
    /// its title and short name, no «name taken». The medium's own files stay as they are
    /// (T577, part b); the set is filed beside them. Refused as a whole, nothing added:
    /// `INVALID_INPUT` for not exactly one file or no such medium (`MEDIA_NOT_FOUND`);
    /// `MEDIA_HAS_SET` when it already has a set of its own; `MEDIA_SET_IN_WORK` when one is
    /// being built or another video on the list is on its way to it. The server is asked, so
    /// an unreachable server refuses the call with its own code.
    ///
    /// **A set of its name nobody claims on the server is not a refusal** (T677, the owner's
    /// decision of 2026-10-02): the video is added stopped on `MEDIA_HAS_SET` +
    /// `OLD_SET_UNRECOGNIZED`, with «Replace» — `video_replace` removes that set and builds.
    pub async fn video_add(
        state: &AppState,
        server_id: &str,
        paths: &[String],
        media_id: Option<&str>,
    ) -> Result<VideoAdded> {
        ensure_watching(state);
        // The server has to exist; a list of videos for nowhere helps nobody.
        super::super::library::api::profile_of(state, server_id)?;

        let existing = rows::list(&state.db).map_err(storage)?;
        let into = match media_id {
            Some(media_id) => {
                Some(into_medium(state, server_id, paths, media_id, &existing).await?)
            }
            None => None,
        };
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
            let mut row = match &into {
                Some(m) => {
                    let mut row = VideoRow::new(&id, server_id, path, &m.title, &m.slug);
                    row.media_id = Some(m.id.clone());
                    // The medium has no set (refused otherwise), so nobody can be watching
                    // one: the build needs no «anyway» against viewers (T571) — the same
                    // footing as a medium this video made itself. Its own files are not
                    // touched by the build (`tasks::ladder_build::name_prepared_files`).
                    row.own_medium = true;
                    row
                }
                None => {
                    let title = video::title_of(path);
                    let slug = video::slug_for(&title, &id);
                    VideoRow::new(&id, server_id, path, &title, &slug)
                }
            };
            row.audio_track = video::default_audio(&source);
            // T695: with more than one track the person chooses; none is taken for them.
            row.audio_chosen = source.audio_tracks.len() <= 1;
            row.source_json = serde_json::to_string(&source).ok();
            // A set of the medium's name nobody claims (T677): added stopped on it, with
            // «Replace». The plan is still made meanwhile, so «Replace» starts at once.
            if let Some(old) = into.as_ref().and_then(|m| m.old.clone()) {
                set_problem(&mut row, old);
            }
            rows::save(&state.db, &row).map_err(storage)?;
            begin_planning(state, &id);
            added.push(view_of(state, &row));
            emit(state, &id);
        }
        Ok(VideoAdded { added, refused })
    }

    /// The medium a video is to be built into (T675).
    struct Target {
        id: String,
        title: String,
        slug: String,
        /// A set of its name nobody claims is on the server (T677): the video is added
        /// stopped on this, with «Replace».
        old: Option<AppError>,
    }

    /// Whether a set may be built into this medium, and what it is called.
    async fn into_medium(
        state: &AppState,
        server_id: &str,
        paths: &[String],
        media_id: &str,
        existing: &[VideoRow],
    ) -> Result<Target> {
        use super::super::library::api as library;
        if paths.len() != 1 {
            return Err(AppError::new(ErrorCode::InvalidInput)
                .with_cause(format!("one file for a medium, not {}", paths.len())));
        }
        let in_work = |cause: String| AppError::new(ErrorCode::MediaSetInWork).with_cause(cause);
        // Asked here first, before the server: another video on its way to this medium.
        if let Some(v) = existing.iter().find(|v| {
            v.server_id == server_id
                && v.media_id.as_deref() == Some(media_id)
                && v.state.is_unfinished()
        }) {
            return Err(in_work(format!("video {} is on its way to it", v.id)));
        }

        let profile = library::profile_of(state, server_id)?;
        let conn = crate::server::gate::open(
            state.secrets.as_ref(),
            &profile,
            crate::server::gate::Intent::Read,
        )
        .await?
        .conn;
        let looked = async {
            let manifest = crate::server::manifest_io::read(&conn, &profile.video_dir).await?;
            let entries: Vec<(String, bool)> =
                crate::server::listing::list(&conn, &profile.video_dir)
                    .await?
                    .into_iter()
                    .map(|e| (e.name, e.is_dir))
                    .collect();
            Ok::<_, AppError>((manifest, entries))
        }
        .await;
        conn.close().await;
        let (manifest, entries) = looked?;

        let Some(medium) = manifest.find_by_id(media_id) else {
            return Err(AppError::new(ErrorCode::InvalidInput)
                .detail(DetailCode::MediaNotFound)
                .with_cause(media_id));
        };
        if !medium.ladders.is_empty() {
            return Err(AppError::new(ErrorCode::MediaHasSet).with_cause(medium.ladders.join(", ")));
        }
        // A set of its name nobody claims: an old build, or another film's. It would be taken
        // for this film's rungs by length alone (the reason for T676). Not a refusal (the
        // owner's decision of 2026-10-02, T677): the video is added stopped on it, with
        // «Replace» — removing it is that one click, confirmed (T577, part b).
        let old = video::old_set_problem(&medium.slug, &entries, &manifest.all_claimed_paths());
        if let Some(task) =
            super::super::ladder::api::running_build_for(state, server_id, &medium.slug)?
        {
            return Err(in_work(format!("build {task}")));
        }
        if let Some(v) = existing
            .iter()
            .find(|v| v.server_id == server_id && v.slug == medium.slug && v.state.is_unfinished())
        {
            return Err(in_work(format!("video {} has its name", v.id)));
        }
        Ok(Target {
            id: medium.id.clone(),
            title: medium.title.clone(),
            slug: medium.slug.clone(),
            old,
        })
    }

    /// Every video, in the order they were added.
    pub fn video_list(state: &AppState) -> Result<Vec<VideoView>> {
        ensure_watching(state);
        // One version for the whole list, taken before it is read (T687): an event sent
        // after it has a higher one, and wins over this list however late the list arrives.
        let rev = state.videos.next_rev();
        Ok(rows::list(&state.db)
            .map_err(storage)?
            .iter()
            .map(|r| view_at(state, r, rev))
            .collect())
    }

    /// One video.
    pub fn video_get(state: &AppState, id: &str) -> Result<VideoView> {
        let rev = state.videos.next_rev();
        Ok(view_at(state, &load(state, id)?, rev))
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
            row.audio_chosen = true;
            Ok(())
        })?;
        video_get(state, id)
    }

    /// Choose the subtitles drawn into the picture (T696, owner's decision B3): a track of the
    /// source that can be drawn, or `None` for none. At the same moments as the audio track —
    /// before anything is encoded — because the choice changes every frame of every rung.
    pub fn video_set_subtitles(
        state: &AppState,
        id: &str,
        track: Option<usize>,
    ) -> Result<VideoView> {
        change(state, id, |row| {
            if !video::allowed(Act::SetAudio, row.state, row.stage, row.media_id.is_some()) {
                return Err(not_now(row));
            }
            if let Some(track) = track {
                let drawable = source_of(row).is_some_and(|s| {
                    crate::domain::ladder_build::subtitle_burn(&s, track).is_some()
                });
                if !drawable {
                    return Err(AppError::new(ErrorCode::InvalidInput).with_detail(
                        Detail::new(DetailCode::PlanNoSuchSubtitles).with("number", track + 1),
                    ));
                }
            }
            row.subtitle_track = track;
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
    ///
    /// **A rung not measured yet is taken** (T680, the owner's decision of 2026-10-02): an
    /// edited rung, or a preliminary one chosen before the first «Start». It is saved as it
    /// is — never marked measured — and the plan says it needs measuring; «Start» measures
    /// what is missing, then builds. Refused only what no measurement can answer:
    /// no rungs, or a rung that is not a whole number of megabits (no point of the grid).
    pub fn video_set_rungs(
        state: &AppState,
        id: &str,
        rungs: Option<Vec<Rung>>,
    ) -> Result<VideoView> {
        if let Some(rungs) = &rungs {
            if rungs.is_empty() {
                return Err(AppError::new(ErrorCode::InvalidInput));
            }
            let unmeasurable: Vec<String> = rungs
                .iter()
                .enumerate()
                .filter(|(_, r)| {
                    !r.quality.is_enough_to_build_on()
                        && crate::domain::ladder::cell_of(r).is_none()
                })
                .map(|(i, _)| (i + 1).to_string())
                .collect();
            if !unmeasurable.is_empty() {
                return Err(AppError::new(ErrorCode::LadderNotMeasured)
                    .with_cause(format!("rungs {}", unmeasurable.join(", "))));
            }
        }
        change(state, id, |row| {
            if !video::allowed(Act::SetRungs, row.state, row.stage, row.media_id.is_some()) {
                return Err(not_now(row));
            }
            // What this film already has measured for these rungs is taken now (T680), so the
            // plan says exactly what «Start» will still have to measure.
            let rungs = rungs
                .as_ref()
                .map(|r| with_measured_here(state, &row.source_path, r));
            row.rungs_json = rungs.as_ref().and_then(|r| serde_json::to_string(r).ok());
            Ok(())
        })?;
        video_get(state, id)
    }

    /// «Start» — each video on its own. One still being planned starts the moment its plan is
    /// ready.
    ///
    /// **In the order asked** (T700): the videos of one «Start all» are put on the queue one
    /// after another, the first first. Started side by side, whichever made its medium on the
    /// server soonest took the first place, and the card at the top waited behind the second.
    pub fn video_start(state: &AppState, ids: &[String]) -> Vec<VideoStarted> {
        ensure_watching(state);
        let mut going: Vec<String> = Vec::new();
        let answers = ids
            .iter()
            .map(|id| {
                let outcome = change(state, id, |row| {
                    if !video::allowed(Act::Start, row.state, row.stage, row.media_id.is_some()) {
                        return Err(not_now(row));
                    }
                    audio_not_chosen(row)?;
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
                            going.push(id.clone());
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
            .collect();
        if !going.is_empty() {
            let state = state.clone();
            spawn(async move {
                for id in going {
                    carry_on(&state, &id).await;
                }
            });
        }
        answers
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
        let next = change(state, id, |row| {
            if !video::allowed(Act::Retry, row.state, row.stage, row.media_id.is_some()) {
                return Err(not_now(row));
            }
            audio_not_chosen(row)?;
            // Waiting on a set of the medium's name nobody claims (T677): going on would build
            // over it, taking its rungs for this film's. «Retry» puts the video back on that
            // problem, where «Replace» is what goes on.
            if row.stage == VideoStage::Planned
                && problem_of(row).is_some_and(|p| video::is_old_set_problem(&p.error))
            {
                row.state = VideoState::Problem;
                row.paused_by_person = false;
                return Ok(None);
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
                Ok(Some(true))
            } else {
                row.state = VideoState::Working;
                row.start_requested = true;
                Ok(Some(false))
            }
        })?;
        match next {
            Some(true) => begin_planning(state, id),
            Some(false) => start_going(state, id),
            None => {}
        }
        video_get(state, id)
    }

    /// The name is taken: build into the medium that has it, **its old set removed first**
    /// (T676, the owner's decision of 2026-10-01).
    ///
    /// **Why removed and not reused.** `ladder_build` takes a rung on the server for done
    /// when it lasts as long as the source, within a second, and the cutting leaves alone a
    /// variant already cut whole. Under a taken name there may be another film of the same
    /// length — its rungs would be taken for this one's and served under its link. So the
    /// set's directory (`{slug}/`: the cut rungs and `master.m3u8`) and its prepared rungs
    /// (`{slug}_{N}.mp4`, `.part`) go, through the same guarded `rm` deleting a medium uses,
    /// and the set is built again whole. The medium's other files stay (T577, part b).
    ///
    /// **Only here.** A video that carries on after a restart, or after «Retry», still finds
    /// its own rungs done: nothing but this command removes anything.
    ///
    /// Refused, with nothing removed: `FILE_IN_USE` while the server is serving somebody and
    /// `confirmed` is false (as T571 — the way on is «anyway», `confirmed: true`);
    /// `MEDIA_BUSY` while a build or a sending of this name is running;
    /// `INVALID_INPUT` + `RUNG_FILE_CLAIMED` when a medium claims a file named like
    /// one of the set's rungs (somebody's file, not ours to remove, and the build would
    /// take it for a finished rung).
    ///
    /// **And for a set built into a medium** (T677, the owner's decision of 2026-10-02): a
    /// video added with a medium while a set of the medium's name nobody claims lay on the
    /// server waits on that, and «Replace» removes it the same way. The medium's own files are
    /// its own — the person chose this medium for this film — so they are not a refusal here:
    /// they are left alone, and a build names a rung around one that is not that rung.
    pub async fn video_replace(state: &AppState, id: &str, confirmed: bool) -> Result<VideoView> {
        let row = load(state, id)?;
        let may = may_replace_row;
        if !may(&row) {
            return Err(not_now(&row));
        }
        audio_not_chosen(&row)?;
        let into_medium = held_for_replace(&row);
        // One replace at a time for a video: a second click must not remove what the first
        // has just begun to build.
        if !lock(&state.videos.inner.starting).insert(id.to_owned()) {
            return Err(not_now(&row));
        }
        // **Kept before anything is removed** (T686, QA-25 №7): what was confirmed, so that a
        // restart between the removal and the build carries the replace through rather than
        // leaving the video on its old problem with the old set already gone.
        let noted = change(state, id, |r| {
            r.replacing_json = serde_json::to_string(&video::Replacing {
                phase: video::ReplacePhase::Deleting,
                into_medium,
                confirmed,
                media_id: r.media_id.clone(),
            })
            .ok();
            Ok(())
        });
        if let Err(e) = noted {
            lock(&state.videos.inner.starting).remove(id);
            return Err(e);
        }
        let outcome = replace_through(state, id, confirmed, into_medium).await;
        lock(&state.videos.inner.starting).remove(id);
        if outcome? {
            begin_planning(state, id);
        } else {
            start_going(state, id);
        }
        video_get(state, id)
    }

    /// The rest of a confirmed «Replace», from wherever it is (T686): the old set removed
    /// (again — a repeat removes nothing new and asks the same questions), then the video set
    /// going. `Ok(true)` when its plan has to be made first. On a refusal the note is struck
    /// out and the video stays on its problem, «Replace» still there.
    async fn replace_through(
        state: &AppState,
        id: &str,
        confirmed: bool,
        into_medium: bool,
    ) -> Result<bool> {
        let row = load(state, id)?;
        let may = may_replace_row;
        let building = replacing_of(&row).is_some_and(|r| r.phase == video::ReplacePhase::Building);
        let cleared = if building {
            Ok(replacing_of(&row).and_then(|r| r.media_id))
        } else {
            clear_old_set(state, &row, confirmed, into_medium).await
        };
        // The old set is gone: from here a restart only builds (T686).
        if let Ok(media_id) = &cleared {
            let _ = change(state, id, |r| {
                if let Some(mut replacing) = replacing_of(r) {
                    replacing.phase = video::ReplacePhase::Building;
                    replacing.media_id = media_id.clone();
                    r.replacing_json = serde_json::to_string(&replacing).ok();
                }
                Ok(())
            });
        }
        let outcome = cleared.and_then(|media_id| {
            change(state, id, |r| {
                if !may(r) {
                    return Err(not_now(r));
                }
                r.replacing_json = None;
                r.media_id = media_id.clone();
                // Somebody's medium before this video came (it held the name): never the
                // video's to take away with it (T700).
                r.made_medium = false;
                // The set this medium had is gone, so nobody can be watching it: the build
                // that makes it again needs no «anyway» against viewers (T571), as for a
                // medium this video made itself.
                r.own_medium = true;
                r.problem_json = None;
                r.start_requested = true;
                // A video for a medium may still be waiting for its plan (T677): it is made
                // again, and the video starts the moment it is ready.
                if r.plan_json.is_none() {
                    r.state = VideoState::Planning;
                    Ok(true)
                } else {
                    r.state = VideoState::Working;
                    Ok(false)
                }
            })
        });
        if outcome.is_err() {
            let _ = change(state, id, |r| {
                r.replacing_json = None;
                Ok(())
            });
        }
        outcome
    }

    /// Carry a confirmed «Replace» through after a restart (T686).
    pub(super) fn replace_after_restart(state: &AppState, row: &VideoRow) {
        let Some(replacing) = replacing_of(row) else {
            return;
        };
        if !lock(&state.videos.inner.starting).insert(row.id.clone()) {
            return;
        }
        let state = state.clone();
        let id = row.id.clone();
        spawn(async move {
            // A server that does not answer yet is asked again — the replace was confirmed,
            // and the old set may be half gone; a refusal (busy, watched, claimed, gone) puts
            // the video back on its problem with «Replace» to press again.
            let mut pause = Duration::from_secs(2);
            let outcome = loop {
                let outcome =
                    replace_through(&state, &id, replacing.confirmed, replacing.into_medium).await;
                match &outcome {
                    Err(e)
                        if e.code == ErrorCode::SshUnreachable
                            && load(&state, &id).is_ok_and(|r| r.replacing_json.is_some()) =>
                    {
                        tokio::time::sleep(pause).await;
                        pause = (pause * 2).min(Duration::from_secs(60));
                    }
                    _ => break outcome,
                }
            };
            lock(&state.videos.inner.starting).remove(&id);
            match outcome {
                Ok(true) => begin_planning(&state, &id),
                Ok(false) => start_going(&state, &id),
                Err(e) => {
                    // The video is still on the problem it was replacing, «Replace» on it:
                    // pressing it asks again, with whatever the server now says.
                    tracing::warn!(video = %id, error = %e, "the replace could not be carried through");
                    emit(&state, &id);
                }
            }
        });
    }

    /// Remove what a set of this video's name has on the server; the medium that holds the
    /// name, if any is left.
    ///
    /// `into_medium` (T677): the video was added for a medium of the library and waits on a
    /// set of the medium's name that nobody claims. The medium has to be there still and still
    /// have no set of its own — its own set is never removed here — and its own files are left
    /// alone rather than refused: the person chose this medium for this film.
    async fn clear_old_set(
        state: &AppState,
        row: &VideoRow,
        confirmed: bool,
        into_medium: bool,
    ) -> Result<Option<String>> {
        use super::super::library::api as library;
        let profile = library::profile_of(state, &row.server_id)?;
        let conn = crate::server::gate::open(
            state.secrets.as_ref(),
            &profile,
            crate::server::gate::Intent::Change,
        )
        .await?
        .conn;
        let cleared = async {
            let manifest = crate::server::manifest_io::read(&conn, &profile.video_dir).await?;
            let media_id = if into_medium {
                let wanted = row.media_id.as_deref().unwrap_or_default();
                let Some(medium) = manifest.find_by_id(wanted) else {
                    return Err(AppError::new(ErrorCode::InvalidInput)
                        .detail(DetailCode::MediaNotFound)
                        .with_cause(wanted));
                };
                if !medium.ladders.is_empty() {
                    return Err(
                        AppError::new(ErrorCode::MediaHasSet).with_cause(medium.ladders.join(", "))
                    );
                }
                Some(medium.id.clone())
            } else {
                manifest.find_by_slug(&row.slug).map(|m| m.id.clone())
            };
            let entries: Vec<(String, bool)> =
                crate::server::listing::list(&conn, &profile.video_dir)
                    .await?
                    .into_iter()
                    .map(|e| (e.name, e.is_dir))
                    .collect();
            let claimed = manifest.all_claimed_paths();
            let old = if into_medium {
                video::unclaimed_old_set(&row.slug, &entries, &claimed)
            } else {
                video::old_set(&row.slug, &entries, &claimed)
            };
            if let Some(name) = old.in_the_way.first().filter(|_| !into_medium) {
                return Err(AppError::new(ErrorCode::InvalidInput)
                    .with_detail(
                        Detail::new(DetailCode::RungFileClaimed).with("name", name.clone()),
                    )
                    .with_cause(old.in_the_way.join(", ")));
            }
            if old.is_empty() {
                return Ok(media_id);
            }
            // The name itself is asked about even when only loose rungs are left: a build of
            // it writes `{slug}_{N}.mp4`, and those tops are not the slug.
            let mut tops = old.tops(&row.slug);
            if !tops.contains(&row.slug) {
                tops.push(row.slug.clone());
            }
            if let Some(busy) =
                library::refuse_if_busy(state, &row.server_id, &tops, ErrorCode::MediaBusy)?
            {
                return Err(busy);
            }
            if !confirmed {
                let connections = library::active_connections(&conn).await;
                if connections > 0 {
                    return Err(AppError::new(ErrorCode::FileInUse)
                        .with_cause(format!("connections={connections}")));
                }
            }
            // T686: the medium whose set is about to go is named in the note before a byte
            // is removed — the library reads it as building from here, not as missing.
            if let Some(media) = &media_id {
                let _ = change(state, &row.id, |r| {
                    if let Some(mut replacing) = replacing_of(r) {
                        replacing.media_id = Some(media.clone());
                        r.replacing_json = serde_json::to_string(&replacing).ok();
                    }
                    Ok(())
                });
            }
            library::remove_entries(&conn, &profile.video_dir, old.tops(&row.slug).iter()).await?;
            tracing::info!(
                video = %row.id,
                slug = %row.slug,
                removed = old.tops(&row.slug).len(),
                "the old set under a taken name was removed before building it again"
            );
            state.invalidate_library(&row.server_id);
            Ok(media_id)
        }
        .await;
        conn.close().await;
        cleared
    }

    /// Take a video off the list. Nothing on the server is touched (T577, part b).
    ///
    /// **With its work alive, the work is stopped first** (T683, the owner's decision of
    /// 2026-10-02): a paused build, a task waiting for its place, a stop still being
    /// confirmed. The video goes `cancelling`, exactly as after «Cancel», with
    /// `remove_requested` kept; it leaves the list — `video:removed` — the moment everything
    /// has stopped, the server's cutting included, and not before: a build with no video
    /// left to answer for it is what this prevents. A restart in between finishes the job.
    ///
    /// Answers `null` when the video is gone at once, and the video as it is now
    /// (`cancelling`) when it leaves once its work has stopped.
    pub fn video_remove(state: &AppState, id: &str) -> Result<Option<VideoView>> {
        ensure_watching(state);
        if let Some(planning) = lock(&state.videos.inner.planning).remove(id) {
            planning.cancel();
        }
        let living = alive(state, id);
        let busy = !living.is_empty() || lock(&state.videos.inner.starting).contains(id);
        let gone = {
            let _held = lock(&state.videos.inner.rows);
            let mut row = load(state, id)?;
            if !busy && row.state != VideoState::Cancelling {
                rows::remove(&state.db, id).map_err(storage)?;
                drop_medium_left_empty(state, &row);
                None
            } else {
                row.remove_requested = true;
                row.state = VideoState::Cancelling;
                row.paused_by_person = false;
                row.start_requested = false;
                rows::save(&state.db, &row).map_err(storage)?;
                Some(view_of(state, &row))
            }
        };
        let Some(stopping) = gone else {
            // Rows of tasks a run that is over left behind: nothing works behind them, and a
            // video that is gone must not leave them looking unfinished.
            cancel_tasks_of(state, id);
            forget_video(state, id);
            return Ok(None);
        };
        emit(state, id);
        cancel_tasks_of(state, id);
        Ok(Some(stopping))
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
            // A confirmed «Replace» the last run did not finish (T686) is carried through,
            // whatever the video's state reads: it is still on the problem it was replacing.
            if row.replacing_json.is_some() {
                replace_after_restart(state, &row);
                continue;
            }
            match video::after_restart(row.state) {
                video::AfterRestart::PlanAgain => begin_planning(state, &row.id),
                video::AfterRestart::CarryOn => {
                    carried += 1;
                    start_going(state, &row.id);
                }
                video::AfterRestart::NowCancelled => stop_after_restart(state, &row),
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
        media_id: Option<String>,
    ) -> Result<VideoAdded> {
        api::video_add(&state, &server_id, &paths, media_id.as_deref()).await
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
    pub fn video_set_subtitles(
        state: State<'_, AppState>,
        id: String,
        track: Option<usize>,
    ) -> Result<VideoView> {
        api::video_set_subtitles(&state, &id, track)
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
    pub async fn video_replace(
        state: State<'_, AppState>,
        id: String,
        confirmed: bool,
    ) -> Result<VideoView> {
        api::video_replace(&state, &id, confirmed).await
    }

    #[tauri::command]
    pub fn video_remove(state: State<'_, AppState>, id: String) -> Result<Option<VideoView>> {
        api::video_remove(&state, &id)
    }
}
