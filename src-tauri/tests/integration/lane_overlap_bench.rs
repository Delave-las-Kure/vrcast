//! T517 — measure the resource overlap between `BuildLadder` and `Compute`.
//!
//! This is deliberately an ignored measurement rather than an assertion about which lane is
//! right. Principle VI requires numbers before either of the two existing, measured rules is
//! changed. Run it explicitly, with Docker available, and keep the per-run readings as well as
//! the medians:
//!
//! ```text
//! cargo test --features integration --test integration -- --ignored --nocapture lane_overlap
//! ```

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::fixture::TestServer;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Barrier, Notify, Semaphore};
use vrcast_studio_lib::domain::convert_plan::{self, ConvertPlan, ConvertRequest, VideoAction};
use vrcast_studio_lib::domain::measure_grid::Cell;
use vrcast_studio_lib::domain::source::SourceFile;
use vrcast_studio_lib::media::convert::{self, ConvertJob};
use vrcast_studio_lib::media::encoders::{self, Encoder};
use vrcast_studio_lib::media::{ffmpeg, probe, vmaf};
use vrcast_studio_lib::ssh::Connection;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::tasks::engine::TaskContext;
use vrcast_studio_lib::tasks::process::ManagedProcess;

const SOURCE_SECONDS: u64 = 40;
const MEASURE_SECONDS: u64 = 4;
const RUNS: usize = 3;
const VIDEO_DIR: &str = "/var/lib/vrcast/videos";

/// Five real grid-like encodes: one source, five positions and cells, four seconds apiece.
const MEASURE_POINTS: [(u64, Cell); 5] = [
    (
        0,
        Cell {
            bitrate_mbps: 2,
            height: 360,
        },
    ),
    (
        8,
        Cell {
            bitrate_mbps: 4,
            height: 480,
        },
    ),
    (
        16,
        Cell {
            bitrate_mbps: 6,
            height: 540,
        },
    ),
    (
        24,
        Cell {
            bitrate_mbps: 8,
            height: 720,
        },
    ),
    (
        32,
        Cell {
            bitrate_mbps: 10,
            height: 720,
        },
    ),
];

#[derive(Clone, Copy, Debug)]
enum Scenario {
    A,
    B,
    C,
}

impl Scenario {
    fn label(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::C => "C",
        }
    }
}

struct Samples {
    a: Vec<Duration>,
    b: Vec<Duration>,
    c: Vec<Duration>,
}

/// Conditions shared by every timed scenario. Keeping them together also makes it harder for
/// one scenario to be measured against subtly different input or a different connection.
struct Bench<'a> {
    ffmpeg_bin: &'a Path,
    source_path: &'a Path,
    source: &'a SourceFile,
    plan: &'a ConvertPlan,
    encoder: &'a Encoder,
    conn: &'a Connection,
    server: &'a TestServer,
    root: &'a Path,
}

impl Samples {
    fn new() -> Self {
        Self {
            a: Vec::with_capacity(RUNS),
            b: Vec::with_capacity(RUNS),
            c: Vec::with_capacity(RUNS),
        }
    }

    fn push(&mut self, scenario: Scenario, elapsed: Duration) {
        match scenario {
            Scenario::A => self.a.push(elapsed),
            Scenario::B => self.b.push(elapsed),
            Scenario::C => self.c.push(elapsed),
        }
    }
}

fn make_source(ffmpeg_bin: &Path, path: &Path) -> Result<(), String> {
    let made = std::process::Command::new(ffmpeg_bin)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=1280x720:rate=30",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            &SOURCE_SECONDS.to_string(),
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            "18",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-b:a",
            "128k",
            "-ac",
            "2",
            "-movflags",
            "+faststart",
        ])
        .arg(path)
        .output()
        .map_err(|e| format!("could not start FFmpeg to make the source: {e}"))?;

    if made.status.success() {
        Ok(())
    } else {
        Err(format!(
            "FFmpeg would not make the source: {}",
            String::from_utf8_lossy(&made.stderr).trim()
        ))
    }
}

async fn try_grid_encode(
    ffmpeg_bin: &Path,
    source: &Path,
    work_dir: &Path,
    at_s: u64,
    seconds: u64,
    cell: Cell,
    encoder: &Encoder,
) -> Result<(), String> {
    std::fs::create_dir_all(work_dir)
        .map_err(|e| format!("could not make {}: {e}", work_dir.display()))?;
    let args = vmaf::chunk_args(source, at_s, seconds, cell, encoder);
    let mut child = ManagedProcess::spawn_in(Some(work_dir), &ffmpeg_bin.to_string_lossy(), &args)
        .map_err(|e| format!("could not start the grid-point encode: {e}"))?;
    let (_stdout, stderr) = child.take_output();
    let complaints = tokio::spawn(async move {
        let mut text = String::new();
        if let Some(mut stderr) = stderr {
            let _ = stderr.read_to_string(&mut text).await;
        }
        text
    });
    let status = child
        .wait()
        .await
        .map_err(|e| format!("could not wait for the grid-point encode: {e}"))?;
    let said = complaints.await.unwrap_or_default();

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "the grid-point encode with {} failed: {}",
            encoder.ffmpeg_name(),
            said.trim()
        ))
    }
}

/// Take the encoder the application would prefer, then prove that it starts on this machine.
/// `probe_self` names what the bundled build contains; the one-second encode separates that
/// from a codec whose name is present but whose hardware or driver is not.
async fn benchmark_encoder(
    info: &ffmpeg::FfmpegInfo,
    ffmpeg_bin: &Path,
    source: &Path,
    root: &Path,
) -> Result<(Encoder, bool), String> {
    let chosen = encoders::choose(&info.hardware, info.has_x264, true)
        .map_err(|e| format!("the bundled FFmpeg has no usable encoder: {e}"))?
        .encoder;

    if matches!(chosen, Encoder::Hardware { .. }) {
        let trial = root.join("hardware-trial");
        match try_grid_encode(
            ffmpeg_bin,
            source,
            &trial,
            0,
            1,
            Cell {
                bitrate_mbps: 2,
                height: 360,
            },
            &chosen,
        )
        .await
        {
            Ok(()) => return Ok((chosen, true)),
            Err(reason) => println!(
                "lane_overlap hardware_trial=failed encoder={} reason={reason}",
                chosen.ffmpeg_name()
            ),
        }
    }

    if !info.has_x264 {
        return Err(String::from(
            "the preferred hardware encoder would not run and libx264 is absent",
        ));
    }
    let software = Encoder::Software;
    try_grid_encode(
        ffmpeg_bin,
        source,
        &root.join("software-trial"),
        0,
        1,
        Cell {
            bitrate_mbps: 2,
            height: 360,
        },
        &software,
    )
    .await?;
    Ok((software, false))
}

async fn measurement_workload(
    ffmpeg_bin: &Path,
    source: &Path,
    work_dir: &Path,
    encoder: &Encoder,
    encode_lane: Option<&Semaphore>,
) -> Result<(), String> {
    for (at_s, cell) in MEASURE_POINTS {
        let _place = match encode_lane {
            Some(lane) => Some(
                lane.acquire()
                    .await
                    .map_err(|_| String::from("the encode lane was closed"))?,
            ),
            None => None,
        };
        try_grid_encode(
            ffmpeg_bin,
            source,
            work_dir,
            at_s,
            MEASURE_SECONDS,
            cell,
            encoder,
        )
        .await?;
    }
    Ok(())
}

async fn build_encode(
    source: &SourceFile,
    plan: &ConvertPlan,
    encoder: &Encoder,
    output: &Path,
) -> Result<(), String> {
    let db = Arc::new(Db::open_in_memory().map_err(|e| e.to_string())?);
    let ctx = TaskContext::detached(db);
    let output = output.to_string_lossy().into_owned();
    let job = ConvertJob {
        source,
        plan,
        encoder,
        out_path: &output,
    };
    let notices = convert::run(&job, &ctx)
        .await
        .map_err(|e| format!("the build encode failed: {e}"))?;
    if !notices.is_empty() {
        return Err(format!(
            "the build encode changed encoder, so this sample is not comparable: {:?}",
            notices.iter().map(|notice| notice.key).collect::<Vec<_>>()
        ));
    }
    Ok(())
}

/// The same SFTP shape as the ladder build: read the encoded variant, write a staged remote
/// file, close it, then rename it into place. This is network input-output, not a local copy.
async fn send_file(conn: &Connection, local: &Path, remote: &str) -> Result<(), String> {
    let body = tokio::fs::read(local)
        .await
        .map_err(|e| format!("could not read the encoded variant: {e}"))?;
    let staged = format!("{remote}.part");
    let sftp = conn
        .sftp()
        .await
        .map_err(|e| format!("could not open SFTP: {e}"))?;
    let mut file = sftp
        .create(staged.clone())
        .await
        .map_err(|e| format!("could not create the staged remote file: {e}"))?;
    file.write_all(&body)
        .await
        .map_err(|e| format!("could not send the variant: {e}"))?;
    file.flush()
        .await
        .map_err(|e| format!("could not flush the variant: {e}"))?;
    file.shutdown()
        .await
        .map_err(|e| format!("could not close the variant: {e}"))?;
    drop(file);
    drop(sftp);

    let moved = conn
        // Both paths are made entirely in this test from fixed ASCII names.
        .exec(&format!("mv '{staged}' '{remote}'"))
        .await
        .map_err(|e| format!("could not put the sent variant in place: {e}"))?;
    if !moved.ok() {
        return Err(format!(
            "the server would not put the variant in place: {}",
            moved.stderr.trim()
        ));
    }
    Ok(())
}

fn verify_and_remove(server: &TestServer, remote: &str, expected_size: u64) -> Result<(), String> {
    let size = server.exec_inside(&format!("stat -c %s '{remote}'"))?;
    let size: u64 = size
        .trim()
        .parse()
        .map_err(|e| format!("the remote size would not parse from {size:?}: {e}"))?;
    if size != expected_size {
        return Err(format!(
            "SFTP sent {size} bytes, but the encoded variant has {expected_size}"
        ));
    }
    server.exec_inside(&format!("rm -f '{remote}'"))?;
    Ok(())
}

async fn scenario_a(bench: &Bench<'_>, work: &Path, remote: &str) -> Result<Duration, String> {
    let measurement_dir = work.join("measurement");
    let variant = work.join("variant.mp4");
    std::fs::create_dir_all(work).map_err(|e| e.to_string())?;

    let started = Instant::now();
    measurement_workload(
        bench.ffmpeg_bin,
        bench.source_path,
        &measurement_dir,
        bench.encoder,
        None,
    )
    .await?;
    build_encode(bench.source, bench.plan, bench.encoder, &variant).await?;
    send_file(bench.conn, &variant, remote).await?;
    let elapsed = started.elapsed();

    let size = std::fs::metadata(&variant)
        .map_err(|e| e.to_string())?
        .len();
    verify_and_remove(bench.server, remote, size)?;
    Ok(elapsed)
}

async fn scenario_b(bench: &Bench<'_>, work: &Path, remote: &str) -> Result<Duration, String> {
    let measurement_dir = work.join("measurement");
    let variant = work.join("variant.mp4");
    std::fs::create_dir_all(work).map_err(|e| e.to_string())?;

    let start_together = Arc::new(Barrier::new(2));
    let measurement_gate = start_together.clone();
    let build_gate = start_together;
    let started = Instant::now();
    let measurement = async {
        measurement_gate.wait().await;
        measurement_workload(
            bench.ffmpeg_bin,
            bench.source_path,
            &measurement_dir,
            bench.encoder,
            None,
        )
        .await
    };
    let build = async {
        build_gate.wait().await;
        build_encode(bench.source, bench.plan, bench.encoder, &variant).await?;
        send_file(bench.conn, &variant, remote).await
    };
    let (measured, built) = tokio::join!(measurement, build);
    measured?;
    built?;
    let elapsed = started.elapsed();

    let size = std::fs::metadata(&variant)
        .map_err(|e| e.to_string())?
        .len();
    verify_and_remove(bench.server, remote, size)?;
    Ok(elapsed)
}

async fn scenario_c(bench: &Bench<'_>, work: &Path, remote: &str) -> Result<Duration, String> {
    let measurement_dir = work.join("measurement");
    let variant = work.join("variant.mp4");
    std::fs::create_dir_all(work).map_err(|e| e.to_string())?;

    // Both workloads start now. The build takes the one encode place first. The measurement
    // waits for that phase, then takes the same place point by point while the build sends the
    // file without holding it. Thus no two encodes overlap, but SFTP and encoding do.
    let encode_lane = Arc::new(Semaphore::new(1));
    let build_has_lane = Arc::new(Notify::new());
    let started = Instant::now();
    let build_lane = encode_lane.clone();
    let build_notice = build_has_lane.clone();
    let build = async {
        let place = build_lane
            .acquire()
            .await
            .map_err(|_| String::from("the encode lane was closed"))?;
        build_notice.notify_one();
        build_encode(bench.source, bench.plan, bench.encoder, &variant).await?;
        drop(place);
        send_file(bench.conn, &variant, remote).await
    };
    let measurement = async {
        build_has_lane.notified().await;
        measurement_workload(
            bench.ffmpeg_bin,
            bench.source_path,
            &measurement_dir,
            bench.encoder,
            Some(&encode_lane),
        )
        .await
    };
    let (built, measured) = tokio::join!(build, measurement);
    built?;
    measured?;
    let elapsed = started.elapsed();

    let size = std::fs::metadata(&variant)
        .map_err(|e| e.to_string())?
        .len();
    verify_and_remove(bench.server, remote, size)?;
    Ok(elapsed)
}

fn median(samples: &[Duration]) -> Duration {
    assert!(
        samples.len() >= RUNS,
        "fewer than three readings prove nothing"
    );
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    sorted[sorted.len() / 2]
}

fn seconds(duration: Duration) -> f64 {
    duration.as_secs_f64()
}

fn outcome(a: Duration, b: Duration, c: Duration) -> &'static str {
    // Five percent is stated here rather than judged by eye after seeing the answer.
    const NOTICEABLE: f64 = 0.05;
    let (a, b, c) = (seconds(a), seconds(b), seconds(c));
    let c_faster_than_both = c < a * (1.0 - NOTICEABLE) && c < b * (1.0 - NOTICEABLE);
    let b_faster_than_a = b < a * (1.0 - NOTICEABLE);
    let c_near_b = (c - b).abs() <= b * NOTICEABLE;

    if c_faster_than_both {
        "3: C is noticeably faster than both A and B; phase-level serialization wins"
    } else if b_faster_than_a && c_near_b {
        "1: B is noticeably faster than A and C is approximately B; overlap helps"
    } else if !b_faster_than_a {
        "2: B is approximately A or slower; concurrent encodes buy nothing here"
    } else {
        "none of the three named outcomes: B beats A, but C is not approximately B and does not beat both"
    }
}

async fn run_scenario(
    bench: &Bench<'_>,
    scenario: Scenario,
    round: usize,
) -> Result<Duration, String> {
    let label = scenario.label();
    let work = bench.root.join(format!("round-{round}-{label}"));
    let remote = format!(
        "{VIDEO_DIR}/t517-{}-{round}.mp4",
        label.to_ascii_lowercase()
    );
    let elapsed = match scenario {
        Scenario::A => scenario_a(bench, &work, &remote).await?,
        Scenario::B => scenario_b(bench, &work, &remote).await?,
        Scenario::C => scenario_c(bench, &work, &remote).await?,
    };
    println!(
        "lane_overlap sample scenario={label} round={} seconds={:.3}",
        round + 1,
        seconds(elapsed)
    );
    Ok(elapsed)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "T517 measurement: real FFmpeg, real SFTP, Docker, and several minutes"]
async fn lane_overlap_measurement_prints_three_medians() {
    let logical_cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let info = ffmpeg::probe_self()
        .await
        .expect("the bundled FFmpeg could not be examined");
    let ffmpeg_bin = ffmpeg::locate("ffmpeg").expect("the bundled FFmpeg is absent");
    println!("lane_overlap logical_cores={logical_cores}");
    println!(
        "lane_overlap probe_self_hardware={} names={:?}",
        if info.hardware.is_empty() {
            "no"
        } else {
            "yes"
        },
        info.hardware
    );

    let root = std::env::temp_dir().join(format!(
        "vrcast-lane-overlap-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&root).expect("the benchmark directory would not be created");
    let source_path = root.join("source.mp4");
    make_source(&ffmpeg_bin, &source_path).expect("the source clip would not be made");
    let source = probe::probe(&source_path)
        .await
        .expect("the source clip would not be examined");
    assert!(
        source.duration_s >= SOURCE_SECONDS as f64 - 0.5,
        "the source is too short for the last measurement point: {} s",
        source.duration_s
    );

    let (encoder, usable_hardware) = benchmark_encoder(&info, &ffmpeg_bin, &source_path, &root)
        .await
        .expect("no encoder could run the benchmark");
    println!(
        "lane_overlap usable_hardware={} benchmark_encoder={}",
        if usable_hardware { "yes" } else { "no" },
        encoder.ffmpeg_name()
    );

    let plan = convert_plan::plan(
        &source,
        &ConvertRequest {
            audio_track: 0,
            target_kbps: Some(4_000),
            height: Some(720),
        },
    )
    .unwrap_or_else(|problems| panic!("the build encode plan was refused: {problems:?}"));
    assert!(
        matches!(
            &plan.video,
            VideoAction::Reencode { .. } | VideoAction::ReencodeCapped { .. }
        ),
        "the build workload must encode video rather than copy it"
    );

    let server = TestServer::start().expect("the throwaway SFTP server would not start");
    let conn = super::ssh_live::connect(&server).await;

    // Warm the SFTP path before the clock starts. Container/image startup, key generation and
    // first protocol setup belong to the fixture, not to any of the three workloads.
    let warm = root.join("sftp-warmup.bin");
    std::fs::write(&warm, vec![0x5a; 1024 * 1024]).expect("the warm-up file would not be made");
    let warm_remote = format!("{VIDEO_DIR}/t517-warmup.bin");
    send_file(&conn, &warm, &warm_remote)
        .await
        .expect("the SFTP warm-up failed");
    verify_and_remove(&server, &warm_remote, 1024 * 1024)
        .expect("the SFTP warm-up did not arrive whole");

    let bench = Bench {
        ffmpeg_bin: &ffmpeg_bin,
        source_path: &source_path,
        source: &source,
        plan: &plan,
        encoder: &encoder,
        conn: &conn,
        server: &server,
        root: &root,
    };

    // Rotate the order so A is not always the cold run and C is not always the warm one.
    let orders = [
        [Scenario::A, Scenario::B, Scenario::C],
        [Scenario::B, Scenario::C, Scenario::A],
        [Scenario::C, Scenario::A, Scenario::B],
    ];
    let mut samples = Samples::new();
    for (round, order) in orders.into_iter().enumerate() {
        for scenario in order {
            let elapsed = run_scenario(&bench, scenario, round)
                .await
                .unwrap_or_else(|e| {
                    panic!(
                        "scenario {} round {} failed: {e}",
                        scenario.label(),
                        round + 1
                    )
                });
            samples.push(scenario, elapsed);
        }
    }

    let a = median(&samples.a);
    let b = median(&samples.b);
    let c = median(&samples.c);
    println!(
        "lane_overlap median_a_seconds={:.3} median_b_seconds={:.3} median_c_seconds={:.3}",
        seconds(a),
        seconds(b),
        seconds(c)
    );
    println!(
        "lane_overlap relative_b_vs_a={:.3} relative_c_vs_a={:.3} relative_c_vs_b={:.3}",
        seconds(b) / seconds(a),
        seconds(c) / seconds(a),
        seconds(c) / seconds(b)
    );
    println!("lane_overlap outcome={}", outcome(a, b, c));

    conn.close().await;
    let _ = std::fs::remove_dir_all(&root);
}
