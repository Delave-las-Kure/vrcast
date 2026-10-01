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
use crate::server::hls_package::{Cutting, CuttingError};
use crate::server::hls_verify;
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

    /// A variant came out of the encoder and does not decode (principle II, T499).
    ///
    /// **Named separately from a failure to prepare**, because it is a different afternoon:
    /// the encoder said it had finished, and the file it finished is broken. The decoder's own
    /// words are carried — "Invalid NAL unit size" is cryptic and searchable, and "the file is
    /// broken" is neither.
    #[error("the variant {variant} does not decode: {}", .problems.join("; "))]
    VariantBroken {
        variant: String,
        problems: Vec<String>,
    },

    /// The **local** disk cannot hold one variant (T452).
    ///
    /// Kept apart from the one above, and not merged with a "which disk" field: the two are
    /// answered differently. A full server is emptied by removing media through this
    /// application; a full local disk is the person's own housekeeping, and telling them to
    /// go and free space on the wrong machine is worse than saying nothing.
    ///
    /// One variant, not the set: they are made and sent one at a time and removed once away,
    /// so only ever one is here. Asking for the set would refuse a build that had room all
    /// along, on every small scratch disk.
    #[error("one variant needs {needed} bytes locally and {free} are free, short by {short_by}")]
    NoRoomHere {
        needed: u64,
        free: u64,
        short_by: u64,
        at: String,
    },

    #[error("the build was cancelled")]
    Cancelled,

    /// A cutting of this set is still alive on the server — left by an earlier run of this
    /// application, or started by another one — and a second one was not put beside it
    /// (T605).
    #[error("a cutting of \"{0}\" is already running on the server")]
    AlreadyCutting(String),

    /// The work ended, but the end of its processes on the server is not confirmed yet
    /// (T605). Never a final answer: [`settle_stop`] turns it into one, and only once the
    /// stop is confirmed.
    #[error(transparent)]
    StopUnconfirmed(crate::server::hls_package::CuttingError),

    #[error(transparent)]
    Ssh(#[from] crate::ssh::SshError),

    #[error("preparing a variant failed: {0}")]
    Prepare(String),

    #[error(transparent)]
    Ffmpeg(#[from] crate::media::ffmpeg::FfmpegError),

    #[error("the serving could not be reached: {0}")]
    Unreachable(String),

    /// The catalogue could not be read to ask which rung files a medium claims (T675).
    #[error("the catalogue could not be read: {0}")]
    Catalogue(String),
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
    /// Where these rungs came from, for the description (T433). A code, never the donor's
    /// path: this description is served to every viewer.
    pub provenance: hls_master::Provenance,
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
}

/// Build the ladder.
pub async fn run(job: &BuildJob<'_>, ctx: &TaskContext) -> Result<Built, BuildError> {
    ladder::buildable(job.rungs).map_err(BuildError::NotBuildable)?;

    // How the source's own keyframes sit decides whether a rung may be carried across
    // untouched. Not knowing means not copying — see `ladder_build::work_for`.
    let spacing = crate::media::keyframes::spacing_s(Path::new(&job.source.path))
        .await
        .unwrap_or(None);

    let mut work = ladder_build::work_for(
        job.slug,
        job.rungs,
        job.source,
        job.audio_track,
        spacing,
        SEGMENT_SECONDS,
    );

    // ⚠ **Said to the task as it happens, not gathered up and handed over at the end**
    // (T524). This used to be a local `Vec` that reached the outside world only through
    // `Built`, which exists only on the successful path — so any `Err` (a cancellation, a
    // refusal to prepare, a refusal to send, a refusal to cut) took every notice with it.
    // The build that fails is exactly the one whose notices explain the failure: "how much
    // room is needed could not be worked out" followed by "the build failed" is a pair, and
    // the pair never arrived together.
    //
    // `TaskContext::add_notice` is the right channel because `TaskEngine::finish` writes the
    // notices down for **every** ending — completed, failed and cancelled alike.
    for said in work.iter().flat_map(|w| w.notices.clone()) {
        ctx.add_notice(said);
    }

    // **A file somebody owns is never written over, and is not a reason to stop** (T675;
    // T677, the owner's decision of 2026-10-02). A medium's own single file is often named the
    // way a rung's prepared file is — `film_9.mp4`, the shell script's convention — and a
    // build into that medium would replace it. Asked once, before a byte is encoded: a claimed
    // file that already is this rung, whole, is taken as it is (a rebuild of the same set
    // whose rungs a person filed under the medium); otherwise the rung takes the next name
    // nobody claims (`film_9v.mp4`), and the set records it so that carrying on finds it.
    let record = name_prepared_files(job, &mut work).await?;

    // **Will it fit?** Asked once, here, before a byte is encoded. A set is hours of work
    // and tens of gigabytes; running into the end of the disk halfway leaves the first
    // rungs being served, the next one half written, and a person with no idea which is
    // which.
    if let Some(unknown) = room_for_the_set(job, &work).await? {
        ctx.add_notice(unknown);
    }
    // And the disk this machine is about to write to, which had never been asked (T452). It
    // is the one that fills first: a variant is written whole before a byte of it is sent.
    if let Some(unknown) = room_here(job, &work)? {
        ctx.add_notice(unknown);
    }
    if record {
        write_prepared_record(job, &work).await?;
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
        for said in prepare_and_send(job, variant, ctx).await? {
            ctx.add_notice(said);
        }
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
    let facts = match cutting.run(ctx, |_| {}).await {
        Ok(facts) => facts,
        Err(e) => {
            if matches!(e, CuttingError::StopUnconfirmed(_)) {
                // Said out loud while it lasts: a person who pressed "stop" and sees the task
                // still running is owed the reason, not a frozen bar.
                ctx.report_important(
                    work.len() as f64 / (work.len() as f64 + 1.0),
                    DetailCode::StageStopUnconfirmed,
                );
            }
            return Err(from_cutting(e));
        }
    };

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
    // **What this rebuild stops serving** (T468). Asked before the master is written, because
    // after it the answer is already the new one. On 2026-08-29 a set on the production server
    // was rebuilt without one of its rungs and that quality vanished for viewers; the shell
    // script loses it by deleting the directory, and this loses it by no longer naming it —
    // quieter, and so worse. The set is not widened to put it back: somebody who built 7/2/1
    // may have meant to drop 4, and quietly restoring it would overrule that. What is owed is
    // the fact, said out loud.
    if let Some(said) = left_behind(job, &work).await {
        ctx.add_notice(said);
    }

    write_master(
        job.conn,
        &master_path,
        &hls_master::build_with_provenance(&variants, job.provenance),
    )
    .await?;
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
        ctx.add_notice(Detail::new(DetailCode::NoticeVariantsReused).with("count", reused as u64));
    }
    ctx.report_important(1.0, DetailCode::StageDone);

    Ok(Built {
        master_path,
        variants: work.iter().map(|w| w.sub.clone()).collect(),
        prepared,
        reused,
        verdict,
    })
}

/// How the cutting's own ending reads as the build's.
fn from_cutting(e: CuttingError) -> BuildError {
    match e {
        CuttingError::Cancelled => BuildError::Cancelled,
        CuttingError::Ssh(inner) => BuildError::Ssh(inner),
        CuttingError::AlreadyRunning { base } => BuildError::AlreadyCutting(base),
        pending @ CuttingError::StopUnconfirmed(_) => BuildError::StopUnconfirmed(pending),
    }
}

/// Settle a build whose cutting's stop was not confirmed: try the stop again until it is,
/// and only then hand back how the build really ended (T605).
///
/// Every other outcome passes straight through. The retries are
/// [`crate::server::hls_package::PendingStop::confirm`]'s — a growing pause with a
/// ceiling, for as long as it takes, because returning before the stop is confirmed would
/// let the engine write `Cancelled` or `Failed` and release the media's directory while a
/// process of the cutting may still be writing into it (constitution III).
///
/// `attempt` is one try through a fresh connection; the caller makes it, because that is
/// where the secrets and the profile are (`commands::ladder`). Public so that the rule —
/// "does not return until confirmed" — can be checked against the real task engine
/// without a server (`tests/unit/cutting_stop.rs`).
pub async fn settle_stop<T, A, Fut>(
    outcome: Result<T, BuildError>,
    attempt: A,
) -> Result<T, BuildError>
where
    A: FnMut(crate::server::hls_package::JobMark) -> Fut,
    Fut: std::future::Future<Output = Result<crate::server::hls_package::Stopped, String>>,
{
    match outcome {
        Err(BuildError::StopUnconfirmed(CuttingError::StopUnconfirmed(pending))) => {
            Err(from_cutting(pending.confirm(attempt).await))
        }
        other => other,
    }
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
/// What is on the server, was being served, and this build is about to stop mentioning.
///
/// `None` when nothing is — which is the ordinary case, and a notice on every build would be
/// one nobody reads. `None` too when the server will not say: being unable to look is not a
/// reason to fail a build that has already done its work, and this is an aside to it.
async fn left_behind(job: &BuildJob<'_>, work: &[VariantWork]) -> Option<Detail> {
    let show = format!("{}/{}", job.video_dir.trim_end_matches('/'), job.slug);
    let entries = match crate::server::listing::list(job.conn, &show).await {
        Ok(entries) => entries,
        Err(e) => {
            tracing::warn!(error = %e, "could not read what is already in the show's directory");
            return None;
        }
    };
    let on_server: Vec<(String, bool)> = entries.into_iter().map(|e| (e.name, e.is_dir)).collect();
    let stranded = ladder_build::stranded(&on_server, work);
    if stranded.is_empty() {
        return None;
    }
    Some(
        Detail::new(DetailCode::NoticeVariantsStranded)
            .with("count", stranded.len() as u64)
            .with("names", stranded.join(", ")),
    )
}

/// Refuse a build the local disk cannot hold, before any of it is made (T452).
///
/// **The same arithmetic as the server's**, through `free_space::check` — the margin, the
/// floor under it, the naming of what is short. Two answers to the same question would drift,
/// and the day they disagreed nobody would know which to believe.
///
/// **The heaviest variant, not the set and not the first.** Only one is on this disk at a
/// time, so the set would refuse builds that had room all along; the first is not always the
/// largest once a rung has been left out, and being wrong in that direction is being wrong in
/// the one direction this check exists to avoid.
///
/// `Ok(None)` — it fits. `Ok(Some(notice))` — could not be worked out, and the notice says so
/// rather than passing for a check that ran. `Err` — it will not fit.
pub fn room_here(job: &BuildJob<'_>, work: &[VariantWork]) -> Result<Option<Detail>, BuildError> {
    let audio_bps = job
        .source
        .audio_tracks
        .get(job.audio_track)
        .and_then(|t| t.bitrate_bps)
        .unwrap_or(ladder_size::AUDIO_BUDGET_BPS)
        .max(ladder_size::AUDIO_BUDGET_BPS);

    let heaviest = work.iter().map(|w| w.rung.bitrate_bps).max().unwrap_or(0);
    let needed = ladder_size::bytes_for_rung(heaviest, audio_bps, job.source.duration_s);
    if needed == 0 {
        return Ok(Some(Detail::new(DetailCode::LadderSpaceUnknown)));
    }

    let at = std::path::Path::new(job.work_dir);
    let Some(disk) = crate::media::local_disk::usage(at) else {
        // Said out loud. A check that could not run must not look like one that ran and was
        // content — and it must not refuse either: hours of work stopped because a question
        // could not be asked is the worst of the three.
        tracing::warn!(at = %at.display(), "could not read the free space on this machine");
        return Ok(Some(Detail::new(DetailCode::LadderSpaceUnknown)));
    };

    match crate::server::free_space::check(&disk, needed, 0) {
        crate::server::free_space::SpaceVerdict::Fits => Ok(None),
        crate::server::free_space::SpaceVerdict::NotEnough {
            needed,
            free,
            short_by,
        } => Err(BuildError::NoRoomHere {
            needed,
            free,
            short_by,
            at: at.display().to_string(),
        }),
    }
}

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

/// Give each rung the prepared file it is made into (T675, T677). See [`run`] and
/// `domain::ladder_build::choose_files`.
///
/// Returns whether the set's own record of its names has to be written: a rung took a name
/// other than its first, or a record is already there (it is kept in step).
async fn name_prepared_files(
    job: &BuildJob<'_>,
    work: &mut [VariantWork],
) -> Result<bool, BuildError> {
    let manifest = crate::server::manifest_io::read(job.conn, job.video_dir)
        .await
        .map_err(|e| BuildError::Catalogue(e.to_string()))?;
    let claimed = manifest.all_claimed_paths();

    let record_path = prepared_record_path(job);
    let stored_text = job
        .conn
        .exec(&format!(
            "cat {} 2>/dev/null || true",
            crate::server::shell_quote(&record_path)
        ))
        .await?
        .stdout;
    let stored = ladder_build::parse_prepared(&stored_text);

    // A claimed file under a rung's first name that already is that rung, whole.
    let mut whole_claimed = Vec::new();
    for variant in work.iter() {
        if claimed.contains(&variant.file.as_str())
            && variant_already_there(
                job.conn,
                job.video_dir,
                &variant.file,
                job.source.duration_s,
            )
            .await?
        {
            whole_claimed.push(variant.file.clone());
        }
    }

    let names = ladder_build::choose_files(job.slug, work, &claimed, &stored, &whole_claimed);
    let mut moved = false;
    for (variant, name) in work.iter_mut().zip(names) {
        if variant.file != name {
            tracing::info!(
                rung = %variant.sub,
                first = %variant.file,
                name = %name,
                "a rung's first name belongs to a medium: it is made under another"
            );
            variant.file = name;
            moved = true;
        }
    }
    Ok(moved || !stored.is_empty())
}

fn prepared_record_path(job: &BuildJob<'_>) -> String {
    format!(
        "{}/{}/{}",
        job.video_dir.trim_end_matches('/'),
        job.slug,
        ladder_build::PREPARED_RECORD
    )
}

/// Write the set's record of its prepared files (T677), staged and renamed into place.
async fn write_prepared_record(job: &BuildJob<'_>, work: &[VariantWork]) -> Result<(), BuildError> {
    let path = prepared_record_path(job);
    let dir = format!("{}/{}", job.video_dir.trim_end_matches('/'), job.slug);
    job.conn
        .exec(&format!(
            "mkdir -p {d} && printf '%s' {body} > {p}.part && mv -f {p}.part {p}",
            d = crate::server::shell_quote(&dir),
            body = crate::server::shell_quote(&ladder_build::prepared_text(work)),
            p = crate::server::shell_quote(&path),
        ))
        .await?
        .require_ok("could not write the set's record of its prepared files")?;
    Ok(())
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
    let started = std::time::Instant::now();
    let said = crate::media::convert::run(&convert, ctx)
        .await
        .map_err(|e| match e {
            crate::media::convert::ConvertError::Cancelled => BuildError::Cancelled,
            other => BuildError::Prepare(other.to_string()),
        })?;
    // **What this machine really does, for the next plan's time** (T672). Only a real encode
    // counts — a rung carried across untouched says nothing about the encoder — and a lost
    // write costs nothing but a rougher estimate next time.
    if !variant.lossless {
        let took = started.elapsed().as_secs_f64();
        if took > 1.0 {
            let pixels = crate::domain::video::pixels_of(&variant.rung, job.source);
            if let Err(e) = crate::store::videos::record_encode_speed(
                ctx.db(),
                job.encoder.ffmpeg_name(),
                pixels / took,
            ) {
                tracing::debug!(error = %e, "the encoder's speed was not written down");
            }
        }
    }

    // ⚠ **Nothing unchecked reaches viewers** (constitution, principle II; FR-027). Between
    // the encode and the send there used to be nothing at all: `media::validate` was called
    // from the single-file path and from a command no screen calls, and every rung of every
    // ladder went to the server on the strength of ffmpeg having exited zero. A variant is not
    // a lesser file — it is what a viewer is actually served, and the whole point of the set
    // is that the player switches to it without asking anybody.
    //
    // **The cost is real and is not a reason.** A decode pass is a fraction of the encode that
    // just happened, and the alternative is a broken rung that nobody meets until a person is
    // watching. It is reported as its own stage so the time is accounted for rather than
    // looking like a stall.
    //
    // **And it answers cancel and pause while it runs** (T663): it used to be a plain call
    // that took neither, so a cancel pressed during a long decode waited for the whole film.
    ctx.report_important(0.0, DetailCode::StageValidating);
    let verdict =
        match crate::media::validate::validate_in_task(&out_path, ctx, job.source.duration_s).await
        {
            Ok(verdict) => verdict,
            Err(e) => {
                let _ = std::fs::remove_file(&out_path);
                return Err(match e {
                    crate::media::validate::ValidateError::Cancelled => BuildError::Cancelled,
                    other => BuildError::Prepare(other.to_string()),
                });
            }
        };
    if !verdict.ok {
        let _ = std::fs::remove_file(&out_path);
        return Err(BuildError::VariantBroken {
            variant: variant.file.clone(),
            problems: verdict.problems,
        });
    }

    // **Asked again before the next heavy phase** (T663). A cancel that landed as the decode
    // finished would otherwise go on to send gigabytes to the server before anything looked
    // at it again.
    if ctx.is_cancelled() {
        let _ = std::fs::remove_file(&out_path);
        return Err(BuildError::Cancelled);
    }

    let sent = send(job, &out_path, &variant.file, ctx).await;
    // The local copy goes whether the sending worked or not: it is gigabytes, and a failed
    // build that quietly fills somebody's disk is a second failure on top of the first.
    let _ = std::fs::remove_file(&out_path);
    sent.map(|()| said)
}

/// Send a prepared variant to the serving directory.
///
/// **In blocks, not whole** (T660, QA-24B-01). It used to read the entire variant into memory
/// — gigabytes, after hours of encoding — and write it in one call with no way to stop and no
/// word of how far it had got. Now it goes through [`stream_blocks`]: one [`SEND_BLOCK`] in
/// memory at a time, `STAGE_SENDING_VARIANT` with the share sent, and a cancel looked at
/// between blocks, after which the staged `.part` on the server is removed.
async fn send(
    job: &BuildJob<'_>,
    local: &Path,
    name: &str,
    ctx: &TaskContext,
) -> Result<(), BuildError> {
    let target = format!("{}/{}", job.video_dir.trim_end_matches('/'), name);
    send_file(job.conn, local, &target, ctx, PAUSE_HOLD_LIMIT).await
}

/// How long a pause may keep a variant's file session open on the server (T670(4)).
///
/// A pause pressed mid-send used to hold the SFTP session — a channel of the connection, and
/// an `sftp-server` process on the server — for as long as the pause lasted, which may be the
/// night. A minute covers the pause pressed to look at something and let go; past it the
/// session is closed, and opened again when the task carries on.
pub const PAUSE_HOLD_LIMIT: std::time::Duration = std::time::Duration::from_secs(60);

/// Send `local` to `target` on the server, staged as `target.part` and renamed into place
/// (T660), letting go of the file session during a long pause (T670(4)).
///
/// **Carrying on writes on from where it stopped, not from the start.** The staged file is
/// closed cleanly when the pause outlasts `hold_limit` — everything handed over is on the
/// server — and on carrying on it is opened again *without truncating*, its size asked, and
/// the sending goes on from that byte, the local file read from the same place. Starting the
/// rung over would throw away gigabytes for nothing. The one exception is a staged file whose
/// size is not what was sent (somebody touched it, or a write was lost): then nothing on the
/// server can be trusted and the rung is sent again from zero, into a truncated file.
///
/// A cancel during the closed pause still removes the staged `.part`, through a session
/// opened for that alone.
///
/// Public for the check against a real server, which lives in another crate (like
/// [`write_master`]).
pub async fn send_file(
    conn: &Connection,
    local: &Path,
    target: &str,
    ctx: &TaskContext,
    hold_limit: std::time::Duration,
) -> Result<(), BuildError> {
    use russh_sftp::protocol::OpenFlags;
    use tokio::io::{AsyncSeekExt, AsyncWriteExt};

    let staged = format!("{target}.part");
    let sftp_failed = |e: &(dyn std::error::Error + Send + Sync)| {
        BuildError::Ssh(crate::ssh::SshError::sftp(
            crate::store::redact::safe_display(e),
        ))
    };

    let mut source = tokio::fs::File::open(local)
        .await
        .map_err(|e| BuildError::Prepare(e.to_string()))?;
    let total = source
        .metadata()
        .await
        .map_err(|e| BuildError::Prepare(e.to_string()))?
        .len();

    ctx.report_important(0.0, DetailCode::StageSendingVariant);
    // Where the staged file stands: `None` before anything was opened, so the first opening
    // truncates whatever a previous attempt left under the same name.
    let mut resume_at: Option<u64> = None;
    loop {
        let sftp = conn.sftp().await?;
        let written = async {
            let (mut file, from) = match resume_at {
                None => (sftp.create(staged.clone()).await?, 0),
                Some(at) => {
                    let there = sftp.metadata(staged.clone()).await?.size.unwrap_or(0);
                    if there == at {
                        let mut file = sftp
                            .open_with_flags(staged.clone(), OpenFlags::WRITE)
                            .await?;
                        file.seek(std::io::SeekFrom::Start(at)).await?;
                        (file, at)
                    } else {
                        tracing::warn!(
                            expected = at,
                            found = there,
                            "the staged variant is not what was sent; sending it again whole"
                        );
                        (sftp.create(staged.clone()).await?, 0)
                    }
                }
            };
            source.seek(std::io::SeekFrom::Start(from)).await?;
            let sent =
                stream_blocks_holding(&mut source, &mut file, from, total, ctx, Some(hold_limit))
                    .await;
            if matches!(sent, Ok(_) | Err(StreamError::PausedTooLong { .. })) {
                // Closed cleanly in both cases: on a long pause what was handed over has to be
                // on the server before the session goes, or the size asked on carrying on
                // would not be the size sent.
                file.flush().await?;
                file.shutdown().await?;
            }
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(sent)
        }
        .await;

        match written {
            Ok(Ok(_)) => break,
            Ok(Err(StreamError::PausedTooLong { sent })) => {
                // Let go of the session for the rest of the pause: the channel, its place in
                // the connection's pool and the server's `sftp-server` all go with it.
                let _ = sftp.close().await;
                drop(sftp);
                tracing::info!(sent, "a long pause: the variant's file session is closed");
                ctx.wait_while_paused().await;
                if ctx.is_cancelled() {
                    if let Ok(again) = conn.sftp().await {
                        let _ = again.remove_file(staged.clone()).await;
                    }
                    return Err(BuildError::Cancelled);
                }
                resume_at = Some(sent);
            }
            Ok(Err(e)) => {
                let _ = sftp.remove_file(staged.clone()).await;
                if ctx.is_cancelled() {
                    return Err(BuildError::Cancelled);
                }
                return Err(sftp_failed(&e));
            }
            Err(e) => {
                let _ = sftp.remove_file(staged.clone()).await;
                if ctx.is_cancelled() {
                    return Err(BuildError::Cancelled);
                }
                return Err(sftp_failed(&*e));
            }
        }
    }

    // Renamed into place only once it is all there: a reader sees either no file or the
    // whole one, never a growing one. That is also what lets `already_there` trust a file
    // it finds.
    conn.exec(&format!(
        "mv {} {}",
        crate::server::shell_quote(&staged),
        crate::server::shell_quote(target)
    ))
    .await?
    .require_ok("could not put the variant in place")?;
    Ok(())
}

/// How much of a variant is held in memory at once while it is sent (T660).
///
/// A megabyte: large enough that SFTP's per-write round trips do not dominate, small enough
/// that a 20 GB rung costs the machine what a small image does.
pub const SEND_BLOCK: usize = 1024 * 1024;

/// Why a variant's sending stopped (T660).
#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    #[error("the variant could not be read: {0}")]
    Read(std::io::Error),
    #[error("the variant could not be written: {0}")]
    Write(std::io::Error),
    #[error("the sending was cancelled")]
    Cancelled,
    /// A pause outlasted the hold limit (T670(4)): `sent` bytes went across in all, counting
    /// from the start of the file, and the caller is to let go of the session.
    #[error("paused for longer than the session is held, after {sent} bytes")]
    PausedTooLong { sent: u64 },
}

/// Copy `from` into `into` one [`SEND_BLOCK`] at a time (T660).
///
/// The whole of what makes the sending bounded, stoppable and visible, apart from the SFTP
/// session it is used with — so that it can be checked against a reader of any size and a
/// writer that records what it was handed, without a server. One buffer, reused: memory does
/// not grow with the file. Between blocks it asks whether the task was cancelled and waits
/// out a pause; after each block it reports the share sent under `STAGE_SENDING_VARIANT`.
///
/// Returns how many bytes went across.
pub async fn stream_blocks<R, W>(
    from: &mut R,
    into: &mut W,
    total: u64,
    ctx: &TaskContext,
) -> Result<u64, StreamError>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    stream_blocks_holding(from, into, 0, total, ctx, None).await
}

/// The same, starting `already` bytes into the file, and giving up a pause that lasts
/// longer than `hold_limit` (T670(4)).
///
/// `already` only counts: the reader and the writer are expected to stand at that byte
/// already. With a `hold_limit`, a pause that is still on when the limit runs out ends the
/// copy with [`StreamError::PausedTooLong`] carrying the bytes sent so far (from the start of
/// the file), so the caller can close what it holds and pick up from there later. A pause let
/// go of, or a cancel, before the limit is answered here as before. Returns the bytes sent
/// in all, counting from the start of the file.
pub async fn stream_blocks_holding<R, W>(
    from: &mut R,
    into: &mut W,
    already: u64,
    total: u64,
    ctx: &TaskContext,
    hold_limit: Option<std::time::Duration>,
) -> Result<u64, StreamError>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut block = vec![0u8; SEND_BLOCK];
    let mut sent: u64 = already;
    // How fast it is going (T672), worked out the way an upload's is: over the last few
    // seconds, and forgotten across a pause rather than averaged through it.
    let mut pace = crate::domain::progress_estimate::ProgressEstimate::default();
    pace.record(std::time::Instant::now(), sent);
    loop {
        if ctx.is_cancelled() {
            return Err(StreamError::Cancelled);
        }
        if ctx.is_paused() {
            pace.reset();
        }
        match hold_limit {
            Some(limit) if ctx.is_paused() => {
                if tokio::time::timeout(limit, ctx.wait_while_paused())
                    .await
                    .is_err()
                {
                    return Err(StreamError::PausedTooLong { sent });
                }
            }
            _ => ctx.wait_while_paused().await,
        }
        if ctx.is_cancelled() {
            return Err(StreamError::Cancelled);
        }

        let n = from.read(&mut block).await.map_err(StreamError::Read)?;
        if n == 0 {
            return Ok(sent);
        }
        into.write_all(&block[..n])
            .await
            .map_err(StreamError::Write)?;
        sent += n as u64;
        if total > 0 {
            pace.record(std::time::Instant::now(), sent);
            let speed = pace.speed_bps().map(|s| s.min(i64::MAX as u64) as i64);
            let eta = pace
                .eta(total.saturating_sub(sent))
                .map(|d| d.as_secs().min(i64::MAX as u64) as i64);
            ctx.report_stage_transfer(
                (sent as f64 / total as f64).clamp(0.0, 1.0),
                DetailCode::StageSendingVariant,
                speed,
                eta,
            );
        }
    }
}

/// Write `master.m3u8` staged-and-renamed, the way [`send`] already writes a variant (T572).
///
/// **Why this had to change and `send` did not have to.** On a rebuild `master.m3u8` almost
/// always already exists and is already being served — that is the whole point of a rebuild.
/// Writing straight into it left a window, exactly as an unstaged variant write would have,
/// where a viewer's player could read a plain description mid-write and get a truncated or
/// empty file: not corrupt bytes inside a segment, but a playlist that names none of them.
/// `-f` on the final `mv` (unlike `send`'s bare `mv`) is deliberate and not an oversight:
/// `path` is expected to exist here, the same case `server::upload::publish` already uses
/// `-f` for, and `send`'s bare `mv` is right for exactly the opposite reason — there `target`
/// is ordinarily a fresh name.
///
/// **Public rather than private, like [`variant_already_there`] a little above it in this
/// file** (T529): the point being checked here only exists on a real server — what a real
/// SFTP session does when its write is cut short — and the check for it lives in the
/// integration tests, in a separate crate, which can only reach what this module exports.
pub async fn write_master(conn: &Connection, path: &str, body: &str) -> Result<(), BuildError> {
    use tokio::io::AsyncWriteExt;

    let staged = format!("{path}.part");

    let sftp = conn.sftp().await?;
    let written = async {
        let mut file = sftp.create(staged.clone()).await?;
        file.write_all(body.as_bytes()).await?;
        file.flush().await?;
        file.shutdown().await?;
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;
    if let Err(e) = written {
        // Best-effort, exactly as `send` already does: the write's own failure is what is
        // owed to the caller, and a cleanup that also fails must not bury it.
        let _ = sftp.remove_file(staged.clone()).await;
        return Err(BuildError::Ssh(crate::ssh::SshError::sftp(
            crate::store::redact::safe_display(&*e),
        )));
    }

    conn.exec(&format!(
        "mv -f {} {}",
        crate::server::shell_quote(&staged),
        crate::server::shell_quote(path)
    ))
    .await?
    .require_ok("could not put master.m3u8 in place")?;
    Ok(())
}

/// ffprobe reports a level as a number — 30 is 3.0, 51 is 5.1 — and the description wants
/// the level it stands for.
fn level_as_written(level: &str) -> String {
    match level.trim().parse::<u32>() {
        Ok(n) if n >= 10 => format!("{}.{}", n / 10, n % 10),
        _ => String::from("5.2"),
    }
}
