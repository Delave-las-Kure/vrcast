//! T235 — running a quality measurement as a task.
//!
//! **The longest thing this application does.** About half an hour on a 4K film at 48
//! frames a second, and longer without a graphics card. That length is the whole reason
//! this is a task rather than a function: it has to show where it has got to, stop when
//! asked, and pick up where it left off — including after the application has been closed
//! and opened again.
//!
//! Every point is written down the moment it is measured, so what survives a cancellation
//! is not "roughly half" but exactly the points that answered.

use std::collections::HashSet;
use std::path::Path;

use crate::domain::ladder::SourceFacts;
use crate::domain::measure_grid::{grid, Cell};
use crate::domain::measured_ladder::{select, Selection, TARGET_VMAF, VMAF_STEP};
use crate::domain::wording::{Detail, DetailCode};
use crate::media::encoders::Encoder;
use crate::media::vmaf::{self, VmafError};
use crate::store::db::Db;
use crate::store::measurements::{self, Run};
use crate::tasks::engine::TaskContext;

#[derive(Debug, thiserror::Error)]
pub enum MeasureError {
    #[error("this build of FFmpeg cannot measure quality")]
    Unavailable,

    #[error("the measurement was cancelled")]
    Cancelled,

    /// Not one point of the grid would encode.
    ///
    /// Distinct from a partial measurement on purpose: some points failing still leaves a
    /// hull to choose from, whereas none leaves nothing to choose between and must not be
    /// dressed up as a ladder.
    #[error("no point of the grid could be measured")]
    NothingMeasured,

    #[error(transparent)]
    Vmaf(#[from] VmafError),

    #[error(transparent)]
    Db(#[from] crate::store::db::DbError),
}

/// What is to be measured.
pub struct MeasureJob<'a> {
    pub source: &'a Path,
    pub run: &'a Run,
    pub encoder: &'a Encoder,
    pub db: &'a Db,
}

/// What came of it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Outcome {
    pub selection: Selection,
    /// How many points of the grid answered, and how many there were.
    pub measured: usize,
    pub total: usize,
    /// What has to be said about this result, if anything.
    pub notices: Vec<Detail>,
}

/// Measure the grid, and choose a ladder from what it says.
pub async fn run(job: &MeasureJob<'_>, ctx: &TaskContext) -> Result<Outcome, MeasureError> {
    // Asked before a single frame is encoded. Finding this out half an hour in would be
    // finding it out at the worst possible moment.
    if !vmaf::available().await.unwrap_or(false) {
        return Err(MeasureError::Unavailable);
    }

    measurements::begin(job.db, job.run)?;

    let facts = SourceFacts {
        width: job.run.width,
        height: job.run.height,
        fps: job.run.fps,
        // Not used by the grid, which works from the anchor and the frame; a real value
        // would only invite somebody to believe the grid consults it.
        bitrate_bps: 0,
        heavier_codec: false,
        native_height: job.run.native_height,
    };
    let cells = grid(&facts, job.run.anchor_mbps);
    let total = cells.len();

    // What a previous run already answered. **Resumption is read from the points
    // themselves, not from a note about how far it got**: a note can outlive the thing it
    // describes, and then a grid would be declared complete with holes in it.
    let already: HashSet<(u64, u32)> =
        measurements::points(job.db, &job.run.source_key, &job.run.codec)?
            .iter()
            .map(|p| (p.bitrate_mbps, p.height))
            .collect();

    let mut done = already.len();
    report(ctx, done, total);

    for cell in cells {
        if already.contains(&(cell.bitrate_mbps, cell.height)) {
            continue;
        }
        if ctx.is_cancelled() {
            return Err(MeasureError::Cancelled);
        }
        ctx.wait_while_paused().await;

        match measure_one(job, cell, ctx).await {
            Ok(point) => {
                measurements::record(job.db, &job.run.source_key, &job.run.codec, &point)?;
                done += 1;
            }
            Err(VmafError::Cancelled) => return Err(MeasureError::Cancelled),
            Err(VmafError::Unavailable) => return Err(MeasureError::Unavailable),
            Err(e) => {
                // A point that will not encode is a hole in the grid, not the end of the
                // measurement: the hull steps over it and the rest still answers.
                tracing::warn!(?cell, error = %e, "this point of the grid would not measure");
            }
        }
        report(ctx, done, total);
    }

    let points = measurements::points(job.db, &job.run.source_key, &job.run.codec)?;
    if points.is_empty() {
        return Err(MeasureError::NothingMeasured);
    }

    let mut notices = Vec::new();
    if points.len() < total {
        notices.push(
            Detail::new(DetailCode::NoticeMeasurementPartial)
                .with("measured", points.len() as u64)
                .with("total", total as u64),
        );
    }
    if let Some(from) = &job.run.borrowed_from {
        notices.push(Detail::new(DetailCode::NoticeMeasurementBorrowed).with("from", from.clone()));
    }

    Ok(Outcome {
        selection: select(&points, TARGET_VMAF, VMAF_STEP),
        measured: points.len(),
        total,
        notices,
    })
}

async fn measure_one(
    job: &MeasureJob<'_>,
    cell: Cell,
    ctx: &TaskContext,
) -> Result<crate::domain::measured_ladder::Point, VmafError> {
    vmaf::measure_point(
        job.source,
        job.run.width,
        job.run.height,
        &job.run.chunk_starts,
        job.run.chunk_s,
        cell,
        job.encoder,
        &ctx.cancel_token(),
    )
    .await
}

/// How far along, as a share of the grid.
///
/// Every point costs about the same — the same three chunks, the same seconds of film — so
/// counting points is an honest measure of progress rather than a flattering one.
fn report(ctx: &TaskContext, done: usize, total: usize) {
    let progress = if total == 0 {
        1.0
    } else {
        done as f64 / total as f64
    };
    ctx.report(progress, DetailCode::StageMeasuringQuality);
}
