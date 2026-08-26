//! T236, T237 — keeping a quality measurement, and lending it to the next episode.
//!
//! Half an hour of somebody's machine goes into a measurement. It survives a cancellation,
//! a restart and a crash because the points are written as they are taken rather than at
//! the end.

use std::path::Path;

use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use crate::domain::measured_ladder::Point;
use crate::store::db::{now_rfc3339, Db, DbError};

/// What is being measured, and how.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub source_key: String,
    /// The codec the ladder is for, not the source's own.
    pub codec: String,
    pub source_path: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// The source's own bitrate — what caps the ladder made from these points.
    pub source_bitrate_bps: u64,
    /// Whether the source carries more picture per bit than H.264.
    pub heavier_codec: bool,
    pub native_height: Option<u32>,
    pub anchor_mbps: u64,
    pub chunk_starts: Vec<u64>,
    pub chunk_s: u64,
    /// Which file this measurement really came from, when it was not made here.
    pub borrowed_from: Option<String>,
}

impl Run {
    /// Whether the rungs resting on this can be called measured.
    pub fn is_measured_here(&self) -> bool {
        self.borrowed_from.is_none()
    }
}

/// What identifies the material.
///
/// Its size and its name. **Not its path**: a person who tidies their films into folders
/// has not changed a single frame, and making them measure again for half an hour would be
/// punishing them for housekeeping. Editing a file in place changes its size, and that is a
/// different film — as it should be.
pub fn key_for(path: &Path) -> std::io::Result<String> {
    let size = std::fs::metadata(path)?.len();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(format!("{size}:{name}"))
}

/// Begin, or resume, a measurement of this material for this codec.
///
/// Repeating it does not lose the points already taken: the header is replaced, the points
/// stay. That is what lets a cancelled run be picked up.
pub fn begin(db: &Db, run: &Run) -> Result<(), DbError> {
    let chunks = run
        .chunk_starts
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
        .join(",");
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO quality_measurements
                (source_key, codec, source_path, width, height, fps,
                 source_bitrate_bps, heavier_codec, native_height,
                 anchor_mbps, chunk_starts, chunk_s, borrowed_from, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
             ON CONFLICT (source_key, codec) DO UPDATE SET
                source_path = excluded.source_path,
                width = excluded.width,
                height = excluded.height,
                fps = excluded.fps,
                source_bitrate_bps = excluded.source_bitrate_bps,
                heavier_codec = excluded.heavier_codec,
                native_height = excluded.native_height,
                anchor_mbps = excluded.anchor_mbps,
                chunk_starts = excluded.chunk_starts,
                chunk_s = excluded.chunk_s,
                borrowed_from = excluded.borrowed_from,
                updated_at = excluded.updated_at",
            rusqlite::params![
                run.source_key,
                run.codec,
                run.source_path,
                run.width,
                run.height,
                run.fps,
                run.source_bitrate_bps as i64,
                run.heavier_codec,
                run.native_height,
                run.anchor_mbps as i64,
                chunks,
                run.chunk_s as i64,
                run.borrowed_from,
                now_rfc3339(),
            ],
        )?;
        Ok(())
    })
}

/// Write down one measured point, and how long it took.
pub fn record(
    db: &Db,
    source_key: &str,
    codec: &str,
    point: &Point,
    took: std::time::Duration,
) -> Result<(), DbError> {
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO quality_points
                (source_key, codec, bitrate_mbps, height, vmaf, actual_bps,
                 measured_at, took_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT (source_key, codec, bitrate_mbps, height) DO UPDATE SET
                vmaf = excluded.vmaf,
                actual_bps = excluded.actual_bps,
                measured_at = excluded.measured_at,
                took_ms = excluded.took_ms",
            rusqlite::params![
                source_key,
                codec,
                point.bitrate_mbps as i64,
                point.height,
                point.vmaf,
                point.actual_bps as i64,
                now_rfc3339(),
                took.as_millis().min(i64::MAX as u128) as i64,
            ],
        )?;
        Ok(())
    })
}

/// What has been measured so far.
pub fn points(db: &Db, source_key: &str, codec: &str) -> Result<Vec<Point>, DbError> {
    db.with_conn(|c| {
        let mut q = c.prepare(
            "SELECT bitrate_mbps, height, vmaf, actual_bps
             FROM quality_points WHERE source_key = ?1 AND codec = ?2
             ORDER BY bitrate_mbps, height",
        )?;
        let rows = q
            .query_map(rusqlite::params![source_key, codec], |r| {
                Ok(Point {
                    bitrate_mbps: r.get::<_, i64>(0)? as u64,
                    height: r.get(1)?,
                    vmaf: r.get(2)?,
                    actual_bps: r.get::<_, i64>(3)? as u64,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
}

/// How this machine compares with the one the cost model was measured on.
///
/// A plain number: 1.0 means it behaves as the model says, 3.0 that everything takes
/// three times as long here. Returned with the count of timed points behind it, so a
/// person can be told whether the estimate rests on their machine or on somebody
/// else's.
///
/// **A factor rather than a flat number of seconds**, because the flat number does not
/// carry between films: measure a small one and then a 4K one and the estimate would
/// be a quarter of the truth. The factor carries; the model handles the size.
///
/// **The middle value, not the average.** One point in a run is regularly an outlier —
/// the card throttles, something else wants the processor, a chunk is a hard one — and
/// an average carries that outlier into every estimate afterwards.
///
/// `None` until this machine has timed anything. There is nothing to learn from yet,
/// and a made-up correction would be worse than none.
pub fn machine_factor(db: &Db) -> Result<Option<(f64, usize)>, DbError> {
    db.with_conn(|c| {
        let mut q = c.prepare(
            "SELECT p.took_ms, m.width, m.height, m.fps, m.chunk_s, m.chunk_starts
             FROM quality_points p
             JOIN quality_measurements m
               ON m.source_key = p.source_key AND m.codec = p.codec
             WHERE p.took_ms > 0
             ORDER BY p.measured_at DESC LIMIT ?1",
        )?;
        let mut factors: Vec<f64> = q
            .query_map([RECENT_POINTS], |r| {
                let took_ms: i64 = r.get(0)?;
                let width: u32 = r.get(1)?;
                let height: u32 = r.get(2)?;
                let fps: u32 = r.get(3)?;
                let chunk_s: i64 = r.get(4)?;
                let chunks: String = r.get(5)?;
                let chunks = chunks.split(',').filter(|s| !s.trim().is_empty()).count();
                Ok(crate::domain::measure_grid::seconds_per_point(
                    width,
                    height,
                    fps,
                    chunk_s.max(0) as u64,
                    chunks,
                ))
                .map(|expected| (took_ms as f64 / 1000.0) / expected.max(0.001))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        if factors.is_empty() {
            return Ok(None);
        }
        let counted = factors.len();
        factors.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        Ok(Some((factors[counted / 2], counted)))
    })
}

/// How far back the estimate looks.
///
/// Recent ones only: a machine gets a new card, or the film being measured changes
/// kind, and last year's timings then describe neither.
const RECENT_POINTS: i64 = 60;

/// The header of a measurement, if there is one.
pub fn run(db: &Db, source_key: &str, codec: &str) -> Result<Option<Run>, DbError> {
    db.with_conn(|c| {
        Ok(c.query_row(
            "SELECT source_key, codec, source_path, width, height, fps,
                    source_bitrate_bps, heavier_codec, native_height,
                    anchor_mbps, chunk_starts, chunk_s, borrowed_from
             FROM quality_measurements WHERE source_key = ?1 AND codec = ?2",
            rusqlite::params![source_key, codec],
            row_to_run,
        )
        .optional()?)
    })
}

/// Every measurement kept, newest first.
///
/// This is what a person is offered the next episode from.
pub fn all(db: &Db) -> Result<Vec<Run>, DbError> {
    db.with_conn(|c| {
        let mut q = c.prepare(
            "SELECT source_key, codec, source_path, width, height, fps,
                    source_bitrate_bps, heavier_codec, native_height,
                    anchor_mbps, chunk_starts, chunk_s, borrowed_from
             FROM quality_measurements ORDER BY updated_at DESC",
        )?;
        let rows = q
            .query_map([], row_to_run)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
}

/// Why a measurement cannot be lent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LendRefusal {
    /// There is no such measurement to lend.
    NothingToLend,
    /// The two are not the same kind of material.
    ///
    /// A ladder measured on a native 4K master says nothing about an upscale, and the other
    /// way round: the point where the resolution should drop sits somewhere else entirely.
    /// Frame size, frame rate and the height the material really has must all agree.
    DifferentMaterial,
}

/// Lend one film's measurement to another — the next episode of the same season.
///
/// **The chunks are lent with it.** Chosen afresh the percentiles would be the same and the
/// scenes different, and then the difference between two episodes would mix into the
/// difference between two rungs.
///
/// The borrowed run is marked as borrowed. A rung standing on somebody else's measurement
/// is not a measured rung, and a person is owed the difference plainly (FR-145).
pub fn lend(db: &Db, from_key: &str, codec: &str, to: &Run) -> Result<Run, LendRefusal> {
    let source = match run(db, from_key, codec) {
        Ok(Some(r)) => r,
        _ => return Err(LendRefusal::NothingToLend),
    };
    if source.width != to.width
        || source.height != to.height
        || source.fps != to.fps
        || source.native_height != to.native_height
        || source.heavier_codec != to.heavier_codec
    {
        return Err(LendRefusal::DifferentMaterial);
    }

    let borrowed = Run {
        anchor_mbps: source.anchor_mbps,
        chunk_starts: source.chunk_starts.clone(),
        chunk_s: source.chunk_s,
        borrowed_from: Some(source.source_path.clone()),
        ..to.clone()
    };
    if begin(db, &borrowed).is_err() {
        return Err(LendRefusal::NothingToLend);
    }

    let lent = points(db, from_key, codec).unwrap_or_default();
    for point in &lent {
        // Zero time: nothing was measured here, and counting a lent point as work done
        // on this machine would flatter every estimate that follows.
        let _ = record(
            db,
            &borrowed.source_key,
            codec,
            point,
            std::time::Duration::ZERO,
        );
    }
    Ok(borrowed)
}

/// Throw a measurement away — the material was re-encoded, or the person wants it redone.
pub fn forget(db: &Db, source_key: &str, codec: &str) -> Result<(), DbError> {
    db.with_conn(|c| {
        c.execute(
            "DELETE FROM quality_points WHERE source_key = ?1 AND codec = ?2",
            rusqlite::params![source_key, codec],
        )?;
        c.execute(
            "DELETE FROM quality_measurements WHERE source_key = ?1 AND codec = ?2",
            rusqlite::params![source_key, codec],
        )?;
        Ok(())
    })
}

fn row_to_run(r: &rusqlite::Row) -> rusqlite::Result<Run> {
    let chunks: String = r.get(10)?;
    Ok(Run {
        source_key: r.get(0)?,
        codec: r.get(1)?,
        source_path: r.get(2)?,
        width: r.get(3)?,
        height: r.get(4)?,
        fps: r.get(5)?,
        source_bitrate_bps: r.get::<_, i64>(6)? as u64,
        heavier_codec: r.get(7)?,
        native_height: r.get(8)?,
        anchor_mbps: r.get::<_, i64>(9)? as u64,
        chunk_starts: chunks
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect(),
        chunk_s: r.get::<_, i64>(11)? as u64,
        borrowed_from: r.get(12)?,
    })
}
