//! T561 — the encoder gets partway through a real source and fails on its own,
//! with a nonzero exit code, and nothing about it comes from outside.
//!
//! **Why the two existing checks miss this.** `tests/integration/convert_kill.rs` proves
//! an *external* kill leaves no process behind and no growing file — the application is
//! murdered, FFmpeg never gets to decide anything for itself. `tests/unit/encoder_fallback.rs`
//! proves a hardware encoder that refuses *at the very start*, before a single frame, sends
//! the work to the processor — that is a refusal to open, not a failure partway through a
//! stream. Neither drives FFmpeg into reading real frames, hitting real garbage in the
//! middle, and exiting with its own nonzero status entirely on its own. `convert::run`
//! handles that path today (`if !status.success() { cleanup(...); return
//! Err(ConvertError::Failed(...)) }`, `media/convert.rs`) — this is what proves it, rather
//! than trusting the read of the code.
//!
//! **What "broken" means here, and why it took three attempts to get right.** A merely
//! truncated MP4 is not good enough: modern FFmpeg is very forgiving of a stream that just
//! stops — it decodes everything before the cut, hits EOF, and exits 0 (verified by hand
//! before writing this: truncating a real clip anywhere from 5% to 30% of its length still
//! gave `EXIT_CODE=0`, every time). What actually forces FFmpeg to give up and exit nonzero
//! is *sustained* garbage feeding its decoder — enough consecutive corrupt packets to trip
//! its own internal "decode error rate exceeds maximum" guard. So the source here keeps its
//! `ftyp`/`moov` boxes (the index) intact — FFmpeg must recognise the file and start decoding
//! real frames — and has the back part of its `mdat` payload (the actual encoded frame data)
//! overwritten solidly. The result, verified by hand before this test was written: FFmpeg
//! decodes real frames from the good front part, encodes them into a real (nonempty) output
//! file, then hits the corrupted back part, exceeds its error-rate tolerance, and exits with
//! a nonzero status — exactly "got partway through and failed on its own".

use std::sync::Arc;
use std::time::Duration;

use vrcast_studio_lib::domain::convert_plan::{AudioAction, ConvertPlan, VideoAction};
use vrcast_studio_lib::domain::source::{AudioTrack, SourceFile};
use vrcast_studio_lib::domain::wording::DetailCode;
use vrcast_studio_lib::media::{convert, encoders::Encoder, ffmpeg};
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::tasks::engine::TaskEngine;
use vrcast_studio_lib::tasks::state::TaskKind;

/// A real film, long enough that corrupting its tail still leaves real frames at the front
/// for FFmpeg to actually decode and encode before it gives up.
fn make_film(ff: &std::path::Path, path: &std::path::Path) -> bool {
    let made = std::process::Command::new(ff)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x240:rate=24",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            "4",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-ac",
            "2",
        ])
        .arg(path)
        .output();
    matches!(made, Ok(o) if o.status.success())
}

/// Read the top-level MP4/ISOBMFF boxes of a file: `(offset, size, four_cc)` each.
///
/// A minimal reader for exactly what this needs — enough to find `mdat` without reaching
/// for a media-parsing crate to corrupt a file on purpose.
fn top_level_boxes(data: &[u8]) -> Vec<(usize, usize, String)> {
    let mut boxes = Vec::new();
    let mut pos = 0usize;
    while pos + 8 <= data.len() {
        let size = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        let kind = String::from_utf8_lossy(&data[pos + 4..pos + 8]).into_owned();
        if size == 0 {
            break;
        }
        boxes.push((pos, size, kind));
        pos += size;
    }
    boxes
}

/// Make a source FFmpeg can start decoding — but not finish — by corrupting the back part
/// of its `mdat` payload while leaving the front part and the index (`ftyp`/`moov`) intact.
///
/// Returns `None` if the film this was handed has no `mdat` box, which would mean the test
/// fixture itself is broken rather than the thing under test.
fn corrupt_tail_of_mdat(source_bytes: &[u8]) -> Option<Vec<u8>> {
    let mut data = source_bytes.to_vec();
    let boxes = top_level_boxes(&data);
    let (start, size, _) = boxes.into_iter().find(|(_, _, kind)| kind == "mdat")?;
    let payload_start = start + 8;
    let payload_end = start + size;
    let total = payload_end - payload_start;

    // The first 30% stays real: enough whole frames for the encoder to actually produce
    // output before the corruption is reached. Verified by hand: at this split FFmpeg
    // writes a genuinely nonempty file and still exits nonzero — neither extreme (0%, which
    // gives an empty output, or 100%, which a merely-truncated file already covers) shows
    // the "partway through, and failed on its own" shape this test is about.
    let corrupt_from = payload_start + total * 30 / 100;
    // A fixed, non-repeating-byte pattern rather than all-zero or all-0xFF: some decoders
    // treat a long run of one byte value as a special case (e.g. padding) rather than as
    // noise, which would risk being tolerated instead of rejected.
    let garbage = [0xFFu8, 0x00, 0xDE, 0xAD, 0xBE, 0xEF, 0x13, 0x37];
    for (offset, byte) in data[corrupt_from..payload_end].iter_mut().enumerate() {
        *byte = garbage[offset % garbage.len()];
    }
    Some(data)
}

fn source_of(path: &str, duration_s: f64) -> SourceFile {
    SourceFile {
        path: path.to_owned(),
        size_bytes: 1,
        duration_s,
        width: 320,
        height: 240,
        fps: 24,
        bitrate_bps: 1_000_000,
        peak_bps: None,
        video_codec: String::from("h264"),
        pix_fmt: String::from("yuv420p"),
        color_transfer: Some(String::from("bt709")),
        audio_tracks: vec![AudioTrack {
            index: 0,
            codec: String::from("aac"),
            profile: Some(String::from("LC")),
            channels: 2,
            bitrate_bps: Some(128_000),
            language: None,
            title: None,
            is_default: true,
        }],
    }
}

fn a_plan() -> ConvertPlan {
    ConvertPlan {
        video: VideoAction::Reencode {
            level: String::from("4.0"),
            reason: vrcast_studio_lib::domain::wording::Detail::new(
                DetailCode::ReasonTargetBitrate,
            ),
        },
        audio: AudioAction::Copy,
        requested_height: None,
        gop: 24,
        tonemap: false,
        faststart: true,
        audio_track: 0,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ffmpeg_failing_partway_through_a_broken_source_is_reported_and_leaves_no_output() {
    let Ok(ff) = ffmpeg::locate("ffmpeg") else {
        eprintln!("SKIPPED: no bundled FFmpeg. Run `npm run ffmpeg` for this to check anything.");
        return;
    };

    let dir =
        std::env::temp_dir().join(format!("vrcast-broken-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).expect("could not make a working directory");
    let good = dir.join("good.mp4");
    if !make_film(&ff, &good) {
        eprintln!("SKIPPED: the bundled FFmpeg would not make a clip");
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }

    let good_bytes = std::fs::read(&good).expect("the clip that was just made would not read");
    let Some(broken_bytes) = corrupt_tail_of_mdat(&good_bytes) else {
        panic!(
            "the test fixture itself is broken: the clip FFmpeg just made has no `mdat` box, \
             so this proves nothing about a broken source"
        );
    };
    let broken = dir.join("broken.mp4");
    std::fs::write(&broken, &broken_bytes).expect("could not write the corrupted source");

    let out = dir.join("out.mp4");
    let source = source_of(&broken.to_string_lossy(), 4.0);
    let plan = a_plan();
    let out_path = out.to_string_lossy().to_string();
    // The processor, not a hardware encoder: this is about the decoder's input, not about
    // which encoder is asked to open. `encoder_fallback.rs` already owns the hardware path.
    let encoder = Encoder::Software;

    let engine = TaskEngine::new(Arc::new(Db::open_in_memory().expect("no database")));
    let error_text: Arc<std::sync::Mutex<Option<String>>> = Arc::new(std::sync::Mutex::new(None));

    let keep = error_text.clone();
    let id = engine
        .submit(TaskKind::Convert, None, move |ctx| async move {
            let job = convert::ConvertJob {
                source: &source,
                plan: &plan,
                encoder: &encoder,
                out_path: &out_path,
            };
            match convert::run(&job, &ctx).await {
                Ok(_notices) => Ok(()),
                Err(e) => {
                    *keep.lock().unwrap() = Some(e.to_string());
                    Err(vrcast_studio_lib::commands::error::AppError::new(
                        vrcast_studio_lib::commands::error::ErrorCode::Internal,
                    )
                    .with_cause(e))
                }
            }
        })
        .await
        .expect("the task would not start");

    for _ in 0..200 {
        if let Ok(Some(task)) = engine.get(&id) {
            if matches!(
                task.state,
                vrcast_studio_lib::tasks::state::TaskState::Completed
                    | vrcast_studio_lib::tasks::state::TaskState::Failed
                    | vrcast_studio_lib::tasks::state::TaskState::Cancelled
            ) {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let task = engine.get(&id).ok().flatten().expect("the task vanished");
    assert_eq!(
        task.state,
        vrcast_studio_lib::tasks::state::TaskState::Failed,
        "a source that FFmpeg itself gives up on partway through must fail the task, not \
         complete it or leave it stuck — got {:?}, error: {:?}",
        task.state,
        task.error
    );
    assert!(
        task.error.is_some(),
        "the task is Failed but carries no error for a person to be told"
    );

    // The specific path this test exists for: `ConvertError::Failed`, not `Spawn` (which
    // would mean the encoder never started at all) and not `Cancelled` (nobody cancelled
    // anything here).
    let message = error_text
        .lock()
        .unwrap()
        .clone()
        .expect("convert::run returned Ok on a source it should have failed on");
    assert!(
        message.starts_with("encoding failed:"),
        "expected ConvertError::Failed's message shape (\"encoding failed: ...\"), got: \
         {message:?} — a Spawn-shaped message here would mean the encoder never started, \
         which is `encoder_fallback.rs`'s territory, not this test's"
    );

    // The property the constitution names directly (principle II): nothing half-made is
    // ever left where the next step (upload) would find it and mistake it for a result.
    // FFmpeg did write real bytes to `out_path` before it gave up — verified by hand while
    // building this fixture — so this is a real assertion about `cleanup()` running, not a
    // trivial one about a file that was simply never created.
    assert!(
        !out.exists(),
        "a partially written output survived a failed conversion — exactly the file \
         principle II exists to forbid: it has a plausible name and the next step would \
         happily try to upload it"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
