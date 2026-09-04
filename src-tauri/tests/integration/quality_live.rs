//! T235 — how long a point of the grid really takes, measured rather than guessed.
//!
//! The estimate a person is shown before agreeing to half an hour of their machine has to
//! come from somewhere. It came from a constant, and a constant is exactly what this
//! project keeps having to replace with a reading.
//!
//! Marked `ignore`: it needs a real film and a real encoder, and it runs for minutes. To
//! run it:
//!
//! ```text
//! VRCAST_MEASURE_SOURCE=F:/films/film.mp4 cargo test --features integration \
//!   --test integration -- --ignored --nocapture how_long_a_point_takes
//! ```
//!
//! `VRCAST_MEASURE_POINTS` says how many points to time (three by default). The whole grid
//! is not needed: the points differ in height and bitrate, and what is wanted is the shape
//! of the cost, not every value of it.

use std::path::Path;
use std::time::Instant;

use tokio_util::sync::CancellationToken;
use vrcast_studio_lib::domain::measure_grid::grid;
use vrcast_studio_lib::domain::{chunks, ladder::SourceFacts};
use vrcast_studio_lib::media::{encoders, ffmpeg, measure, probe_complexity, vmaf};

mod env {
    pub const SOURCE: &str = "VRCAST_MEASURE_SOURCE";
    pub const POINTS: &str = "VRCAST_MEASURE_POINTS";
    pub const NATIVE_HEIGHT: &str = "VRCAST_MEASURE_NATIVE_HEIGHT";
    /// Chunk starts in seconds, comma-separated, instead of the ones the weights choose.
    ///
    /// **For telling two things apart that otherwise mix.** The three chunks are picked by
    /// weight, not by position, so they can land within one minute of each other in a
    /// forty-minute episode — and then the measurement describes that minute. Comparing two
    /// films measured on different minutes mixes "these films differ" with "these minutes
    /// differ". Pinning the chunks holds one of the two still.
    pub const CHUNKS: &str = "VRCAST_MEASURE_CHUNKS";
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs a real film and a real encoder; run it by hand"]
async fn how_long_a_point_takes() {
    let Ok(source_path) = std::env::var(env::SOURCE) else {
        eprintln!("set {} to a film to time this against", env::SOURCE);
        return;
    };
    let path = Path::new(&source_path);
    let want: usize = std::env::var(env::POINTS)
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(3);
    let native_height: Option<u32> = std::env::var(env::NATIVE_HEIGHT)
        .ok()
        .and_then(|n| n.parse().ok());

    let info = ffmpeg::probe_self()
        .await
        .expect("the bundled FFmpeg would not answer");
    assert!(
        info.has_libvmaf,
        "this build of FFmpeg has no libvmaf, so there is nothing to time"
    );
    let choice =
        encoders::choose(&info.hardware, info.has_x264, true).expect("nothing to encode with");
    let encoder = choice.encoder;

    // Everything the real command does, in the same order, so that the timing is of the
    // real thing rather than of a convenient approximation.
    let probe = vrcast_studio_lib::commands::api::source_probe(&source_path)
        .await
        .expect("the film would not open");
    let started = Instant::now();
    let seconds = measure::seconds_of(path)
        .await
        .expect("the packets would not read");
    let reading_took = started.elapsed();
    let chunk_starts = match std::env::var(env::CHUNKS) {
        Ok(given) => given
            .split(',')
            .filter_map(|s| s.trim().parse::<u64>().ok())
            .collect(),
        Err(_) => chunks::reference_chunks(&seconds, chunks::CHUNK_S),
    };

    let started = Instant::now();
    let probed = probe_complexity::probe(path, probe.duration_s, &encoder).await;
    let probing_took = started.elapsed();
    let anchor_mbps = probed
        .measured_bps
        .map(|bps| (bps / 1_000_000).max(1))
        .unwrap_or(35);

    let facts = SourceFacts {
        width: probe.width,
        height: probe.height,
        fps: probe.fps,
        bitrate_bps: probe.bitrate_bps,
        heavier_codec: probe.video_codec.eq_ignore_ascii_case("hevc"),
        native_height,
    };
    let cells = grid(&facts, anchor_mbps);

    println!();
    println!(
        "film      : {}x{}@{} {} ({:.0} min)",
        probe.width,
        probe.height,
        probe.fps,
        probe.video_codec,
        probe.duration_s / 60.0
    );
    println!("encoder   : {}", encoder.ffmpeg_name());
    println!("packets   : {:.1} s to read", reading_took.as_secs_f64());
    println!(
        "chunks    : {chunk_starts:?} ({:.1} s to probe)",
        probing_took.as_secs_f64()
    );
    println!(
        "anchor    : {anchor_mbps} Mbit/s -> {} points in the grid",
        cells.len()
    );
    println!();

    // The heaviest and the lightest points first: if the cost varies at all, it varies
    // between those, and an average of the middle would hide it.
    let mut chosen: Vec<_> = cells.clone();
    chosen.sort_by_key(|c| c.bitrate_mbps * u64::from(c.height));
    let picked: Vec<_> = if chosen.len() <= want {
        chosen
    } else {
        let last = chosen.len() - 1;
        let mut picked = vec![chosen[0], chosen[last]];
        picked.extend(
            chosen[1..last]
                .iter()
                .step_by(((last - 1) / (want - 1)).max(1))
                .take(want - 2)
                .copied(),
        );
        picked
    };

    let cancel = CancellationToken::new();
    let mut took: Vec<f64> = Vec::new();
    for cell in &picked {
        let started = Instant::now();
        let point = vmaf::measure_point(
            path,
            probe.width,
            probe.height,
            &chunk_starts,
            chunks::CHUNK_S as u64,
            *cell,
            &encoder,
            &cancel,
        )
        .await;
        let seconds = started.elapsed().as_secs_f64();
        took.push(seconds);
        match point {
            Ok(p) => println!(
                "{:>3} Mbit/s @ {:>4}p : VMAF {:>6.2}, actually {:.1} Mbit/s — {:.1} s{}",
                p.point.bitrate_mbps,
                p.point.height,
                p.point.vmaf,
                p.point.actual_bps as f64 / 1e6,
                seconds,
                // How much of the film the number describes (R-50): a point averaged over
                // fewer chunks than it was given says less than it looks.
                if p.whole() {
                    String::new()
                } else {
                    format!(" — on {} chunks of {}", p.chunks_used, p.chunks_asked)
                }
            ),
            Err(e) => println!(
                "{:>3} Mbit/s @ {:>4}p : would not measure ({e}) — {:.1} s",
                cell.bitrate_mbps, cell.height, seconds
            ),
        }
    }

    let average = took.iter().sum::<f64>() / took.len() as f64;
    println!();
    println!("per point : {average:.0} s (of {} timed)", took.len());
    println!(
        "whole grid: about {:.0} min for {} points",
        average * cells.len() as f64 / 60.0,
        cells.len()
    );
    println!();
    println!("this is the number SECONDS_PER_POINT in commands/quality.rs stands for");
}
