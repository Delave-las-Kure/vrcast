//! T194, T198 — building a ladder: preparing each variant, sending it, cutting it, and
//! asking whether it is served.
//!
//! **It refuses before it starts if the rungs were not measured** (FR-141). Building is
//! hours of encoding and gigabytes on somebody's server; doing that on the strength of a
//! formula would make a guess permanent and expensive at once.
//!
//! **What is already done is recognised by what is there, not by a note kept here**
//! (FR-048). A note outlives the thing it describes: an interrupted build that wrote "the
//! top rung is ready" and then lost the file would skip it forever, and the ladder would go
//! out with a hole in it that nothing ever looks for again.

use std::path::Path;

use crate::domain::hls_master::{self, Variant};
use crate::domain::hls_package::{ToCut, SEGMENT_SECONDS};
use crate::domain::ladder::{self, NotBuildable, Rung};
use crate::domain::ladder_build::{self, VariantWork};
use crate::domain::ladder_size;
use crate::domain::source::SourceFile;
use crate::domain::wording::{Detail, DetailCode};
use crate::media::encoders::Encoder;
use crate::server::{hls_package::Cutting, hls_verify};
use crate::ssh::Connection;
use crate::tasks::engine::TaskContext;

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("this ladder may not be built: {0:?}")]
    NotBuildable(NotBuildable),

    #[error("the ladder was built but is not served whole: {}", .0.join(", "))]
    Incomplete(Vec<String>),

    /// The set will not fit, and that was worked out before a byte of it was made.
    ///
    /// **A bar, not a warning.** Room does not appear out of consent, and a set that runs
    /// into the end of the disk halfway leaves variants of the first rungs being served and
    /// the next one half written — the state hardest to reason about from the outside.
    #[error("the set needs {needed} bytes and {free} are free, short by {short_by}")]
    NotEnoughSpace {
        needed: u64,
        free: u64,
        short_by: u64,
        rungs: usize,
    },

    #[error("the build was cancelled")]
    Cancelled,

    #[error(transparent)]
    Ssh(#[from] crate::ssh::SshError),

    #[error("preparing a variant failed: {0}")]
    Prepare(String),

    #[error(transparent)]
    Ffmpeg(#[from] crate::media::ffmpeg::FfmpegError),

    #[error("the serving could not be reached: {0}")]
    Unreachable(String),
}

/// What is being built.
pub struct BuildJob<'a> {
    pub conn: &'a Connection,
    pub video_dir: &'a str,
    /// `user:group` the finished files belong to.
    pub owner: &'a str,
    /// The media's own name in the library, and its directory on the server.
    pub slug: &'a str,
    pub source: &'a SourceFile,
    pub rungs: &'a [Rung],
    pub encoder: &'a Encoder,
    pub audio_track: usize,
    /// Where a viewer would open the finished set.
    pub master_url: &'a str,
    /// Somewhere local to put a variant while it is being prepared.
    pub work_dir: &'a Path,
}

/// What came of it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Built {
    pub master_path: String,
    pub variants: Vec<String>,
    /// How many variants were prepared here, as against found already done.
    pub prepared: usize,
    pub reused: usize,
    pub verdict: hls_verify::LadderVerdict,
    pub notices: Vec<Detail>,
}

/// Build the ladder.
pub async fn run(job: &BuildJob<'_>, ctx: &TaskContext) -> Result<Built, BuildError> {
    ladder::buildable(job.rungs).map_err(BuildError::NotBuildable)?;

    // How the source's own keyframes sit decides whether a rung may be carried across
    // untouched. Not knowing means not copying — see `ladder_build::work_for`.
    let spacing = crate::media::keyframes::spacing_s(Path::new(&job.source.path))
        .await
        .unwrap_or(None);

    let work = ladder_build::work_for(
        job.slug,
        job.rungs,
        job.source,
        job.audio_track,
        spacing,
        SEGMENT_SECONDS,
    );

    let mut notices: Vec<Detail> = work.iter().flat_map(|w| w.notices.clone()).collect();

    // **Will it fit?** Asked once, here, before a byte is encoded. A set is hours of work
    // and tens of gigabytes; running into the end of the disk halfway leaves the first
    // rungs being served, the next one half written, and a person with no idea which is
    // which.
    if let Some(unknown) = room_for_the_set(job, &work).await? {
        notices.push(unknown);
    }

    let mut prepared = 0usize;
    let mut reused = 0usize;

    for (done, variant) in work.iter().enumerate() {
        ctx.bail_if_cancelled().map_err(|_| BuildError::Cancelled)?;
        ctx.wait_while_paused().await;
        ctx.report(
            done as f64 / (work.len() as f64 + 1.0),
            DetailCode::StageBuildingLadder,
        );

        // Already on the server, whole? Then it is done, and asking the server is the only
        // way to know that is still true.
        if variant_already_there(
            job.conn,
            job.video_dir,
            &variant.file,
            job.source.duration_s,
        )
        .await?
        {
            reused += 1;
            continue;
        }
        // What preparing this variant had to say — the graphics card refusing and the work
        // going to the processor, for instance. Collected rather than dropped: a fallback
        // nobody is told about is a slower build with no explanation for why (T464).
        notices.extend(prepare_and_send(job, variant, ctx).await?);
        prepared += 1;
    }

    // The cutting resumes by itself: a variant already cut whole is left alone.
    ctx.report(
        work.len() as f64 / (work.len() as f64 + 1.0),
        DetailCode::StageCuttingSegments,
    );
    let to_cut: Vec<ToCut> = work
        .iter()
        .map(|w| ToCut {
            sub: w.sub.clone(),
            file: w.file.clone(),
        })
        .collect();
    let cutting = Cutting {
        conn: job.conn,
        video_dir: job.video_dir,
        owner: job.owner,
        base: job.slug,
        variants: &to_cut,
    };
    let facts = cutting.run(|_| {}).await?;

    // The description is built from what the cutting reported — the segments' own numbers,
    // not an estimate of them.
    let variants: Vec<Variant> = facts
        .iter()
        .map(|f| Variant {
            path: format!("{}/stream.m3u8", f.sub),
            bandwidth: hls_master::peak_bps(&f.segments),
            average_bandwidth: hls_master::average_bps(&f.segments),
            width: f.width,
            height: f.height,
            fps: f.frame_rate.parse().ok(),
            codecs: hls_master::codecs_for(&level_as_written(&f.level)),
        })
        .collect();
    let master_path = format!(
        "{}/{}/master.m3u8",
        job.video_dir.trim_end_matches('/'),
        job.slug
    );
    write_master(job.conn, &master_path, &hls_master::build(&variants)).await?;
    cutting.tidy_up().await?;

    // And the only question that decides whether this was a success.
    ctx.report_important(0.99, DetailCode::StageVerifyingLadder);
    let verdict = hls_verify::verify(job.master_url, work.len())
        .await
        .map_err(|e| BuildError::Unreachable(e.to_string()))?;
    if !verdict.ok() {
        return Err(BuildError::Incomplete(verdict.broken()));
    }

    if reused > 0 {
        notices.push(Detail::new(DetailCode::NoticeVariantsReused).with("count", reused as u64));
    }
    ctx.report_important(1.0, DetailCode::StageDone);

    Ok(Built {
        master_path,
        variants: work.iter().map(|w| w.sub.clone()).collect(),
        prepared,
        reused,
        verdict,
        notices,
    })
}

/// Whether a variant's prepared file is already on the server, whole.
///
/// **Whole, not merely present.** An interrupted transfer leaves a file of the right name
/// and the wrong length, and treating that as done is how a ladder ends up with a variant
/// that plays for ninety seconds and stops. So the server is asked how long the film in it
/// actually is.
///
/// Takes its parts rather than the whole job so that it can be checked against a real
/// server on its own: what is uncertain here is how the shell behaves when the file is not
/// there, and that is not something to reason about.
/// Refuse a set that will not fit, before any of it is made.
///
/// **A bar, not a warning**: room does not appear out of consent. The upload path keeps the
/// same distinction and for the same reason (`commands::upload::space_error`).
///
/// What is already on the server is credited at what it actually weighs, in one listing
/// rather than a round trip per variant. Without that, rebuilding a set to change one rung
/// would be judged as though the whole set had to be made again, and refused on a disk that
/// had room for it all along.
/// Public so that the guard can be asked directly — by a check, and one day by a screen that
/// wants to say "this will not fit" before a person presses anything. A refusal reachable
/// only from inside an hours-long task is a refusal that can be checked only by running one.
///
/// `Ok(None)` — it fits. `Ok(Some(notice))` — it could not be worked out, and the notice says
/// so. `Err` — it will not fit.
pub async fn room_for_the_set(
    job: &BuildJob<'_>,
    work: &[VariantWork],
) -> Result<Option<Detail>, BuildError> {
    // What the audio will weigh. A re-encoded track is held to the budget; a copied one is
    // whatever the source carries, and a multichannel track carries far more. Where the
    // source does not say, the budget is the floor rather than the answer — guessing low
    // here is guessing in the one direction this check exists to avoid.
    let audio_bps = job
        .source
        .audio_tracks
        .get(job.audio_track)
        .and_then(|t| t.bitrate_bps)
        .unwrap_or(ladder_size::AUDIO_BUDGET_BPS)
        .max(ladder_size::AUDIO_BUDGET_BPS);

    let bitrates: Vec<u64> = work.iter().map(|w| w.rung.bitrate_bps).collect();
    let needed = ladder_size::bytes_for_set(&bitrates, audio_bps, job.source.duration_s);
    if needed == 0 {
        // The source's length is unknown, so there is nothing to reckon with. Said out loud:
        // a check that could not run must not look like one that ran and was content.
        return Ok(Some(Detail::new(DetailCode::LadderSpaceUnknown)));
    }

    let disk = match crate::server::disk::usage(job.conn, job.video_dir).await {
        Ok(disk) => disk,
        Err(e) => {
            // The server would not say. That is not a reason to refuse hours of work —
            // it is a reason to say the check did not happen.
            tracing::warn!(error = %e, "could not read the free space before building a set");
            return Ok(Some(Detail::new(DetailCode::LadderSpaceUnknown)));
        }
    };

    let already: u64 = match crate::server::listing::list(job.conn, job.video_dir).await {
        Ok(entries) => entries
            .iter()
            .filter(|e| work.iter().any(|w| w.file == e.name))
            .map(|e| e.size_bytes)
            .sum(),
        // No credit rather than a wrong one: over-counting what is needed refuses a build
        // that would have fitted, and that costs a person one look at the number.
        Err(_) => 0,
    };

    match crate::server::free_space::check(&disk, needed, already) {
        crate::server::free_space::SpaceVerdict::Fits => Ok(None),
        crate::server::free_space::SpaceVerdict::NotEnough {
            needed,
            free,
            short_by,
        } => Err(BuildError::NotEnoughSpace {
            needed,
            free,
            short_by,
            rungs: work.len(),
        }),
    }
}

pub async fn variant_already_there(
    conn: &Connection,
    video_dir: &str,
    file: &str,
    expected_s: f64,
) -> Result<bool, crate::ssh::SshError> {
    let path = format!("{}/{}", video_dir.trim_end_matches('/'), file);
    let out = conn
        .exec(&format!(
            "test -f {p} && ffprobe -v error -show_entries format=duration -of csv=p=0 {p} || true",
            p = crate::server::shell_quote(&path)
        ))
        .await?;
    let duration: f64 = out.trimmed().parse().unwrap_or(0.0);
    // Within a second of the source's own length. A variant is the same film, so anything
    // else means it was cut short.
    Ok(duration > 0.0 && (duration - expected_s).abs() < 1.0)
}

/// Prepare one variant here and send it.
async fn prepare_and_send(
    job: &BuildJob<'_>,
    variant: &VariantWork,
    ctx: &TaskContext,
) -> Result<Vec<Detail>, BuildError> {
    let out_path = job.work_dir.join(&variant.file);
    std::fs::create_dir_all(job.work_dir).map_err(|e| BuildError::Prepare(e.to_string()))?;

    let convert = crate::media::convert::ConvertJob {
        source: job.source,
        plan: &variant.plan,
        encoder: job.encoder,
        out_path: &out_path.to_string_lossy(),
    };
    let said = crate::media::convert::run(&convert, ctx)
        .await
        .map_err(|e| match e {
            crate::media::convert::ConvertError::Cancelled => BuildError::Cancelled,
            other => BuildError::Prepare(other.to_string()),
        })?;

    let sent = send(job, &out_path, &variant.file).await;
    // The local copy goes whether the sending worked or not: it is gigabytes, and a failed
    // build that quietly fills somebody's disk is a second failure on top of the first.
    let _ = std::fs::remove_file(&out_path);
    sent.map(|()| said)
}

/// Send a prepared variant to the serving directory.
async fn send(job: &BuildJob<'_>, local: &Path, name: &str) -> Result<(), BuildError> {
    use tokio::io::AsyncWriteExt;

    let target = format!("{}/{}", job.video_dir.trim_end_matches('/'), name);
    let staged = format!("{target}.part");

    let body = tokio::fs::read(local)
        .await
        .map_err(|e| BuildError::Prepare(e.to_string()))?;
    let sftp = job.conn.sftp().await?;
    let written = async {
        let mut file = sftp.create(staged.clone()).await?;
        file.write_all(&body).await?;
        file.flush().await?;
        file.shutdown().await?;
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;
    if let Err(e) = written {
        let _ = sftp.remove_file(staged.clone()).await;
        return Err(BuildError::Ssh(crate::ssh::SshError::sftp(
            crate::store::redact::safe_display(&*e),
        )));
    }

    // Renamed into place only once it is all there: a reader sees either no file or the
    // whole one, never a growing one. That is also what lets `already_there` trust a file
    // it finds.
    job.conn
        .exec(&format!(
            "mv {} {}",
            crate::server::shell_quote(&staged),
            crate::server::shell_quote(&target)
        ))
        .await?
        .require_ok("could not put the variant in place")?;
    Ok(())
}

async fn write_master(conn: &Connection, path: &str, body: &str) -> Result<(), BuildError> {
    use tokio::io::AsyncWriteExt;

    let sftp = conn.sftp().await?;
    let written = async {
        let mut file = sftp.create(path.to_owned()).await?;
        file.write_all(body.as_bytes()).await?;
        file.flush().await?;
        file.shutdown().await?;
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;
    written.map_err(|e| {
        BuildError::Ssh(crate::ssh::SshError::sftp(
            crate::store::redact::safe_display(&*e),
        ))
    })
}

/// ffprobe reports a level as a number — 30 is 3.0, 51 is 5.1 — and the description wants
/// the level it stands for.
fn level_as_written(level: &str) -> String {
    match level.trim().parse::<u32>() {
        Ok(n) if n >= 10 => format!("{}.{}", n / 10, n % 10),
        _ => String::from("5.2"),
    }
}
