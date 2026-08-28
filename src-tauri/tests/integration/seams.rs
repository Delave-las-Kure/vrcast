//! The seams between the pieces — the two runs that had never happened whole.
//!
//! Every part of measuring quality and of building a ladder is checked on its own: the
//! grid's arithmetic without a film, one point of it against a real encoder, the cutting
//! and the serving against a real server. **The orchestration was checked nowhere.** That
//! is the shape of thing this project's own audits have twice found a fault in: the parts
//! were right and the joins between them were not.
//!
//! So each of these runs the real task, through the real engine, on a real film — from the
//! first refusal to the last verdict.
//!
//! **They run every time rather than being kept for special occasions.** The first run of
//! the first one found that the database did not know `measure_quality` as a kind of task
//! at all, so measuring anything failed the moment it started — a fault that had been sitting
//! there since the kind was added. A check kept behind `ignore` would have let the next one
//! sit just as long. Between them they cost about twenty seconds, and they make their own
//! film: nothing has to be brought from outside.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use vrcast_studio_lib::domain::chunks;
use vrcast_studio_lib::domain::ladder::{self, Quality, Rung};
use vrcast_studio_lib::media::{encoders, ffmpeg, measure};
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::measurements::{self, Run};
use vrcast_studio_lib::tasks::engine::TaskEngine;
use vrcast_studio_lib::tasks::state::{TaskKind, TaskState};

use super::fixture::TestServer;
use super::hls_fixture::VIDEO_DIR;
use super::ssh_live::connect;

/// A small but real film, made here so the checks need nothing brought from outside.
///
/// Small on purpose: what is being checked is the joins, not the encoder's speed, and a
/// two-hour film would turn a check of the joins into an afternoon.
fn make_film(path: &Path, height: u32, seconds: u32) -> Result<(), String> {
    let ffmpeg_bin = ffmpeg::locate("ffmpeg").map_err(|e| e.to_string())?;
    let out = std::process::Command::new(ffmpeg_bin)
        .args([
            "-nostdin",
            "-y",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!(
                "testsrc2=size={}x{height}:rate=24:duration={seconds}",
                height * 16 / 9
            ),
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency=440:duration={seconds}"),
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-b:v",
            "4000k",
            "-g",
            "24",
            "-keyint_min",
            "24",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
        ])
        .arg(path)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    Ok(())
}

/// A place to work that clears up after itself.
struct Workspace(std::path::PathBuf);

impl Workspace {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("vrcast-seam-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("nowhere to work");
        Self(dir)
    }

    fn path(&self, name: &str) -> std::path::PathBuf {
        self.0.join(name)
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn wait_for(engine: &TaskEngine, id: &str, want: TaskState, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if let Ok(Some(record)) = engine.get(id) {
            if record.state == want {
                return true;
            }
            if record.state.is_final() && record.state != want {
                return false;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

fn an_encoder() -> encoders::Encoder {
    // Software on purpose: what is being checked is the joins, and they must hold on a
    // machine with no graphics card — which is most machines this will ever run on.
    encoders::Encoder::Software
}

// ---------- the measurement, whole ----------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn measuring_quality_runs_whole_and_picks_up_where_it_stopped() {
    let work = Workspace::new("measure");
    let film = work.path("film.mp4");
    make_film(&film, 360, 30).expect("the film would not be made");

    let db = Arc::new(Db::open_in_memory().expect("the database would not open"));
    let engine = TaskEngine::new(db.clone());

    let seconds = measure::seconds_of(&film)
        .await
        .expect("the packets would not read");
    let run = Run {
        source_key: measurements::key_for(&film).expect("no key for the film"),
        codec: String::from("h264"),
        source_path: film.to_string_lossy().into_owned(),
        width: 640,
        height: 360,
        fps: 24,
        source_bitrate_bps: 4_000_000,
        heavier_codec: false,
        native_height: None,
        // A small anchor keeps the grid small: five points rather than fifteen. The joins
        // do not care how many there are, and neither does anybody waiting for this.
        anchor_mbps: 2,
        chunk_starts: chunks::reference_chunks(&seconds, 5),
        chunk_s: 5,
        borrowed_from: None,
    };

    let encoder = an_encoder();
    let job_run = run.clone();
    let job_db = db.clone();
    let job_film = film.clone();
    let id = engine
        .submit(TaskKind::MeasureQuality, None, move |ctx| async move {
            let job = vrcast_studio_lib::tasks::quality_measure::MeasureJob {
                source: &job_film,
                run: &job_run,
                encoder: &encoder,
                db: &job_db,
            };
            let outcome = vrcast_studio_lib::tasks::quality_measure::run(&job, &ctx)
                .await
                .map_err(|e| {
                    vrcast_studio_lib::commands::error::AppError::new(
                        vrcast_studio_lib::commands::error::ErrorCode::Internal,
                    )
                    .with_cause(e)
                })?;
            assert!(
                !outcome.selection.rungs.is_empty(),
                "the measurement finished and chose no rungs at all"
            );
            Ok(())
        })
        .await
        .expect("the task would not start");

    assert!(
        wait_for(&engine, &id, TaskState::Completed, Duration::from_secs(600)).await,
        "the measurement did not finish: {:?}",
        engine.get(&id).unwrap().unwrap()
    );

    // Every point is in the store, with a time against it — that is what the estimate for
    // the next film is built from.
    let points = measurements::points(&db, &run.source_key, &run.codec).expect("no points");
    assert!(
        points.len() >= 3,
        "the grid answered with {} points, which is not a measurement",
        points.len()
    );
    for point in &points {
        assert!(
            point.vmaf > 0.0 && point.vmaf <= 100.0,
            "a point came back with a nonsensical score: {point:?}"
        );
    }
    let (factor, counted) = measurements::machine_factor(&db)
        .expect("the correction would not read")
        .expect("nothing was learned from a run that just happened");
    assert_eq!(counted, points.len());
    assert!(factor > 0.0);

    // **And now the join that had never been walked.** Running it again must add nothing:
    // the points are already there, and each one costs minutes.
    let before = std::time::Instant::now();
    let again_run = run.clone();
    let again_db = db.clone();
    let again_film = film.clone();
    let encoder = an_encoder();
    let second = engine
        .submit(TaskKind::MeasureQuality, None, move |ctx| async move {
            let job = vrcast_studio_lib::tasks::quality_measure::MeasureJob {
                source: &again_film,
                run: &again_run,
                encoder: &encoder,
                db: &again_db,
            };
            vrcast_studio_lib::tasks::quality_measure::run(&job, &ctx)
                .await
                .map(|_| ())
                .map_err(|e| {
                    vrcast_studio_lib::commands::error::AppError::new(
                        vrcast_studio_lib::commands::error::ErrorCode::Internal,
                    )
                    .with_cause(e)
                })
        })
        .await
        .expect("the second task would not start");
    assert!(
        wait_for(
            &engine,
            &second,
            TaskState::Completed,
            Duration::from_secs(120)
        )
        .await,
        "the second run did not finish"
    );

    let after = measurements::points(&db, &run.source_key, &run.codec).expect("no points");
    assert_eq!(
        after.len(),
        points.len(),
        "the second run measured something all over again"
    );
    assert!(
        before.elapsed() < Duration::from_secs(60),
        "the second run took {:?}, which means it was not resuming but repeating",
        before.elapsed()
    );
}

// ---------- building a ladder, whole ----------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn building_a_ladder_runs_from_the_refusal_to_the_verdict() {
    let work = Workspace::new("build");
    let film = work.path("film.mp4");
    make_film(&film, 360, 24).expect("the film would not be made");

    let server = TestServer::start().expect("the container would not come up");
    let conn = connect(&server).await;
    let db = Arc::new(Db::open_in_memory().expect("the database would not open"));
    let engine = TaskEngine::new(db.clone());

    let source = vrcast_studio_lib::commands::api::source_probe(&film.to_string_lossy())
        .await
        .expect("the film would not open");

    // Two rungs, both under the source, spaced the way the checker wants them.
    let facts = ladder::SourceFacts {
        width: source.width,
        height: source.height,
        fps: source.fps,
        bitrate_bps: source.bitrate_bps,
        heavier_codec: false,
        native_height: None,
    };
    let rungs: Vec<Rung> = vec![
        rung(0, 2_000_000, source.height, source.width, &source),
        rung(1, 1_000_000, source.height, source.width, &source),
    ];

    // **The refusal first**, because it is the point of the whole arrangement: an unmeasured
    // ladder is hours of encoding spent on a guess, and the task must stop before the first
    // frame rather than after the last.
    let unmeasured: Vec<Rung> = rungs
        .iter()
        .map(|r| Rung {
            quality: Quality::NotMeasured,
            ..r.clone()
        })
        .collect();
    assert!(
        ladder::buildable(&unmeasured).is_err(),
        "an unmeasured ladder was declared buildable before the task even ran"
    );

    let master_url = format!(
        "http://{}:{}/videos/seam/master.m3u8",
        server.host(),
        server.http_port
    );
    let encoder = an_encoder();
    let job_source = source.clone();
    let job_rungs = rungs.clone();
    let job_url = master_url.clone();
    let work_dir = work.path("prepared");

    let id = engine
        .submit(TaskKind::BuildLadder, None, move |ctx| async move {
            let job = vrcast_studio_lib::tasks::ladder_build::BuildJob {
                conn: &conn,
                video_dir: VIDEO_DIR,
                owner: "root:root",
                slug: "seam",
                source: &job_source,
                rungs: &job_rungs,
                encoder: &encoder,
                audio_track: 0,
                master_url: &job_url,
                work_dir: &work_dir,
            };
            let built = vrcast_studio_lib::tasks::ladder_build::run(&job, &ctx)
                .await
                .map_err(|e| {
                    vrcast_studio_lib::commands::error::AppError::new(
                        vrcast_studio_lib::commands::error::ErrorCode::Internal,
                    )
                    .with_cause(e)
                })?;

            assert_eq!(built.variants.len(), 2, "not every rung became a variant");
            assert_eq!(built.prepared, 2, "something was skipped on a first build");
            assert!(
                built.verdict.ok(),
                "the build finished and the ladder is not served whole: {:?}",
                built.verdict
            );
            Ok(())
        })
        .await
        .expect("the task would not start");

    assert!(
        wait_for(&engine, &id, TaskState::Completed, Duration::from_secs(900)).await,
        "the build did not finish: {:?}",
        engine.get(&id).unwrap().unwrap()
    );

    // What a viewer would be given, asked for as a viewer would ask.
    let master = reqwest::get(&master_url)
        .await
        .expect("the serving would not answer")
        .text()
        .await
        .expect("no answer body");
    let variants = vrcast_studio_lib::domain::hls_master::parse(&master)
        .expect("the description would not read");
    assert_eq!(variants.len(), 2);
    for variant in &variants {
        assert!(
            variant.bandwidth > 0,
            "a variant went into the description with no bandwidth: {variant:?}"
        );
    }

    let _ = facts;
}

fn rung(
    index: usize,
    bitrate_bps: u64,
    height: u32,
    width: u32,
    source: &vrcast_studio_lib::domain::source::SourceFile,
) -> Rung {
    Rung {
        index,
        bitrate_bps,
        maxrate_bps: bitrate_bps * 11 / 10,
        bufsize_bps: bitrate_bps * 11 / 10,
        width,
        height,
        level: vrcast_studio_lib::domain::convert_plan::h264_level(width, height, source.fps)
            .to_owned(),
        reasons: Vec::new(),
        // Measured, because the task refuses otherwise — and the refusal is checked above.
        quality: Quality::MeasuredHere { vmaf_x100: 9500 },
    }
}
