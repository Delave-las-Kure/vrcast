//! T672 — the videos in work, as they are kept between runs.
//!
//! Only reading and writing rows. What a video may do and what its stages mean is
//! `domain::video`; what the plan and the problem hold is `commands::video` — kept here as the
//! JSON those types serialise to, so this layer does not need to know their shape.

use crate::domain::video::{VideoStage, VideoState};
use crate::store::db::{now_rfc3339, Db, DbError};

/// One video, as stored.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoRow {
    pub id: String,
    pub server_id: String,
    pub source_path: String,
    pub title: String,
    pub slug: String,
    pub audio_track: usize,
    pub stage: VideoStage,
    pub state: VideoState,
    pub paused_by_person: bool,
    pub start_requested: bool,
    pub measured: bool,
    pub own_medium: bool,
    pub confirmed: bool,
    pub task_id: Option<String>,
    pub media_id: Option<String>,
    pub source_json: Option<String>,
    pub plan_json: Option<String>,
    pub rungs_json: Option<String>,
    pub problem_json: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl VideoRow {
    /// A video just added: its plan is still to be worked out.
    pub fn new(id: &str, server_id: &str, source_path: &str, title: &str, slug: &str) -> Self {
        let now = now_rfc3339();
        Self {
            id: id.to_owned(),
            server_id: server_id.to_owned(),
            source_path: source_path.to_owned(),
            title: title.to_owned(),
            slug: slug.to_owned(),
            audio_track: 0,
            stage: VideoStage::Planned,
            state: VideoState::Planning,
            paused_by_person: false,
            start_requested: false,
            measured: false,
            own_medium: false,
            confirmed: false,
            task_id: None,
            media_id: None,
            source_json: None,
            plan_json: None,
            rungs_json: None,
            problem_json: None,
            created_at: now.clone(),
            updated_at: now,
        }
    }
}

fn row_to_video(row: &rusqlite::Row<'_>) -> rusqlite::Result<VideoRow> {
    let stage: String = row.get("stage")?;
    let state: String = row.get("state")?;
    Ok(VideoRow {
        id: row.get("id")?,
        server_id: row.get("server_id")?,
        source_path: row.get("source_path")?,
        title: row.get("title")?,
        slug: row.get("slug")?,
        audio_track: row.get::<_, i64>("audio_track")?.max(0) as usize,
        // A stage or a state this build does not know was written by a newer one. The safest
        // reading is the start, standing still: nothing runs by itself from there.
        stage: VideoStage::parse(&stage).unwrap_or(VideoStage::Planned),
        state: VideoState::parse(&state).unwrap_or(VideoState::Problem),
        paused_by_person: row.get::<_, i64>("paused_by_person")? != 0,
        start_requested: row.get::<_, i64>("start_requested")? != 0,
        measured: row.get::<_, i64>("measured")? != 0,
        own_medium: row.get::<_, i64>("own_medium")? != 0,
        confirmed: row.get::<_, i64>("confirmed")? != 0,
        task_id: row.get("task_id")?,
        media_id: row.get("media_id")?,
        source_json: row.get("source_json")?,
        plan_json: row.get("plan_json")?,
        rungs_json: row.get("rungs_json")?,
        problem_json: row.get("problem_json")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

/// Write a video whole: create it, or replace what is stored.
pub fn save(db: &Db, v: &VideoRow) -> Result<(), DbError> {
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO videos
                (id, server_id, source_path, title, slug, audio_track, stage, state,
                 paused_by_person, start_requested, measured, own_medium, confirmed,
                 task_id, media_id, source_json, plan_json, rungs_json, problem_json,
                 created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                     ?17, ?18, ?19, ?20, ?21)
             ON CONFLICT (id) DO UPDATE SET
                title = excluded.title,
                slug = excluded.slug,
                audio_track = excluded.audio_track,
                stage = excluded.stage,
                state = excluded.state,
                paused_by_person = excluded.paused_by_person,
                start_requested = excluded.start_requested,
                measured = excluded.measured,
                own_medium = excluded.own_medium,
                confirmed = excluded.confirmed,
                task_id = excluded.task_id,
                media_id = excluded.media_id,
                source_json = excluded.source_json,
                plan_json = excluded.plan_json,
                rungs_json = excluded.rungs_json,
                problem_json = excluded.problem_json,
                updated_at = excluded.updated_at",
            rusqlite::params![
                v.id,
                v.server_id,
                v.source_path,
                v.title,
                v.slug,
                v.audio_track as i64,
                v.stage.as_str(),
                v.state.as_str(),
                v.paused_by_person as i64,
                v.start_requested as i64,
                v.measured as i64,
                v.own_medium as i64,
                v.confirmed as i64,
                v.task_id,
                v.media_id,
                v.source_json,
                v.plan_json,
                v.rungs_json,
                v.problem_json,
                v.created_at,
                now_rfc3339(),
            ],
        )?;
        Ok(())
    })
}

/// Read one video.
pub fn get(db: &Db, id: &str) -> Result<Option<VideoRow>, DbError> {
    db.with_conn(|c| {
        let mut stmt = c.prepare("SELECT * FROM videos WHERE id = ?1")?;
        let mut rows = stmt.query([id])?;
        Ok(match rows.next()? {
            Some(row) => Some(row_to_video(row)?),
            None => None,
        })
    })
}

/// Every video, oldest first — the order they were added in.
pub fn list(db: &Db) -> Result<Vec<VideoRow>, DbError> {
    db.with_conn(|c| {
        let mut stmt = c.prepare("SELECT * FROM videos ORDER BY created_at, rowid")?;
        let rows = stmt
            .query_map([], row_to_video)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
}

/// Take a video off the list. Nothing on the server is touched (T577, part b).
pub fn remove(db: &Db, id: &str) -> Result<bool, DbError> {
    db.with_conn(|c| Ok(c.execute("DELETE FROM videos WHERE id = ?1", [id])? > 0))
}

/// Remember how fast an encoder made a rung on this machine, in pixels of output a second.
pub fn record_encode_speed(db: &Db, encoder: &str, pixels_per_s: f64) -> Result<(), DbError> {
    if !pixels_per_s.is_finite() || pixels_per_s <= 0.0 {
        return Ok(());
    }
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO encode_speeds (encoder, pixels_per_s, measured_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![encoder, pixels_per_s, now_rfc3339()],
        )?;
        Ok(())
    })
}

/// How fast this encoder has been here lately: the middle of its last timings, or nothing
/// when it has never been timed.
pub fn encode_speed(db: &Db, encoder: &str) -> Result<Option<f64>, DbError> {
    let timed: Vec<f64> = db.with_conn(|c| {
        let mut stmt = c.prepare(
            "SELECT pixels_per_s FROM encode_speeds WHERE encoder = ?1
             ORDER BY measured_at DESC LIMIT 20",
        )?;
        let rows = stmt
            .query_map([encoder], |r| r.get::<_, f64>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })?;
    Ok(crate::domain::video::middle_speed(timed))
}
