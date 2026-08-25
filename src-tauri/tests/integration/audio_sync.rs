//! T114 — sound must not drift away from the picture (FR-024).
//!
//! Measured from timestamps, never by ear. A drift of a couple of hundredths of
//! a second is plainly visible on lip movement and completely invisible in any
//! summary of the file: duration, frame count and bitrate all stay right.
//!
//! ## What is measured, and why that
//!
//! The source is built with the video deliberately starting one frame after the
//! audio — a real shape, seen in this project on upscaler output. What has to
//! survive conversion is the **distance between the two streams**. Re-encoding is
//! free to move both, and does; what it must not do is move one without the other.
//!
//! ## What this does NOT prove, measured rather than assumed
//!
//! It does not prove that `aresample=async=1:first_pts=0` is doing anything here.
//! Encoding this same source with and without that filter was compared: the two
//! outputs are identical down to the first audio packet, priming and edit list
//! included. The filter earns its place on other material — that is what the
//! project's own notes record — but on a synthetic clip it changes nothing, and
//! claiming otherwise would be a check that proves what it merely asserts.

use std::path::Path;

/// Distance between the start of video and the start of audio, in seconds.
///
/// Read from the container rather than measured while playing: this is exactly
/// what a player uses to line the two up, and it is the same on every run.
fn av_offset(ffprobe: &Path, file: &Path) -> f64 {
    let out = std::process::Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type,start_time",
            "-of",
            "csv=p=0",
        ])
        .arg(file)
        .output()
        .expect("could not run the bundled ffprobe");
    assert!(out.status.success(), "ffprobe refused {}", file.display());

    let text = String::from_utf8_lossy(&out.stdout);
    let mut video = None;
    let mut audio = None;
    for line in text.lines() {
        let mut parts = line.trim().split(',');
        let kind = parts.next().unwrap_or_default();
        let start: f64 = parts.next().unwrap_or("0").parse().unwrap_or(0.0);
        match kind {
            "video" => video = Some(start),
            "audio" => audio = Some(start),
            _ => {}
        }
    }
    video.expect("no video stream") - audio.expect("no audio stream")
}

#[tokio::test]
async fn conversion_keeps_the_sound_where_it_was() {
    use std::sync::Arc;
    use vrcast_studio_lib::commands::convert::{api as convert, ConvertStart};
    use vrcast_studio_lib::commands::AppState;
    use vrcast_studio_lib::store::db::Db;
    use vrcast_studio_lib::store::secrets::InMemorySecretStore;

    let (Ok(ff), Ok(fp)) = (
        vrcast_studio_lib::media::ffmpeg::locate("ffmpeg"),
        vrcast_studio_lib::media::ffmpeg::locate("ffprobe"),
    ) else {
        eprintln!(
            "SKIPPED: no bundled FFmpeg. Run `npm run ffmpeg` for this check to check anything."
        );
        return;
    };

    let dir = std::env::temp_dir().join(format!("vrcast-sync-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).expect("could not make a working directory");
    let source = dir.join("source.mp4");
    let out = dir.join("ready.mp4");

    // Video one frame behind the audio at 24 fps, and six-channel AC-3 so the
    // audio genuinely has to be re-encoded — a copy would prove nothing.
    let made = std::process::Command::new(&ff)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-itsoffset",
            "0.041666",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x240:rate=24",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            "3",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "ac3",
            "-ac",
            "6",
        ])
        .arg(&source)
        .output()
        .expect("could not run the bundled FFmpeg");
    assert!(made.status.success(), "could not prepare a source clip");

    let before = av_offset(&fp, &source);
    assert!(
        before.abs() > 0.02,
        "the source has no offset to preserve ({before:.6} s) — the check would prove nothing"
    );

    let state = AppState::with_db(
        Arc::new(Db::open_in_memory().unwrap()),
        Arc::new(InMemorySecretStore::new()),
    )
    .expect("could not assemble the application state");

    let task = convert::convert_start(
        &state,
        ConvertStart {
            path: source.to_string_lossy().into_owned(),
            audio_track: 0,
            target_kbps: None,
            height: None,
            out_path: out.to_string_lossy().into_owned(),
            prefer_hardware: false,
        },
    )
    .await
    .expect("the conversion did not start");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    loop {
        let record = state.tasks.get(&task).ok().flatten();
        if record.as_ref().is_some_and(|t| t.state.is_final()) {
            let record = record.unwrap();
            assert_eq!(
                record.state,
                vrcast_studio_lib::tasks::state::TaskState::Completed,
                "the conversion did not succeed: {:?}",
                record.error
            );
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the conversion did not finish in time"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    let after = av_offset(&fp, &out);
    let drift = (after - before).abs();

    // Five milliseconds. Not a frame: the values come from the container and are
    // the same on every run, so a frame-wide tolerance would sail past the very
    // defect this exists to catch — a timestamp reset moves the two streams
    // roughly twenty milliseconds apart, half a frame, and that is plainly
    // visible on lip movement.
    assert!(
        drift < 0.005,
        "sound moved relative to picture by {drift:.6} s during conversion \
         (source {before:.6} s, result {after:.6} s)"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
