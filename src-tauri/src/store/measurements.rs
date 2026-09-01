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
    /// What the material actually is (T434). All of it was read and discarded until now.
    ///
    /// `None` on a row written before these were kept — which is "not known", and lending
    /// treats not knowing as a reason to refuse. A measurement made before the columns
    /// existed says nothing whatever about the material it was made on.
    pub material: Option<Material>,
    pub native_height: Option<u32>,
    pub anchor_mbps: u64,
    pub chunk_starts: Vec<u64>,
    pub chunk_s: u64,
    /// Which file this measurement really came from, when it was not made here.
    ///
    /// **The file the points were encoded on, never the one they came through.** A lends to
    /// B and B lends to C: every point C holds was made on A's material. Naming B would send
    /// anybody checking the ladder to a file that is itself only a copy — and if B is deleted,
    /// the trail ends at something that no longer exists (T429).
    pub borrowed_from: Option<String>,
    /// The donor's own anchor, kept beside this film's rather than in place of it (T429).
    ///
    /// `None` when nothing was borrowed. The check after a loan (T437) needs both: two
    /// anchors far apart are the first sign that the material is far apart too.
    pub donor_anchor_mbps: Option<u64>,
    /// What the film's weight-per-second looks like (T435).
    ///
    /// `None` on a row written before it was kept, and on one whose packets could not be
    /// read. Not a shape of noughts: two unknowns must not compare equal.
    pub shape: Option<crate::domain::chunks::Shape>,
}

/// What the material is, in the detail lending has to compare.
///
/// **Why the full codec name and not the boolean beside it.** `heavier_codec` answers "does
/// this carry more picture per bit than H.264", which is the question the ladder's arithmetic
/// asks. Lending was asking it too — so AV1 and VP9 were compared as though they were H.264,
/// and a measurement of an AV1 source was lent to an H.264 one as the same material (T431).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Material {
    /// The source's own codec, by name: `h264`, `hevc`, `av1`, `vp9`.
    pub codec: String,
    pub pix_fmt: String,
    /// The transfer curve. `None` where the file does not say, which is not the same as
    /// knowing it is `bt709`.
    pub color_transfer: Option<String>,
    pub duration_s: f64,
    pub peak_bps: Option<u64>,
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
/// **The two `COALESCE`s are T430, and they are the whole of it.** A measurement starting up
/// builds its run afresh, and a fresh run knows nothing about any loan — so its
/// `borrowed_from` is `None`. Taking that would erase the mark; the task would then skip every
/// cell, because a loan had already filled them, and a run that measured nothing at all would
/// come out of it saying it was measured here. Every rung resting on somebody else's points
/// would say the same.
///
/// Clearing the mark deliberately is `forget`, which throws the points away with it. There is
/// no case for clearing it while keeping them.
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
                 anchor_mbps, chunk_starts, chunk_s, borrowed_from,
                 donor_anchor_mbps, source_codec, pix_fmt, color_transfer,
                 duration_s, peak_bps, shape_median_bps, shape_p90_bps, shape_peak_bps,
                 shape_peak_to_median_x100, shape_walls, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                     ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25)
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
                borrowed_from = COALESCE(excluded.borrowed_from, quality_measurements.borrowed_from),
                donor_anchor_mbps =
                    COALESCE(excluded.donor_anchor_mbps, quality_measurements.donor_anchor_mbps),
                source_codec = COALESCE(excluded.source_codec, quality_measurements.source_codec),
                pix_fmt = COALESCE(excluded.pix_fmt, quality_measurements.pix_fmt),
                color_transfer =
                    COALESCE(excluded.color_transfer, quality_measurements.color_transfer),
                duration_s = COALESCE(excluded.duration_s, quality_measurements.duration_s),
                peak_bps = COALESCE(excluded.peak_bps, quality_measurements.peak_bps),
                shape_median_bps =
                    COALESCE(excluded.shape_median_bps, quality_measurements.shape_median_bps),
                shape_p90_bps =
                    COALESCE(excluded.shape_p90_bps, quality_measurements.shape_p90_bps),
                shape_peak_bps =
                    COALESCE(excluded.shape_peak_bps, quality_measurements.shape_peak_bps),
                shape_peak_to_median_x100 = COALESCE(
                    excluded.shape_peak_to_median_x100,
                    quality_measurements.shape_peak_to_median_x100
                ),
                shape_walls = COALESCE(excluded.shape_walls, quality_measurements.shape_walls),
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
                run.donor_anchor_mbps.map(|v| v as i64),
                run.material.as_ref().map(|m| m.codec.clone()),
                run.material.as_ref().map(|m| m.pix_fmt.clone()),
                run.material.as_ref().and_then(|m| m.color_transfer.clone()),
                run.material.as_ref().map(|m| m.duration_s),
                run.material.as_ref().and_then(|m| m.peak_bps).map(|v| v as i64),
                run.shape.map(|s| s.median_bps as i64),
                run.shape.map(|s| s.p90_bps as i64),
                run.shape.map(|s| s.peak_bps as i64),
                run.shape.map(|s| s.peak_to_median_x100 as i64),
                run.shape.map(|s| s.walls as i64),
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
pub fn machine_factor(db: &Db) -> Result<Option<MachineSpeed>, DbError> {
    db.with_conn(|c| {
        let mut q = c.prepare(
            "SELECT p.took_ms, m.width, m.height, m.fps, m.chunk_s, m.chunk_starts
             FROM quality_points p
             JOIN quality_measurements m
               ON m.source_key = p.source_key AND m.codec = p.codec
             WHERE p.took_ms > 0
             ORDER BY p.measured_at DESC LIMIT ?1",
        )?;
        let mut seen: Vec<(f64, f64)> = q
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
                .map(|expected| {
                    let took = took_ms as f64 / 1000.0;
                    (took / expected.max(0.001), took)
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        if seen.is_empty() {
            return Ok(None);
        }
        let counted = seen.len();
        // Two middles rather than one, and taken separately on purpose. The factor is what
        // corrects an estimate for a film of any size; the seconds are what a person
        // recognises — "my points were running at about forty seconds each". The point
        // whose ratio is the middle one is not necessarily the point whose duration is.
        let mut ratios: Vec<f64> = seen.iter().map(|(r, _)| *r).collect();
        let mut seconds: Vec<f64> = seen.iter().map(|(_, s)| *s).collect();
        seen.clear();
        ratios.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        seconds.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        Ok(Some(MachineSpeed {
            factor: ratios[counted / 2],
            points: counted,
            seconds_per_point: seconds[counted / 2],
        }))
    })
}

/// How this machine has actually behaved, and on how much evidence.
///
/// Kept together rather than as a pair of loose numbers: they are only meaningful in each
/// other's company. A factor with no count behind it cannot be told from a guess, and a
/// count with no factor says nothing about how long anything will take.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MachineSpeed {
    /// 1.0 means the machine behaves as the cost model says; 3.0, three times slower.
    pub factor: f64,
    /// How many timed points it rests on.
    pub points: usize,
    /// What a point actually took here, in seconds — the middle value of the same points.
    pub seconds_per_point: f64,
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
                    anchor_mbps, chunk_starts, chunk_s, borrowed_from,
                    donor_anchor_mbps, source_codec, pix_fmt, color_transfer,
                    duration_s, peak_bps, shape_median_bps, shape_p90_bps, shape_peak_bps,
                    shape_peak_to_median_x100, shape_walls
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
                    anchor_mbps, chunk_starts, chunk_s, borrowed_from,
                    donor_anchor_mbps, source_codec, pix_fmt, color_transfer,
                    duration_s, peak_bps, shape_median_bps, shape_p90_bps, shape_peak_bps,
                    shape_peak_to_median_x100, shape_walls
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
    /// The two are not the same kind of material, and which field said so.
    ///
    /// A ladder measured on a native 4K master says nothing about an upscale, and the other
    /// way round: the point where the resolution should drop sits somewhere else entirely.
    ///
    /// **It carries what differed** (T431). "Different material" on its own leaves a person
    /// comparing two files by eye to work out which of eight things this application looked
    /// at — and the answer is usually one they can see at a glance once they are told where.
    DifferentMaterial(Mismatch),
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
    if let Some(why) = differs(&source, to) {
        return Err(LendRefusal::DifferentMaterial(why));
    }

    let borrowed = Run {
        // The borrower's own anchor stays. It is the one number measured on this material,
        // and the donor's goes beside it rather than over it (T429).
        chunk_starts: source.chunk_starts.clone(),
        chunk_s: source.chunk_s,
        // Through the middleman to the source. `source.borrowed_from` is already the true
        // origin when the donor is itself a borrower, so following it once is enough — a
        // chain cannot be longer than that, because each loan writes the origin and not the
        // step.
        borrowed_from: Some(
            source
                .borrowed_from
                .clone()
                .unwrap_or_else(|| source.source_path.clone()),
        ),
        donor_anchor_mbps: Some(source.anchor_mbps),
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

/// What stops one measurement being lent to another film, or `None` when nothing does.
///
/// **Five fields used to be the whole test, and one of them was a boolean.** Width, height,
/// frame rate, the declared native height, and "is this HEVC" — so a measurement of an AV1
/// source was lent to an H.264 one as the same material, and to a VP9 one, and both were
/// wrong in the same way: the codec decides how much picture a bit buys, which is the entire
/// question a measurement answers (T431).
///
/// Separate from `lend` and returning what differed, so that the refusal can say which field
/// it was — and so that this can be checked without a database.
pub fn differs(source: &Run, to: &Run) -> Option<Mismatch> {
    if source.width != to.width || source.height != to.height {
        return Some(Mismatch::Frame);
    }
    if source.fps != to.fps {
        return Some(Mismatch::Fps);
    }
    if source.native_height != to.native_height {
        return Some(Mismatch::NativeHeight);
    }

    // **Not knowing is a refusal, not a pass.** A row written before the material was kept
    // says nothing about what it was measured on, and lending it would be vouching for
    // something nobody looked at. Measuring again costs half an hour; a ladder built on the
    // wrong material costs the encode and the viewer.
    let (Some(from), Some(onto)) = (source.material.as_ref(), to.material.as_ref()) else {
        return Some(Mismatch::NotKnown);
    };

    if !from.codec.eq_ignore_ascii_case(&onto.codec) {
        return Some(Mismatch::Codec);
    }
    if from.pix_fmt != onto.pix_fmt {
        return Some(Mismatch::PixelFormat);
    }
    if from.color_transfer != onto.color_transfer {
        return Some(Mismatch::ColourTransfer);
    }

    // **The length, and by a rule rather than a threshold.** The borrower is measured on the
    // donor's chunk positions, so those positions have to exist in it — an episode that ends
    // before the donor's last chunk begins would be measured on nothing at all, and on one
    // shorter still the chunks would fall in whatever scene happened to be there. No invented
    // tolerance: either the film covers the chunks or it does not.
    let last = source
        .chunk_starts
        .iter()
        .copied()
        .max()
        .unwrap_or(0)
        .saturating_add(source.chunk_s);
    if onto.duration_s < last as f64 {
        return Some(Mismatch::TooShort);
    }
    None
}

/// Why a measurement cannot be lent to this film.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mismatch {
    Frame,
    Fps,
    NativeHeight,
    /// The codec's own name, not "is it heavier than H.264".
    Codec,
    PixelFormat,
    ColourTransfer,
    /// The film ends before the donor's last reference chunk begins.
    TooShort,
    /// One of them was measured before the material was written down at all.
    NotKnown,
}

impl Mismatch {
    /// The code a person's language words this by.
    pub fn code(self) -> crate::domain::wording::DetailCode {
        use crate::domain::wording::DetailCode as D;
        match self {
            Self::Frame => D::LendFrameDiffers,
            Self::Fps => D::LendFpsDiffers,
            Self::NativeHeight => D::LendNativeHeightDiffers,
            Self::Codec => D::LendCodecDiffers,
            Self::PixelFormat => D::LendPixelFormatDiffers,
            Self::ColourTransfer => D::LendColourTransferDiffers,
            Self::TooShort => D::LendTooShort,
            Self::NotKnown => D::LendMaterialNotKnown,
        }
    }
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
        // By name: this reads `SELECT *`, and a column added by a later migration lands after
        // the ones counted here. Two more positional indexes would be two more chances for a
        // future column to silently shift what is read into what.
        // **By name, and the failure is passed on rather than swallowed.** `.ok().flatten()`
        // stood here for one run of the tests, and it turned "the query did not ask for this
        // column" into "the value is absent" — so the material read back as unknown, lending
        // refused everything, and nothing pointed at the query that had forgotten to select
        // it. A column this cannot find is a fault in the caller, and it says so.
        donor_anchor_mbps: r
            .get::<_, Option<i64>>("donor_anchor_mbps")?
            .map(|v| v as u64),
        // All or nothing: a row that knows its codec but not its pixel format is a row from
        // an interrupted write, and half a description of the material is worse than none —
        // it would let lending vouch for what it cannot see.
        material: match (
            r.get::<_, Option<String>>("source_codec")?,
            r.get::<_, Option<String>>("pix_fmt")?,
            r.get::<_, Option<f64>>("duration_s")?,
        ) {
            (Some(codec), Some(pix_fmt), Some(duration_s)) => Some(Material {
                codec,
                pix_fmt,
                color_transfer: r.get::<_, Option<String>>("color_transfer")?,
                duration_s,
                peak_bps: r.get::<_, Option<i64>>("peak_bps")?.map(|v| v as u64),
            }),
            _ => None,
        },
        // All five or nothing, for the same reason as the material above: half a shape is
        // worse than none, because a comparison would trust it.
        shape: match (
            r.get::<_, Option<i64>>("shape_median_bps")?,
            r.get::<_, Option<i64>>("shape_p90_bps")?,
            r.get::<_, Option<i64>>("shape_peak_bps")?,
            r.get::<_, Option<i64>>("shape_peak_to_median_x100")?,
            r.get::<_, Option<i64>>("shape_walls")?,
        ) {
            (Some(median), Some(p90), Some(peak), Some(ratio), Some(walls)) => {
                Some(crate::domain::chunks::Shape {
                    median_bps: median as u64,
                    p90_bps: p90 as u64,
                    peak_bps: peak as u64,
                    peak_to_median_x100: ratio as u64,
                    walls: walls as u64,
                })
            }
            _ => None,
        },
    })
}
