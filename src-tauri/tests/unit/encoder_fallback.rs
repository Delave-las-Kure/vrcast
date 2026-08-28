//! T464 — the graphics card refuses, and the work goes to the processor.
//!
//! **What it costs to have no fallback.** NVENC limits how many encodes it will run at once,
//! and the limit is reached in ordinary use — the second or third task. Until now that ended
//! the work: the person was told the preparation failed, and nothing said it would have gone
//! through on the processor. Slower, and done, is better than not done.
//!
//! **Driven with a real refusal, not a stand-in.** The encoder asked for does not exist, so
//! the bundled FFmpeg refuses to open it in the same way and at the same moment a card at its
//! session limit does — before a single frame. Faking that with a stub would prove the retry
//! calls itself, and nothing about whether FFmpeg's refusal is caught.

use std::sync::Arc;
use std::time::Duration;

use vrcast_studio_lib::domain::convert_plan::{AudioAction, ConvertPlan, VideoAction};
use vrcast_studio_lib::domain::source::{AudioTrack, SourceFile};
use vrcast_studio_lib::domain::wording::DetailCode;
use vrcast_studio_lib::media::{convert, encoders::Encoder, ffmpeg};
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::tasks::engine::TaskEngine;
use vrcast_studio_lib::tasks::state::TaskKind;

/// A tiny real film, made by the bundled FFmpeg.
fn make_film(path: &std::path::Path) -> bool {
    let Ok(ff) = ffmpeg::locate("ffmpeg") else {
        return false;
    };
    let made = std::process::Command::new(&ff)
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
            "2",
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

fn source_of(path: &str) -> SourceFile {
    SourceFile {
        path: path.to_owned(),
        size_bytes: 1,
        duration_s: 2.0,
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
async fn a_refusing_graphics_card_sends_the_work_to_the_processor() {
    let Ok(_) = ffmpeg::locate("ffmpeg") else {
        eprintln!("SKIPPED: no bundled FFmpeg. Run `npm run ffmpeg` for this to check anything.");
        return;
    };

    let dir =
        std::env::temp_dir().join(format!("vrcast-fallback-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).expect("could not make a working directory");
    let film = dir.join("in.mp4");
    if !make_film(&film) {
        eprintln!("SKIPPED: the bundled FFmpeg would not make a clip");
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    let out = dir.join("out.mp4");

    let source = source_of(&film.to_string_lossy());
    let plan = a_plan();
    let out_path = out.to_string_lossy().to_string();
    // A card that is not there answers exactly as a card at its session limit does.
    let encoder = Encoder::Hardware {
        name: String::from("h264_nothing_of_the_sort"),
    };

    let engine = TaskEngine::new(Arc::new(Db::open_in_memory().expect("no database")));
    let said: Arc<std::sync::Mutex<Vec<DetailCode>>> = Arc::new(std::sync::Mutex::new(Vec::new()));

    let keep = said.clone();
    let id = engine
        .submit(TaskKind::Convert, None, move |ctx| async move {
            let job = convert::ConvertJob {
                source: &source,
                plan: &plan,
                encoder: &encoder,
                out_path: &out_path,
            };
            match convert::run(&job, &ctx).await {
                Ok(notices) => {
                    keep.lock().unwrap().extend(notices.iter().map(|n| n.key));
                    Ok(())
                }
                Err(e) => Err(vrcast_studio_lib::commands::error::AppError::new(
                    vrcast_studio_lib::commands::error::ErrorCode::Internal,
                )
                .with_cause(e)),
            }
        })
        .await
        .expect("the task would not start");

    for _ in 0..100 {
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
    assert!(
        task.error.is_none(),
        "the preparation failed instead of going to the processor: {:?}",
        task.error
    );
    assert!(out.is_file(), "no file came out of the fallback");
    assert_eq!(
        said.lock().unwrap().as_slice(),
        &[DetailCode::NoticeHardwareFailed],
        "the fallback happened and nothing said so — a preparation four times slower with no \
         stated reason is worse than one that failed and explained itself"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
