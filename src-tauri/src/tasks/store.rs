//! T017 — хранение задач: они переживают перезапуск приложения (FR-081).
//!
//! В базу пишутся не все подвижки, а только значимые: смена состояния, позиция
//! возобновления, ошибка. Прогресс, меняющийся сотни раз в секунду, в базу не идёт —
//! иначе диск станет узким местом у задачи, которая должна упираться в канал.

use super::state::{TaskKind, TaskState};
use crate::domain::wording::DetailCode;
use crate::error::AppError;
use crate::store::db::{now_rfc3339, Db, DbError};
use serde::{Deserialize, Serialize};

/// Задача в том виде, в каком она хранится и показывается.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRecord {
    pub id: String,
    pub kind: TaskKind,
    pub server_id: Option<String>,
    pub state: TaskState,
    pub progress: f64,
    /// Which stage it is at. A code, not a phrase: the wording belongs to the
    /// interface, and a task outlives the language it was started in.
    pub stage: Option<DetailCode>,
    pub speed_bps: Option<i64>,
    pub eta_s: Option<i64>,
    /// Позиция возобновления: переданные байты, готовые ступени, выполненные шаги.
    pub resume_token: Option<String>,
    /// Why it failed, if it did. Stored as an object rather than a sentence: a task
    /// finished a week ago must still explain itself in whatever language is chosen
    /// today. Secrets are already redacted.
    pub error: Option<AppError>,
    /// Место в очереди: меньше — раньше.
    ///
    /// Отдельно от времени создания, потому что перестановка (FR-083) обязана менять
    /// порядок, не подделывая время появления задачи.
    pub queue_order: i64,
    pub created_at: String,
    pub updated_at: String,
}

impl TaskRecord {
    pub fn new(id: impl Into<String>, kind: TaskKind, server_id: Option<String>) -> Self {
        let now = now_rfc3339();
        Self {
            id: id.into(),
            kind,
            server_id,
            state: TaskState::Queued,
            progress: 0.0,
            stage: None,
            speed_bps: None,
            eta_s: None,
            resume_token: None,
            error: None,
            queue_order: 0,
            created_at: now.clone(),
            updated_at: now,
        }
    }
}

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskRecord> {
    let kind: String = row.get("kind")?;
    let state: String = row.get("state")?;
    Ok(TaskRecord {
        id: row.get("id")?,
        kind: TaskKind::parse(&kind).unwrap_or(TaskKind::Probe),
        server_id: row.get("server_id")?,
        state: TaskState::parse(&state).unwrap_or(TaskState::Failed),
        progress: row.get("progress")?,
        // An unknown code means a task stored by a newer version of the application.
        // Showing nothing beats showing a key nobody can read.
        stage: row
            .get::<_, Option<String>>("stage")?
            .and_then(|s| DetailCode::parse(&s)),
        speed_bps: row.get("speed_bps")?,
        eta_s: row.get("eta_s")?,
        resume_token: row.get("resume_token")?,
        error: row
            .get::<_, Option<String>>("error")?
            .and_then(|s| serde_json::from_str(&s).ok()),
        queue_order: row.get("queue_order")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

/// Записать задачу: создать или обновить.
pub fn upsert(db: &Db, task: &TaskRecord) -> Result<(), DbError> {
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO tasks
                (id, kind, server_id, state, progress, stage, speed_bps, eta_s,
                 resume_token, error, queue_order, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT (id) DO UPDATE SET
                state = excluded.state,
                progress = excluded.progress,
                stage = excluded.stage,
                speed_bps = excluded.speed_bps,
                eta_s = excluded.eta_s,
                resume_token = excluded.resume_token,
                error = excluded.error,
                updated_at = excluded.updated_at",
            rusqlite::params![
                task.id,
                task.kind.as_str(),
                task.server_id,
                task.state.as_str(),
                task.progress,
                task.stage.map(|s| s.as_str()),
                task.speed_bps,
                task.eta_s,
                task.resume_token,
                task.error
                    .as_ref()
                    .and_then(|e| serde_json::to_string(e).ok()),
                task.queue_order,
                task.created_at,
                task.updated_at,
            ],
        )?;
        Ok(())
    })
}

/// Записать только позицию возобновления, не трогая остального.
///
/// Точечное обновление вместо «прочитать-изменить-записать» всей записи: токен пишет
/// работающая задача, а состояние — пауза или отмена из другого потока, и полная
/// перезапись с любой из сторон затирала бы чужое свежее поле.
pub fn save_resume_token(db: &Db, id: &str, token: &str) -> Result<(), DbError> {
    db.with_conn(|c| {
        c.execute(
            "UPDATE tasks SET resume_token = ?2, updated_at = ?3 WHERE id = ?1",
            rusqlite::params![id, token, now_rfc3339()],
        )?;
        Ok(())
    })
}

/// Записать только продвижение, не трогая остального.
///
/// Нужно ради перезапуска: событиями продвижение расходится по интерфейсу, но живёт
/// только в памяти. Без этой записи задача, прерванная на середине тридцатигигабайтной
/// заливки, после запуска приложения показывает ноль — и решить, продолжать её или
/// снять, человеку не по чему.
///
/// Записи о завершённых задачах не трогаются: сообщение о продвижении может прийти
/// с опозданием и сбить единицу у уже готовой задачи обратно на 0,98.
pub fn save_progress(db: &Db, id: &str, progress: f64) -> Result<(), DbError> {
    db.with_conn(|c| {
        c.execute(
            "UPDATE tasks SET progress = ?2, updated_at = ?3
             WHERE id = ?1 AND state NOT IN ('completed', 'failed', 'cancelled')",
            rusqlite::params![id, progress.clamp(0.0, 1.0), now_rfc3339()],
        )?;
        Ok(())
    })
}

/// Записать только состояние и, при наличии, ошибку — не трогая остального.
///
/// Возвращает `false`, если записи о задаче нет. Причина точечности та же,
/// что у [`save_resume_token`].
pub fn save_state(
    db: &Db,
    id: &str,
    state: TaskState,
    error: Option<&AppError>,
) -> Result<bool, DbError> {
    db.with_conn(|c| {
        let changed = c.execute(
            "UPDATE tasks SET
                state = ?2,
                progress = CASE WHEN ?2 = 'completed' THEN 1.0 ELSE progress END,
                error = COALESCE(?3, error),
                updated_at = ?4
             WHERE id = ?1",
            rusqlite::params![
                id,
                state.as_str(),
                error.and_then(|e| serde_json::to_string(e).ok()),
                now_rfc3339()
            ],
        )?;
        Ok(changed > 0)
    })
}

/// Прочитать одну задачу.
pub fn get(db: &Db, id: &str) -> Result<Option<TaskRecord>, DbError> {
    db.with_conn(|c| {
        let mut stmt = c.prepare("SELECT * FROM tasks WHERE id = ?1")?;
        let mut rows = stmt.query([id])?;
        Ok(match rows.next()? {
            Some(row) => Some(row_to_record(row)?),
            None => None,
        })
    })
}

/// Наибольший занятый номер в очереди.
///
/// Нужен при запуске: следующая задача обязана встать **после** всех, что уже лежат
/// в базе, иначе она молча влезет в середину очереди прошлого запуска.
pub fn max_queue_order(db: &Db) -> Result<i64, DbError> {
    db.with_conn(|c| {
        Ok(
            c.query_row("SELECT COALESCE(MAX(queue_order), 0) FROM tasks", [], |r| {
                r.get(0)
            })?,
        )
    })
}

/// Записать только место в очереди.
pub fn save_queue_order(db: &Db, id: &str, order: i64) -> Result<(), DbError> {
    db.with_conn(|c| {
        c.execute(
            "UPDATE tasks SET queue_order = ?2, updated_at = ?3 WHERE id = ?1",
            rusqlite::params![id, order, now_rfc3339()],
        )?;
        Ok(())
    })
}

/// Все задачи, свежие первыми.
pub fn list(db: &Db) -> Result<Vec<TaskRecord>, DbError> {
    db.with_conn(|c| {
        let mut stmt = c.prepare("SELECT * FROM tasks ORDER BY created_at DESC")?;
        let rows = stmt.query_map([], row_to_record)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    })
}

/// Итог восстановления после запуска.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RecoveryReport {
    /// Задачи, застигнутые в работе прошлым запуском. Переведены в приостановленные.
    pub interrupted: Vec<String>,
}

/// Разобрать состояние после запуска приложения.
///
/// Задачи, оставшиеся в состоянии «выполняется», принадлежат прошлому запуску: их процессов
/// больше нет. Они переводятся в **приостановленные** — и никогда в завершённые
/// (конституция, принцип III; SC-010). Разница не косметическая: «завершено» означало бы,
/// что результат готов, а он оборван на середине.
pub fn recover_after_start(db: &Db) -> Result<RecoveryReport, DbError> {
    let interrupted: Vec<String> = db.with_conn(|c| {
        let mut stmt = c.prepare("SELECT id FROM tasks WHERE state = 'running'")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    })?;

    if interrupted.is_empty() {
        return Ok(RecoveryReport::default());
    }

    db.with_conn(|c| {
        c.execute(
            "UPDATE tasks SET state = 'paused', updated_at = ?1 WHERE state = 'running'",
            [now_rfc3339()],
        )?;
        Ok(())
    })?;

    tracing::warn!(
        count = interrupted.len(),
        "задачи прошлого запуска переведены в приостановленные"
    );

    Ok(RecoveryReport { interrupted })
}

/// Убрать давно завершённые задачи.
pub fn purge_finished_before(db: &Db, before_rfc3339: &str) -> Result<usize, DbError> {
    db.with_conn(|c| {
        Ok(c.execute(
            "DELETE FROM tasks
             WHERE state IN ('completed', 'failed', 'cancelled') AND updated_at < ?1",
            [before_rfc3339],
        )?)
    })
}
