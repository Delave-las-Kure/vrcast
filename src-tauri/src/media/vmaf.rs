//! T233, T234 — measuring what one point of the grid is actually worth.
//!
//! One point is one bitrate at one height. Measuring it means encoding the three reference
//! chunks the way the application would really encode them, and comparing each against the
//! source it came from.
//!
//! **The comparison is made after stretching the result back to the source's own size.**
//! Without that, a rung encoded at 1080 would be compared against a 1080 reference and
//! score beautifully — it would be judged against itself rather than against the film, and
//! every low rung would win. What a viewer sees is a small picture stretched over their
//! whole screen, and that is what is scored here.
//!
//! Carried over from `.claude/skills/vrcast-convert/scripts/measure-ladder.sh` without
//! changing the arithmetic (constitution VI).

use std::path::{Path, PathBuf};

use tokio_util::sync::CancellationToken;

use super::encoders::Encoder;
use super::ffmpeg;
use crate::domain::measure_grid::Cell;
use crate::domain::measured_ladder::Point;
use crate::tasks::process::ManagedProcess;

/// How many threads the scoring may use.
pub const VMAF_THREADS: u32 = 8;

/// What a measured chunk is muxed into.
///
/// See [`chunk_args`] for the measurement that settled this. Not a matter of taste: the same
/// bytes score twenty-three VMAF lower read back out of the mp4 this used to write.
pub const MEASURE_CONTAINER: &str = "matroska";

/// How a measured chunk is encoded and compared — **the production recipe** (T697, QA-26
/// no. 5).
///
/// The premise of every number here is that what is measured is what will be made. It was
/// not: the measurement used a keyframe every 48 frames (once a second on 48-frame material,
/// once every two on 24-frame) and a ceiling in whole megabits (at 1 Mbit/s it let 2 through,
/// where production allows 1.1), and an HDR source was compared raw while the rung made from
/// it is brought down to the ordinary range. Now the keyframe
/// spacing is the rung's own (one a second, `ladder_build::shared_gop`), the ceiling and the
/// buffer are `convert_plan::peak_control` in kilobits, the picture is eight-bit 4:2:0 at the
/// High profile as production writes it, and an HDR source goes through the same
/// [`crate::media::convert::tonemap_chain`] on both sides of the comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recipe {
    /// Frames between keyframes: the source's frame rate, as production uses.
    pub gop: u32,
    /// The HDR curve the source is read with (zscale's name, T715), when HDR is brought down
    /// to the ordinary range — then on both sides. `None` for the ordinary range.
    pub tonemap: Option<&'static str>,
}

impl Recipe {
    /// The recipe for material of this frame rate and colour transfer.
    pub fn for_material(fps: u32, color_transfer: Option<&str>) -> Self {
        Self {
            gop: fps.max(1),
            tonemap: crate::domain::source::is_hdr_transfer(color_transfer)
                .then(|| super::convert::hdr_input_transfer(color_transfer)),
        }
    }

    /// The recipe for a stored measurement's material. A row written before the material
    /// was kept says nothing about HDR, and is measured as the ordinary range.
    pub fn of_run(run: &crate::store::measurements::Run) -> Self {
        Self::for_material(
            run.fps,
            run.material
                .as_ref()
                .and_then(|m| m.color_transfer.as_deref()),
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum VmafError {
    /// The bundled FFmpeg cannot measure quality.
    #[error("this build of FFmpeg has no libvmaf, so quality cannot be measured")]
    Unavailable,

    #[error(transparent)]
    Ffmpeg(#[from] ffmpeg::FfmpegError),

    #[error("could not run the measurement: {0}")]
    NotRunnable(String),

    /// Every chunk of this point failed. The point has no answer, not a bad answer.
    #[error("nothing could be measured at {bitrate_mbps} Mbit/s and {height}p")]
    NothingMeasured { bitrate_mbps: u64, height: u32 },

    #[error("the measurement was cancelled")]
    Cancelled,
}

/// Whether the bundled build can measure quality at all.
pub async fn available() -> Result<bool, ffmpeg::FfmpegError> {
    Ok(ffmpeg::probe_self().await?.has_libvmaf)
}

/// The peak allowed above a rung's target, in the script's integer arithmetic.
///
/// `MR=$(( BR * 11 / 10 )); [[ "$MR" -le "$BR" ]] && MR=$((BR+1))`. Below ten megabits the
/// tenth disappears in the integer division, and the guard keeps the ceiling above the
/// target rather than equal to it — a ceiling equal to the target is a constant bitrate,
/// which is not what any of this was measured with.
pub fn ceiling_mbps(bitrate_mbps: u64) -> u64 {
    let ceiling = bitrate_mbps * 11 / 10;
    if ceiling <= bitrate_mbps {
        bitrate_mbps + 1
    } else {
        ceiling
    }
}

/// What one point of the grid came out at, and on how much of the film.
///
/// **The sample is part of the answer** (R-50). A point averaged over two chunks used to be
/// indistinguishable from one averaged over three, and on 2026-09-04 that let an episode
/// which is 98% zeroes — a half-downloaded file — score 95.81 from its two surviving seconds
/// and pass the check after a loan. The number was not wrong; it described the parts of the
/// film that still existed. Nothing said so.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sampled {
    pub point: Point,
    /// How many chunks actually encoded and scored.
    pub chunks_used: usize,
    /// How many were asked for.
    pub chunks_asked: usize,
}

impl Sampled {
    /// Whether every chunk asked for landed.
    pub fn whole(&self) -> bool {
        self.chunks_used == self.chunks_asked
    }
}

/// Measure one point of the grid on the given chunks.
///
/// The result is the average over the chunks that encoded, **and how many those were**. A
/// chunk that fails is skipped rather than fatal: two chunks out of three still say something
/// about the material, while refusing the point would leave a hole in the grid and the hull
/// would step over it as if the bitrate had never been tried. What must not happen is the
/// skipping going unsaid — see [`Sampled`].
#[allow(clippy::too_many_arguments)]
pub async fn measure_point(
    source: &Path,
    source_width: u32,
    source_height: u32,
    chunk_starts: &[u64],
    chunk_s: u64,
    cell: Cell,
    encoder: &Encoder,
    recipe: &Recipe,
    cancel: &CancellationToken,
) -> Result<Sampled, VmafError> {
    let ffmpeg_bin = ffmpeg::locate("ffmpeg")?;
    let work = Workspace::make(cell)?;

    let mut scores: Vec<f64> = Vec::new();
    let mut weights: Vec<u64> = Vec::new();

    for at_s in chunk_starts {
        if cancel.is_cancelled() {
            return Err(VmafError::Cancelled);
        }

        let encoded = encode_chunk(
            &ffmpeg_bin,
            &work,
            source,
            *at_s,
            chunk_s,
            cell,
            encoder,
            (source_width, source_height),
            recipe,
        )
        .await;
        match encoded {
            Ok(()) => {}
            Err(e) => {
                tracing::debug!(at_s, ?cell, error = %e, "a chunk of this point would not encode");
                continue;
            }
        }

        if cancel.is_cancelled() {
            return Err(VmafError::Cancelled);
        }

        let score = score_chunk(
            &ffmpeg_bin,
            &work,
            source,
            *at_s,
            chunk_s,
            source_width,
            source_height,
            recipe,
        )
        .await;

        match score {
            Ok(vmaf) => {
                scores.push(vmaf);
                weights.push(
                    ffmpeg::bitrate_of(&work.encoded)
                        .await
                        .unwrap_or(cell.bitrate_mbps * 1_000_000),
                );
            }
            Err(e) => {
                tracing::debug!(at_s, ?cell, error = %e, "a chunk of this point would not score")
            }
        }
        let _ = tokio::fs::remove_file(&work.encoded).await;
    }

    if scores.is_empty() {
        return Err(VmafError::NothingMeasured {
            bitrate_mbps: cell.bitrate_mbps,
            height: cell.height,
        });
    }

    Ok(Sampled {
        point: Point {
            bitrate_mbps: cell.bitrate_mbps,
            height: cell.height,
            actual_bps: weights.iter().sum::<u64>() / weights.len() as u64,
            vmaf: scores.iter().sum::<f64>() / scores.len() as f64,
        },
        chunks_used: scores.len(),
        chunks_asked: chunk_starts.len(),
    })
}

/// A directory of our own, with the two working files named relatively inside it.
///
/// See [`ManagedProcess::spawn_in`] for why the names have to be relative.
struct Workspace {
    dir: PathBuf,
    encoded: PathBuf,
}

impl Workspace {
    const ENCODED: &'static str = "point.mkv";
    const SCORE: &'static str = "score.json";

    fn make(cell: Cell) -> Result<Self, VmafError> {
        // This directory's name belongs to this one measurement rather than being shared:
        // two calls that land on the same `(bitrate_mbps, height)` at the same moment — two
        // concurrent quality measurements reaching the same grid cell, which
        // `concurrent_heavy_tasks` (up to `MAX_HEAVY_TASKS`, `store/settings.rs`) makes
        // reachable in production, or a test binary where `cargo test` runs everything in
        // one process — must not read or write into the same `point.mkv`/`score.json`.
        // `std::process::id()` used to stand in for uniqueness here, but a process id is
        // shared by every thread in that one process, so it bought none; a UUID does (T568,
        // same fix as `probe_complexity.rs`'s staged encode, T567).
        let dir = std::env::temp_dir().join(format!(
            "vrcast-vmaf-{}-{}-{}",
            uuid::Uuid::new_v4().simple(),
            cell.bitrate_mbps,
            cell.height
        ));
        std::fs::create_dir_all(&dir).map_err(|e| VmafError::NotRunnable(e.to_string()))?;
        let encoded = dir.join(Self::ENCODED);
        Ok(Self { dir, encoded })
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Encode one chunk at this point of the grid.
#[allow(clippy::too_many_arguments)]
async fn encode_chunk(
    ffmpeg_bin: &Path,
    work: &Workspace,
    source: &Path,
    at_s: u64,
    chunk_s: u64,
    cell: Cell,
    encoder: &Encoder,
    source_size: (u32, u32),
    recipe: &Recipe,
) -> Result<(), VmafError> {
    let args = chunk_args(source, at_s, chunk_s, cell, encoder, source_size, recipe);
    run_in(ffmpeg_bin, &work.dir, &args).await
}

/// The width a picture of `source_size` has at `height`, as `scale=-2:height` makes it: the
/// aspect kept, rounded to an even number.
fn width_at(source_size: (u32, u32), height: u32) -> u32 {
    let (w, h) = source_size;
    if h == 0 {
        return w;
    }
    let exact = f64::from(w) * f64::from(height) / f64::from(h);
    ((exact / 2.0).round() as u32 * 2).max(2)
}

/// The arguments that encode one chunk, apart from the running of them.
///
/// **The container is part of the measurement, and that is measured rather than assumed.**
/// This wrote `-f mp4` until 2026-09-03. Measured that day on one chunk of Blue Eye Samurai
/// S01E01 at 6 Mbit/s: the very same encoded bytes score **75.17 as the mp4 the muxer wrote,
/// and 98.63 remuxed — stream-copied, not re-encoded — into Matroska**. The encode is sound;
/// reading the mp4 back is not. Beside the video the muxer puts a `text` data track, and what
/// libvmaf then lines up against the reference is not what was encoded.
///
/// The damage is worse than a level. Over three chunks the loss was 23.46, 7.94 and 6.49, so
/// it cancels neither between chunks nor between films. Within one chunk it holds steady
/// across bitrates (23.31, 23.59, 23.24 at 2, 4 and 12 Mbit/s), which is exactly why nobody
/// caught it: the curve still rose with bitrate and looked entirely plausible. With the level
/// pushed down some twelve points on average, `TARGET_VMAF` of 96 was all but unreachable,
/// and the top rung was settled by where the grid happened to end rather than by quality.
///
/// Matroska, because that is what every measurement in R-45 through R-48 was taken with: the
/// same cell through this path now gives 96.37 against the script's 96.38.
///
/// **And encoded by the production recipe** (T697, [`Recipe`]): the picture filtered as
/// `media::convert` filters a rung (HDR brought down the same way, then the height, then
/// eight-bit 4:2:0), the ceiling and buffer of `convert_plan::peak_control`, the High profile
/// at the level the rung would carry, and a keyframe every [`Recipe::gop`] frames.
pub fn chunk_args(
    source: &Path,
    at_s: u64,
    chunk_s: u64,
    cell: Cell,
    encoder: &Encoder,
    source_size: (u32, u32),
    recipe: &Recipe,
) -> Vec<String> {
    let target_kbps = (cell.bitrate_mbps * 1000) as u32;
    // Kilobits, +10 %, strictly above the target — production's own arithmetic. The script's
    // whole megabits (`ceiling_mbps`) let a 1 Mbit/s point peak at 2 where the rung made from
    // it may reach 1.1.
    let (maxrate_kbps, bufsize_kbps) = crate::domain::convert_plan::peak_control(target_kbps);
    let mut filter = Vec::new();
    if let Some(transfer) = recipe.tonemap {
        filter.push(super::convert::tonemap_chain(Some(transfer)));
    }
    // `-2` rather than a width of our own: the width follows the height and stays divisible
    // by two, which keeps the aspect of anamorphic and side-by-side material.
    filter.push(format!("scale=-2:{}", cell.height));
    filter.push(String::from("format=yuv420p"));
    let level = crate::domain::convert_plan::h264_level(
        width_at(source_size, cell.height),
        cell.height,
        recipe.gop,
    );

    let mut args: Vec<String> = vec![
        "-nostdin".into(),
        "-y".into(),
        "-v".into(),
        "error".into(),
        // Seeking before the input: FFmpeg jumps rather than decoding its way there.
        "-ss".into(),
        at_s.to_string(),
        "-t".into(),
        chunk_s.to_string(),
        "-i".into(),
        source.to_string_lossy().into_owned(),
        "-map".into(),
        "0:v:0".into(),
        "-vf".into(),
        filter.join(","),
        "-c:v".into(),
        encoder.ffmpeg_name().to_owned(),
    ];

    // The production profile. Measuring through a different one would answer a question
    // nobody asked: the whole premise is that what is measured here is what will be made.
    let family = super::encoder_args::family_of(encoder);
    args.extend(super::encoder_args::quality_preset(family));
    // Buffer equal to the ceiling. A larger one lets bursts through above it, which is what
    // froze viewers once: ceiling 45, buffer 60, peaks at 54.
    args.extend(super::encoder_args::bitrate_capped(
        target_kbps,
        maxrate_kbps,
        bufsize_kbps,
    ));

    let gop = recipe.gop.to_string();
    for a in [
        "-profile:v",
        "high",
        "-level",
        level,
        "-pix_fmt",
        "yuv420p",
        "-g",
        &gop,
        "-keyint_min",
        &gop,
        "-an",
        "-f",
        MEASURE_CONTAINER,
        Workspace::ENCODED,
    ] {
        args.push(a.to_owned());
    }

    args
}

/// What FFmpeg is asked in order to score one chunk — without an FFmpeg to ask.
///
/// **Lifted out for the same reason as [`chunk_args`], and after the same accident.** The
/// container the measured chunk went into was wrong for weeks; the scores came out up to
/// twenty-three VMAF low and the curve went on looking sensible, so nothing said anything
/// (T476, R-49). The comparison carries decisions of exactly that kind — which input is the
/// reference, whether the variant is stretched back up before it is judged (FR-143), which
/// scaler does the stretching, whether the two are put on one clock — and every one of them
/// changes the number without changing the shape of the answer. They lived where no check
/// could look at them until T490.
///
/// **The reference is seen as the rung will show it** (T697): an HDR source goes through the
/// same [`crate::media::convert::tonemap_chain`] the encode went through, and both sides are
/// eight-bit 4:2:0 — the picture production writes. Compared raw, an HDR reference scored the
/// tonemapping as loss, and a ten-bit one was matched against an eight-bit encode by whatever
/// conversion the filter graph chose by itself. The reference is never resized.
pub fn score_args(
    source: &Path,
    at_s: u64,
    chunk_s: u64,
    source_width: u32,
    source_height: u32,
    recipe: &Recipe,
) -> Vec<String> {
    let picture = match recipe.tonemap {
        Some(transfer) => format!(
            "{},format=yuv420p",
            super::convert::tonemap_chain(Some(transfer))
        ),
        None => String::from("format=yuv420p"),
    };
    // The reference is the source's own frames; the distorted one is stretched back up to
    // meet it. `setpts` on both puts them on the same clock — without it the two inputs
    // start at different timestamps and the filter compares frame 0 against frame 240.
    let graph = format!(
        "[0:v]{picture},setpts=PTS-STARTPTS[r];\
         [1:v]scale={source_width}:{source_height}:flags=bicubic,format=yuv420p,\
         setpts=PTS-STARTPTS[d];\
         [d][r]libvmaf=n_threads={VMAF_THREADS}:log_fmt=json:log_path={}",
        Workspace::SCORE
    );

    vec![
        "-nostdin".into(),
        "-v".into(),
        "error".into(),
        "-ss".into(),
        at_s.to_string(),
        "-t".into(),
        chunk_s.to_string(),
        "-i".into(),
        source.to_string_lossy().into_owned(),
        "-i".into(),
        Workspace::ENCODED.into(),
        "-lavfi".into(),
        graph,
        "-f".into(),
        "null".into(),
        "-".into(),
    ]
}

/// Score one encoded chunk against the source it came from.
#[allow(clippy::too_many_arguments)]
async fn score_chunk(
    ffmpeg_bin: &Path,
    work: &Workspace,
    source: &Path,
    at_s: u64,
    chunk_s: u64,
    source_width: u32,
    source_height: u32,
    recipe: &Recipe,
) -> Result<f64, VmafError> {
    let args = score_args(source, at_s, chunk_s, source_width, source_height, recipe);

    run_in(ffmpeg_bin, &work.dir, &args).await?;

    let json = tokio::fs::read_to_string(work.dir.join(Workspace::SCORE))
        .await
        .map_err(|e| VmafError::NotRunnable(e.to_string()))?;
    let score = pooled_mean(&json)?;
    let _ = tokio::fs::remove_file(work.dir.join(Workspace::SCORE)).await;
    Ok(score)
}

/// The one number out of libvmaf's report.
///
/// Kept apart from the running so that it can be checked without an encoder — continuous
/// integration has neither a graphics card nor a film.
pub fn pooled_mean(json: &str) -> Result<f64, VmafError> {
    let parsed: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| VmafError::Ffmpeg(ffmpeg::FfmpegError::Unexpected(e.to_string())))?;
    parsed
        .get("pooled_metrics")
        .and_then(|m| m.get("vmaf"))
        .and_then(|v| v.get("mean"))
        .and_then(|m| m.as_f64())
        .ok_or_else(|| {
            VmafError::Ffmpeg(ffmpeg::FfmpegError::Unexpected(String::from(
                "the quality report has no pooled mean in it",
            )))
        })
}

/// Run FFmpeg in the workspace and wait for it.
async fn run_in(ffmpeg_bin: &Path, dir: &Path, args: &[String]) -> Result<(), VmafError> {
    let mut child = ManagedProcess::spawn_in(Some(dir), &ffmpeg_bin.to_string_lossy(), args)
        .map_err(|e| VmafError::NotRunnable(e.to_string()))?;
    let (_stdout, stderr) = child.take_output();

    let complaints = tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut text = String::new();
        if let Some(mut stderr) = stderr {
            let _ = stderr.read_to_string(&mut text).await;
        }
        text
    });

    let status = child
        .wait()
        .await
        .map_err(|e| VmafError::NotRunnable(e.to_string()))?;
    let said = complaints.await.unwrap_or_default();

    if status.success() {
        Ok(())
    } else {
        Err(VmafError::Ffmpeg(ffmpeg::FfmpegError::Unexpected(
            said.trim().to_owned(),
        )))
    }
}
