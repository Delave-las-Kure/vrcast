//! T193 — the three commands a ladder is worked out with (FR-040, FR-041, FR-044).
//!
//! Contract: `contracts/ipc-commands.md`, the quality ladder section.
//!
//! **Two of the three are pure functions and stay that way.** Checking runs on every edit a
//! person makes (FR-044), and a check that reached for a file — let alone a server — would
//! either lag behind the typing or stop it. What they need to know is passed in.
//!
//! The one that is not pure is [`api::ladder_plan`], and only because it has to look up
//! whether this film has been measured. A measured ladder is the real one; the formula is a
//! preview of where a measurement would look (R-21), and the answer says plainly which of
//! the two is being handed back.

use serde::{Deserialize, Serialize};

use super::error::{AppError, ErrorCode, Result};
use crate::domain::ladder::{self, Layout, NotBuildable, Objection, Plan, Rung, SourceFacts};
use crate::domain::wording::{Detail, DetailCode};
use crate::media::{encoders, ffmpeg, measure, probe_complexity};
use crate::store::measurements;
use crate::tasks::state::TaskKind;

/// What the interface sends to have a ladder worked out.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LadderRequest {
    pub path: String,
    /// The codec the ladder is for. A measurement does not carry between them.
    #[serde(default = "h264")]
    pub codec: String,
    /// The height the material really has, when it was upscaled. Told by the person.
    pub native_height: Option<u32>,
    /// What the person says the picture is, when they know better than a guess.
    pub declared_layout: Option<Layout>,
    /// The peak `ladder_measure` found on this file, when it has finished in time. Always
    /// wins over the complexity probe's own estimate when present — a full read of every
    /// packet is a better anchor than a few seconds of trial encodes (T522). `None` when
    /// the measurement has not landed yet: the probe's own estimate is the fallback.
    pub measured_peak_bps: Option<u64>,
    #[serde(default = "yes")]
    pub prefer_hardware: bool,
}

fn h264() -> String {
    String::from("h264")
}

fn yes() -> bool {
    true
}

/// What the interface sends to build a quality set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildRequest {
    pub server_id: String,
    /// The source on this machine that the variants are made from.
    pub path: String,
    /// The medium's own directory on the server.
    pub slug: String,
    /// The rungs, as the person has them on screen — measured or edited.
    pub rungs: Vec<Rung>,
    /// Which audio track to keep.
    #[serde(default)]
    pub audio_track: usize,
    #[serde(default = "yes")]
    pub prefer_hardware: bool,
    /// Which batch this build belongs to (T445). `None` for a build a person started.
    #[serde(default)]
    pub batch: Option<crate::tasks::store::Batch>,
    /// Consent to the consequences warned about before the start (T571) — the same shape as
    /// `UploadRequest.confirmed`: rewriting `master.m3u8` on a server that is serving it
    /// right now washes what active viewers were mid-stream out of the server's memory, the
    /// same category of harm `media_rename`/`media_delete`/`upload_start` already guard
    /// against for the same reason. `#[serde(default)]` so a request built before this field
    /// existed still reads — the same reasoning `UploadRequest.confirmed` documents.
    #[serde(default)]
    pub confirmed: bool,
}

/// Where a ladder's rungs came from.
///
/// Not decoration: it decides whether the ladder may be built at all (FR-141), and a person
/// looking at rungs deserves to know whether anybody has actually looked at what they are
/// worth on this film.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LadderSource {
    /// From a measurement of this material.
    Measured,
    /// From a measurement of another file, lent to this one.
    Borrowed,
    /// From the formula. A preview of where to measure, not a ladder to build.
    Formula,
}

/// A ladder as the interface receives it.
#[derive(Debug, Clone, Serialize)]
pub struct LadderPreview {
    pub plan: Plan,
    pub from: LadderSource,
    /// What the source turned out to be.
    pub source: SourceFacts,
    /// What the complexity probe found, when it ran. `None` when the ladder came from a
    /// measurement instead and the probe was not needed.
    pub anchor_mbps: Option<u64>,
    /// What the checker says about these rungs, so that an interface has it without asking
    /// again.
    pub verdict: LadderVerdict,
    /// Which codec these rungs are for. A measurement does not carry between codecs, and a
    /// screen looking one up has to ask for the same pair the plan was made from.
    pub codec: String,
    /// Which file the measurement really came from, when it was not made here (T427).
    ///
    /// A local path, and it stays local: this goes to the screen on this machine. What
    /// travels to a server passes through `redact` first (T433).
    pub borrowed_from: Option<String>,
    /// Which measurement these rungs came out of, when they came out of one (T420).
    ///
    /// Without it a screen cannot ask `quality_measure_result` what was actually measured —
    /// which is the only answer there is to "why does the top rung stop at 22 and not 35",
    /// and it is worked out and stored already. A ladder from the formula has none: nothing
    /// was measured, and that is what the provenance line says.
    pub measurement_key: Option<String>,
    pub notices: Vec<Detail>,
}

/// What a person needs to be checked on every edit (FR-044).
#[derive(Debug, Clone, Deserialize)]
pub struct LadderCheck {
    pub rungs: Vec<Rung>,
    pub source: SourceFacts,
}

/// What the interface sends after a person retypes one rung's bitrate by hand (T523).
#[derive(Debug, Clone, Deserialize)]
pub struct RecomputeRungRequest {
    pub index: usize,
    pub bitrate_bps: u64,
    pub source: SourceFacts,
}

/// Everything wrong with a ladder, in two kinds.
///
/// **The two are kept apart because they mean different things.** An objection says the
/// ladder is unsound — a rung above the source, a buffer that will let peaks through, a
/// hole a viewer would fall through. `not_buildable` says nobody has measured it. A ladder
/// can be perfectly sound and still be a guess.
#[derive(Debug, Clone, Serialize)]
pub struct LadderVerdict {
    pub objections: Vec<Objection>,
    pub not_buildable: Option<NotBuildable>,
}

impl LadderVerdict {
    fn of(rungs: &[Rung], source: &SourceFacts) -> Self {
        Self {
            objections: ladder::validate(rungs, source, source.fps),
            not_buildable: ladder::buildable(rungs).err(),
        }
    }
}

pub mod api {
    use super::*;

    /// Measure the source: what it averages, where it peaks (FR-040).
    ///
    /// Before the ladder is worked out, not after: a connection has to hold the peak rather
    /// than the average, and a film that averages 8 Mbit/s and reaches 40 in one scene
    /// freezes everyone whose line is under 40 when that scene arrives.
    pub async fn ladder_measure(path: &str) -> Result<measure::Measured> {
        measure::measure(std::path::Path::new(path))
            .await
            .map_err(|e| match e {
                ffmpeg::FfmpegError::NoVideoTrack => AppError::new(ErrorCode::InvalidInput)
                    .detail(DetailCode::ProbeNoVideo)
                    .with_cause(path),
                other => AppError::new(ErrorCode::FfmpegBroken).with_cause(other),
            })
    }

    /// Work out a ladder for this film.
    ///
    /// Hands back the measured ladder when this material has been measured, and the
    /// formula's preview when it has not — saying which, every time.
    pub async fn ladder_plan(
        state: &super::super::AppState,
        request: &LadderRequest,
    ) -> Result<LadderPreview> {
        let probed = super::super::api::source_probe(&request.path).await?;
        let source = SourceFacts {
            width: probed.width,
            height: probed.height,
            fps: probed.fps,
            bitrate_bps: probed.bitrate_bps,
            heavier_codec: probed.video_codec.eq_ignore_ascii_case("hevc"),
            native_height: request.native_height,
        };

        // A measurement of this material, if there is one. This is the ladder; everything
        // below is what happens when there is not one.
        if let Some(measured) = measured_plan(state, request, &source)? {
            return Ok(measured);
        }

        let (encoder, mut notices) = pick_encoder(request.prefer_hardware).await?;
        let probe = probe_complexity::probe(
            std::path::Path::new(&request.path),
            probed.duration_s,
            &encoder,
        )
        .await;
        notices.extend(probe.notice.clone());

        // The measured peak wins over the probe's own estimate whenever it has landed in
        // time (T522, решение владельца 2026-09-08, правило 1) — a full read of every
        // packet beats a few seconds of trial encodes. Falls back to the probe's estimate
        // when the measurement has not arrived yet, exactly as before.
        let effective_measured_bps = request.measured_peak_bps.or(probe.measured_bps);

        let plan = ladder::plan(effective_measured_bps, &source, request.declared_layout).map_err(
            |refusal| match refusal {
                ladder::Refusal::SourceBitrateTooLow { .. } => {
                    AppError::new(ErrorCode::InvalidInput).with_cause(refusal_text(refusal))
                }
            },
        )?;

        Ok(LadderPreview {
            verdict: LadderVerdict::of(&plan.rungs, &source),
            plan,
            from: LadderSource::Formula,
            source,
            anchor_mbps: effective_measured_bps.map(|bps| (bps / 1_000_000).max(1)),
            codec: request.codec.clone(),
            borrowed_from: None,
            // Nothing was measured, so there is nothing to look into.
            measurement_key: None,
            notices,
        })
    }

    /// Whether a build for this slug on this server is already running.
    ///
    /// The same gap `running_upload_for` documents and accepts, for the same reason: this is
    /// not the last line of defence, and closing it with a lock held for the whole submission
    /// costs more than the case is worth — two builds begun at the very same instant would both
    /// pass this check. What it does close is the realistic case named in T591: a repeat click
    /// before `building` reaches the UI, or a batch's own `then_build` racing a manual click —
    /// neither shares state with the other, so nothing before this stopped both from reaching
    /// `submit_in_batch` and writing the same `master.m3u8`/`v{N}` directories concurrently.
    fn running_build_for(
        state: &super::super::AppState,
        server_id: &str,
        slug: &str,
    ) -> Result<Option<String>> {
        for task in state.tasks.list()? {
            if task.kind != TaskKind::BuildLadder
                || task.state.is_final()
                || task.server_id.as_deref() != Some(server_id)
            {
                continue;
            }
            if task.resume_token.as_deref() == Some(slug) {
                return Ok(Some(task.id));
            }
        }
        Ok(None)
    }

    /// Build the set: prepare each variant, send it, cut it, and check it is served.
    ///
    /// Returns a task number at once (FR-080). Everything that can be refused quickly is
    /// refused here, before a task exists — above all an unmeasured ladder, because
    /// building one is hours of encoding spent on a guess (FR-141).
    pub async fn ladder_build(
        state: &super::super::AppState,
        request: BuildRequest,
    ) -> Result<String> {
        ladder::buildable(&request.rungs).map_err(|why| match why {
            ladder::NotBuildable::NoRungs => AppError::new(ErrorCode::InvalidInput),
            ladder::NotBuildable::RungsNotMeasured { indexes } => {
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

        // **A loan whose check has not landed is refused here** (T478). Refused before a
        // task exists, like every other quick refusal in this function: hours of encoding
        // against a ladder nobody has vouched for is exactly what the check is for, and the
        // check takes under a minute.
        if let Ok(key) = measurements::key_for(std::path::Path::new(&request.path)) {
            if let Ok(Some(run)) = measurements::run(&state.db, &key, &h264()) {
                if run.check_pending {
                    return Err(AppError::new(ErrorCode::LadderCheckPending));
                }
            }
        }

        let profile = super::super::library::api::profile_of(state, &request.server_id)?;

        // **The same guard `media_rename`/`media_delete`/`upload_start` already have, for
        // the same reason** (T571). `write_master` rewrites `master.m3u8` on the server —
        // and on a rebuild that file is very often already being served: the task 468
        // comment two functions down in `tasks::ladder_build.rs` names a real incident where
        // exactly that "quietly vanished quality for viewers" on production. Refused here,
        // before a task exists — the same place every other quick refusal in this function
        // lives, and before a single byte is encoded rather than after hours of it.
        //
        // The same approximation `media_rename` and `upload_start` already live with: what
        // is asked is how many connections the web server holds open on 80/443 at all, not
        // connections to this particular medium — `server::active_use` names why nothing
        // finer exists yet, and this is not the task to fix that.
        if !request.confirmed {
            let conn = crate::server::gate::open(
                state.secrets.as_ref(),
                &profile,
                crate::server::gate::Intent::Read,
            )
            .await?
            .conn;
            let connections = crate::server::active_use::serving_connections(&conn).await;
            conn.close().await;
            if connections > 0 {
                return Err(AppError::new(ErrorCode::FileInUse)
                    .with_cause(format!("connections={connections}")));
            }
        }

        if let Some(busy) = running_build_for(state, &request.server_id, &request.slug)? {
            return Err(AppError::new(ErrorCode::NameExists)
                .with_detail(
                    Detail::new(DetailCode::BuildAlreadyRunning).with("slug", request.slug.clone()),
                )
                .with_cause(busy));
        }

        let source = super::super::api::source_probe(&request.path).await?;
        let (encoder, _) = pick_encoder(request.prefer_hardware).await?;
        // Where these rungs came from, so the description can say it (T433). Asked of the
        // same planner the screen asked, rather than guessed from the rungs: a rung carries
        // `Quality::Borrowed`, but a set is measured, borrowed or guessed as a whole.
        let provenance = match ladder_plan(
            state,
            &LadderRequest {
                path: request.path.clone(),
                codec: h264(),
                native_height: None,
                declared_layout: None,
                // This path only reads `.from`, never `.rungs` — no measured peak to feed
                // in here, and none is needed (T522).
                measured_peak_bps: None,
                prefer_hardware: request.prefer_hardware,
            },
        )
        .await
        {
            Ok(plan) => match plan.from {
                LadderSource::Measured => crate::domain::hls_master::Provenance::Measured,
                LadderSource::Borrowed => crate::domain::hls_master::Provenance::Borrowed,
                LadderSource::Formula => crate::domain::hls_master::Provenance::Formula,
            },
            // Not being able to ask does not stop a build that is otherwise ready. The
            // honest answer then is the weakest one: nobody has shown these were measured.
            Err(_) => crate::domain::hls_master::Provenance::Formula,
        };

        let master_url = crate::domain::links::for_path(
            &profile.domain,
            None,
            &format!("{}/master.m3u8", request.slug),
        )
        .origin;
        // Somewhere local for a variant while it is being made. Beside the source unless the
        // person has said otherwise — `domain::work_dir` holds the whole argument, including
        // why it no longer goes where the comment that stood here said it should.
        let chosen = crate::store::settings::load(&state.db)
            .map(|s| s.work_dir)
            .unwrap_or_default();
        let work_dir = crate::domain::work_dir::for_source(
            chosen.as_deref(),
            std::path::Path::new(&request.path),
        );
        let secrets = state.secrets.clone();
        let db = state.db.clone();
        let events = state.events.clone();
        // Taken before `request` moves into the closure below — the marker write after
        // `submit_in_batch` needs the slug too, and `request` does not survive that move.
        let slug_for_marker = request.slug.clone();

        let task_id = state
            .tasks
            .submit_in_batch(
                TaskKind::BuildLadder,
                Some(request.server_id.clone()),
                request.batch.clone(),
                move |ctx| async move {
                    let conn = crate::server::gate::open(
                        secrets.as_ref(),
                        &profile,
                        crate::server::gate::Intent::Change,
                    )
                    .await?
                    .conn;
                    let job = crate::tasks::ladder_build::BuildJob {
                        conn: &conn,
                        video_dir: &profile.video_dir,
                        owner: &format!("{}:{}", profile.user, profile.user),
                        slug: &request.slug,
                        source: &source,
                        rungs: &request.rungs,
                        encoder: &encoder,
                        audio_track: request.audio_track,
                        master_url: &master_url,
                        provenance,
                        work_dir: &work_dir,
                    };
                    let outcome = crate::tasks::ladder_build::run(&job, &ctx).await;
                    // **The built set's own medium, found while the connection is still
                    // open** (T519(3)), and now attached to it in the catalogue itself
                    // (T528). Best effort and only on success: the slug may match no
                    // medium at all — a build run before its medium was created, or a
                    // slug T528 has not finished tidying up — and a result that pointed
                    // at nothing would be worse than none.
                    if outcome.is_ok() {
                        if let Some(media_id) =
                            attach_built_set(&conn, &profile.video_dir, &request.slug).await
                        {
                            ctx.set_result(crate::tasks::store::TaskResult { media_id });
                        }
                        // **T578 — the same signal the five mutating commands in
                        // `library.rs` already send after writing the manifest.**
                        // `run` above wrote the built set onto the server before
                        // `outcome.is_ok()` could be true at all (`ladder_build::run`'s
                        // own doc explains the sequence), so the cache is stale here
                        // regardless of whether `attach_built_set` found a medium to tie
                        // it to — an unattached set still changed what `library_list`
                        // would read back as unrecognised.
                        crate::commands::invalidate_library_parts(&db, &events, &request.server_id);
                    }
                    conn.close().await;
                    // ⚠ **The build says what it has to say as it happens, and this no
                    // longer carries it** (T524). The first shape of this fixed T416 —
                    // `outcome.map(|_| ())` threw away everything the build had worked out
                    // about itself — by copying `Built.notices` across here. That still lost
                    // every one of them on any failure, because `Built` only exists when
                    // there is no failure, and the build that fails is the one whose notices
                    // explain why. They go into the task where they are produced now.
                    outcome.map_err(build_error)?;
                    Ok(())
                },
            )
            .await?;
        // T591 — not a real resume position (nothing in `tasks::ladder_build` ever reads this
        // field back for BuildLadder tasks; see the doc on `PauseKind::ResumableAcrossRestart`
        // for how this kind actually carries on after a restart). Written here for the one
        // narrower reason `running_build_for` above needs it: a live marker of which slug this
        // task is building, read back the same way `running_upload_for` already reads its own
        // `resume_token` on the upload side.
        let _ = crate::tasks::store::save_resume_token(&state.db, &task_id, &slug_for_marker);
        Ok(task_id)
    }

    /// Ask the serving for every variant of a set (FR-047).
    ///
    /// Separate from building so that a set can be asked about at any time — a variant
    /// can stop being served long after it was made, and nothing else would notice.
    pub async fn ladder_verify(
        state: &super::super::AppState,
        server_id: &str,
        slug: &str,
    ) -> Result<crate::server::hls_verify::LadderVerdict> {
        let profile = super::super::library::api::profile_of(state, server_id)?;
        let master_url =
            crate::domain::links::for_path(&profile.domain, None, &format!("{slug}/master.m3u8"))
                .origin;

        // What the description itself names is what is expected: asking for a number from
        // elsewhere would let a set with a rung missing from its own description pass.
        let verdict = crate::server::hls_verify::verify(&master_url, 0)
            .await
            .map_err(|e| AppError::new(ErrorCode::DomainNotServing).with_cause(e))?;
        let expected = verdict.variants_in_master;
        let verdict = crate::server::hls_verify::LadderVerdict {
            variants_expected: expected,
            ..verdict
        };

        if !verdict.ok() {
            return Err(
                AppError::new(ErrorCode::LadderIncomplete).with_cause(verdict.broken().join(", "))
            );
        }
        Ok(verdict)
    }

    /// Check rungs a person has edited (FR-044).
    ///
    /// A pure function, and called on every edit rather than at the end: learning that a
    /// rung is impossible after agreeing to hours of encoding is learning it too late.
    pub async fn ladder_validate(check: &LadderCheck) -> Result<LadderVerdict> {
        Ok(LadderVerdict::of(&check.rungs, &check.source))
    }

    /// Rebuild one rung after a person retypes its bitrate by hand (T523, FR-025).
    ///
    /// **Why this has to be a round trip through the core and not a screen editing the
    /// numbers itself.** The screen knows one new number — the bitrate — and nothing about
    /// what it decides. The ceiling, the buffer and the height are all worked out from the
    /// bitrate by rules that live here (`height_for`, `peak_control`); asking the screen to
    /// carry them along unchanged is exactly the bug this command exists to close (a rung
    /// retyped from 15 Mbit/s to 3 kept a ceiling near 18 — no ceiling at all at 3 — and
    /// stayed at the old rung's 2160p).
    ///
    /// A pure function, like [`ladder_validate`], and meant to be called the same way: on
    /// every edit, not only when the person is done. It does not check the result against
    /// its neighbours — that is still `ladder_validate`'s job, run straight after on the
    /// rung this returns swapped into the list.
    pub async fn ladder_recompute_rung(request: &RecomputeRungRequest) -> Result<Rung> {
        Ok(ladder::recompute_rung(
            request.index,
            request.bitrate_bps,
            &request.source,
        ))
    }
}

/// Attach a just-built quality set to its medium in the catalogue (T528).
///
/// `slug` is looked up against the catalogue's own `slug`s, exactly as `LadderPreview`'s
/// `T519(3)` lookup already does — this reuses that same matching rather than repeating it.
/// When a medium is found, the set's master playlist is filed under it as `{slug}/master.m3u8`
/// — the nested path `Manifest::with_file_under` needs to recognise it as a quality ladder
/// rather than an ordinary file, and the one a viewer actually opens. Handed the bare slug
/// instead, the set would land in neither `files` nor `ladders` of any medium and surface as
/// an unrecognised top-level directory.
///
/// Best effort throughout and `None` on anything that keeps the tie from being made — no
/// matching medium, an unreadable catalogue, a write that lost the race with another writer.
/// A build that already succeeded on the server must not be reported as failed over a
/// bookkeeping step nobody asked to see the result of.
pub async fn attach_built_set(
    conn: &crate::ssh::Connection,
    video_dir: &str,
    slug: &str,
) -> Option<String> {
    let manifest = match crate::server::manifest_io::read(conn, video_dir).await {
        Ok(m) => m,
        Err(e) => {
            tracing::debug!(
                error = %e,
                "could not read the catalogue to find the built set's medium"
            );
            return None;
        }
    };
    let media_id = manifest.find_by_slug(slug)?.id.clone();

    let ladder_path = format!("{slug}/master.m3u8");
    if let Some(next) = manifest.with_file_under(&media_id, &ladder_path, true) {
        if let Err(e) =
            crate::server::manifest_io::write(conn, video_dir, &next, manifest.generation).await
        {
            tracing::warn!(
                error = %e,
                media_id,
                "the built set could not be attached to its medium in the catalogue"
            );
        }
    }
    Some(media_id)
}

/// The measured ladder for this material, when there is one.
fn measured_plan(
    state: &super::AppState,
    request: &LadderRequest,
    source: &SourceFacts,
) -> Result<Option<LadderPreview>> {
    let Ok(key) = measurements::key_for(std::path::Path::new(&request.path)) else {
        return Ok(None);
    };
    let Ok(Some(run)) = measurements::run(&state.db, &key, &request.codec) else {
        return Ok(None);
    };
    let points = measurements::points(&state.db, &key, &request.codec).unwrap_or_default();
    if points.is_empty() {
        return Ok(None);
    }
    // **A loan still being checked is not a measurement yet** (T478). The points are already
    // here — that is the whole reason this has to be said out loud: a provisional ladder
    // looks exactly like a vouched-for one, and the check that would refuse it is still
    // running.
    let pending = run.check_pending;

    let chosen = crate::domain::measured_ladder::select(
        &points,
        crate::domain::measured_ladder::TARGET_VMAF,
        crate::domain::measured_ladder::VMAF_STEP,
    );
    let borrowed = run.borrowed_from.is_some();
    let plan = ladder::from_measurement(&chosen.rungs, source, request.declared_layout, borrowed)
        .map_err(|refusal| {
        AppError::new(ErrorCode::InvalidInput).with_cause(refusal_text(refusal))
    })?;

    let mut notices = Vec::new();
    if pending {
        notices.push(Detail::new(
            crate::domain::wording::DetailCode::NoticeCheckPointRunning,
        ));
    }
    if let Some(from) = &run.borrowed_from {
        notices.push(
            Detail::new(crate::domain::wording::DetailCode::NoticeMeasurementBorrowed)
                .with("from", from.clone()),
        );
    }

    Ok(Some(LadderPreview {
        verdict: LadderVerdict::of(&plan.rungs, source),
        plan,
        from: if borrowed {
            LadderSource::Borrowed
        } else {
            LadderSource::Measured
        },
        source: *source,
        anchor_mbps: None,
        codec: request.codec.clone(),
        borrowed_from: run.borrowed_from.clone(),
        measurement_key: Some(key),
        notices,
    }))
}

fn build_error(e: crate::tasks::ladder_build::BuildError) -> AppError {
    use crate::tasks::ladder_build::BuildError as E;
    match e {
        E::Cancelled => AppError::new(ErrorCode::TaskCancelled),
        E::NotBuildable(_) => AppError::new(ErrorCode::LadderNotMeasured),
        // The one failure that names names: a person is owed "the lower rung" rather
        // than "something went wrong", because the two ask for different work.
        E::Incomplete(which) => {
            AppError::new(ErrorCode::LadderIncomplete).with_cause(which.join(", "))
        }
        E::NotEnoughSpace {
            needed,
            free,
            short_by,
            rungs,
        } => AppError::new(ErrorCode::RemoteDiskFull)
            .with_detail(
                Detail::new(DetailCode::LadderNotEnoughSpace)
                    .with("short_by", short_by)
                    .with("needed", needed)
                    .with("free", free)
                    .with("rungs", rungs as u64),
            )
            .with_cause(format!("short_by={short_by}")),
        E::NoRoomHere {
            needed,
            free,
            short_by,
            at,
        } => AppError::new(ErrorCode::InvalidInput)
            .with_detail(
                Detail::new(DetailCode::LadderNoRoomHere)
                    .with("short_by", short_by)
                    .with("needed", needed)
                    .with("free", free)
                    .with("at", at.clone()),
            )
            .with_cause(format!("short_by={short_by} at={at}")),
        // The code that already exists for exactly this, rather than the catch-all below: a
        // file that came out of the encoder and does not decode is not an internal fault, it
        // is the answer principle II exists to get. The decoder's own words go in the cause —
        // cryptic and searchable beats "the file is broken", which is neither.
        E::VariantBroken { variant, problems } => AppError::new(ErrorCode::DecodeValidationFailed)
            .with_cause(format!("{variant}: {}", problems.join("; "))),
        other => AppError::new(ErrorCode::Internal).with_cause(other),
    }
}

fn refusal_text(refusal: ladder::Refusal) -> String {
    match refusal {
        ladder::Refusal::SourceBitrateTooLow { bitrate_bps } => format!(
            "the source holds {bitrate_bps} bit/s, which is under a whole megabit: \
             there is nothing here to build a ladder out of"
        ),
    }
}

async fn pick_encoder(prefer_hardware: bool) -> Result<(encoders::Encoder, Vec<Detail>)> {
    let info = ffmpeg::probe_self()
        .await
        .map_err(|e| AppError::new(ErrorCode::FfmpegBroken).with_cause(e))?;
    let choice = encoders::choose(&info.hardware, info.has_x264, prefer_hardware)
        .map_err(|_| AppError::new(ErrorCode::NoHwEncoder))?;
    Ok((choice.encoder, choice.notice.into_iter().collect()))
}

pub mod ipc {
    use super::*;
    use tauri::State;

    #[tauri::command]
    pub async fn ladder_measure(path: String) -> Result<measure::Measured> {
        api::ladder_measure(&path).await
    }

    #[tauri::command]
    pub async fn ladder_plan(
        state: State<'_, super::super::AppState>,
        request: LadderRequest,
    ) -> Result<LadderPreview> {
        api::ladder_plan(&state, &request).await
    }

    #[tauri::command]
    pub async fn ladder_build(
        state: State<'_, super::super::AppState>,
        request: BuildRequest,
    ) -> Result<String> {
        api::ladder_build(&state, request).await
    }

    #[tauri::command]
    pub async fn ladder_verify(
        state: State<'_, super::super::AppState>,
        server_id: String,
        slug: String,
    ) -> Result<crate::server::hls_verify::LadderVerdict> {
        api::ladder_verify(&state, &server_id, &slug).await
    }

    #[tauri::command]
    pub async fn ladder_validate(check: LadderCheck) -> Result<LadderVerdict> {
        api::ladder_validate(&check).await
    }

    #[tauri::command]
    pub async fn ladder_recompute_rung(request: RecomputeRungRequest) -> Result<Rung> {
        api::ladder_recompute_rung(&request).await
    }
}
