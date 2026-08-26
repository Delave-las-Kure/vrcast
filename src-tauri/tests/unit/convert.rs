//! T117, T118 — the FFmpeg command line (FR-022, FR-023, FR-024).
//!
//! Every flag checked here was bought with a bug in this project, and each one
//! fails quietly when it is wrong: the file encodes, looks fine, and misbehaves
//! only at the viewer. That is why the command is built by a pure function —
//! so it can be inspected without encoding anything.

use vrcast_studio_lib::domain::convert_plan::{self as plan, ConvertRequest};
use vrcast_studio_lib::domain::source::{AudioTrack, SourceFile};
use vrcast_studio_lib::media::convert::{self, ConvertJob};
use vrcast_studio_lib::media::encoders::Encoder;

fn track(codec: &str, channels: u16) -> AudioTrack {
    AudioTrack {
        index: 0,
        codec: String::from(codec),
        channels,
        bitrate_bps: None,
        language: Some(String::from("rus")),
        title: None,
        is_default: true,
    }
}

/// A source that already matches the target format.
fn compatible() -> SourceFile {
    SourceFile {
        path: String::from("/video/source.mp4"),
        size_bytes: 8_000_000_000,
        duration_s: 7200.0,
        width: 1920,
        height: 1080,
        fps: 24,
        bitrate_bps: 9_000_000,
        peak_bps: None,
        video_codec: String::from("h264"),
        pix_fmt: String::from("yuv420p"),
        color_transfer: Some(String::from("bt709")),
        audio_tracks: vec![track("aac", 2)],
    }
}

fn as_is() -> ConvertRequest {
    ConvertRequest {
        audio_track: 0,
        target_kbps: None,
        height: None,
    }
}

/// Build the command for a source and a request.
fn args_for(source: &SourceFile, request: &ConvertRequest, encoder: &Encoder) -> Vec<String> {
    let plan = plan::plan(source, request).expect("the plan did not come together");
    convert::build_args(&ConvertJob {
        source,
        plan: &plan,
        encoder,
        out_path: "/video/ready.mp4",
    })
}

/// Is `flag` followed by `value` anywhere in the command?
///
/// Position matters, not mere presence: FFmpeg reads a value from whatever comes
/// next, so a flag with the wrong neighbour is a different command entirely.
fn has_pair(args: &[String], flag: &str, value: &str) -> bool {
    args.windows(2).any(|w| w[0] == flag && w[1] == value)
}

fn has(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

/// The value that follows `flag`.
fn value_of<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|w| w[0] == flag)
        .map(|w| w[1].as_str())
}

// ---------- copying versus re-encoding ----------

#[test]
fn compatible_streams_are_copied() {
    let args = args_for(&compatible(), &as_is(), &Encoder::Software);
    assert!(
        has_pair(&args, "-c:v", "copy"),
        "video is re-encoded: {args:?}"
    );
    assert!(
        has_pair(&args, "-c:a", "copy"),
        "audio is re-encoded: {args:?}"
    );
    // Nothing to filter when nothing is being changed — and a filter would force
    // a decode, quietly turning a copy into a re-encode.
    assert!(
        !has(&args, "-vf"),
        "a filter was added to a plain copy: {args:?}"
    );
}

#[test]
fn re_encoded_audio_always_gets_the_sync_fix() {
    // FR-024. AAC records its priming samples through an edit list, and the
    // VRChat player ignores edit lists — without this the sound drifts away from
    // the picture. Nothing about the file looks wrong; you only hear it.
    let mut source = compatible();
    source.audio_tracks = vec![track("eac3", 6)];

    let args = args_for(&source, &as_is(), &Encoder::Software);
    assert!(has_pair(&args, "-c:a", "aac"));
    assert_eq!(
        value_of(&args, "-af"),
        Some("aresample=async=1:first_pts=0"),
        "audio is re-encoded without the sync fix: {args:?}"
    );
    assert!(
        has_pair(&args, "-ac", "2"),
        "audio is not downmixed to stereo"
    );
}

#[test]
fn copied_audio_gets_no_filter() {
    // A filter would force a decode, and a decoded-then-re-encoded track is a lost
    // generation for nothing.
    let args = args_for(&compatible(), &as_is(), &Encoder::Software);
    assert!(!has(&args, "-af"), "a filter was applied to a copied track");
}

// ---------- what the viewer would notice ----------

#[test]
fn the_index_is_written_at_the_front() {
    // FR-023. Without this the player must download the tail before it can start,
    // and seeking over plain HTTP is impossible.
    let args = args_for(&compatible(), &as_is(), &Encoder::Software);
    assert!(
        has_pair(&args, "-movflags", "+faststart"),
        "the index is left at the end of the file: {args:?}"
    );
}

#[test]
fn streams_are_chosen_explicitly() {
    // Left to itself FFmpeg picks differently on files with several video streams
    // — cover art counts as one — and the result is a still image with sound.
    let mut source = compatible();
    source.audio_tracks = vec![track("aac", 2), track("aac", 2)];
    source.audio_tracks[1].index = 1;
    source.audio_tracks[1].is_default = false;

    let request = ConvertRequest {
        audio_track: 1,
        ..as_is()
    };
    let args = args_for(&source, &request, &Encoder::Software);

    assert!(has_pair(&args, "-map", "0:v:0"), "video stream not pinned");
    assert!(
        has_pair(&args, "-map", "0:a:1"),
        "the chosen audio track is not the one being mapped: {args:?}"
    );
}

#[test]
fn stdin_is_never_read() {
    // Without this FFmpeg grabs the terminal's input, and a background job can
    // wait forever for a keypress nobody will make.
    let args = args_for(&compatible(), &as_is(), &Encoder::Software);
    assert!(has(&args, "-nostdin"));
}

// ---------- bitrate and peaks ----------

#[test]
fn the_buffer_matches_the_ceiling() {
    // A larger buffer lets bursts run above the ceiling, and that is what froze
    // viewers: a ceiling of 45 with a buffer of 60 produced 54 Mbit/s peaks.
    let request = ConvertRequest {
        target_kbps: Some(8_000),
        ..as_is()
    };
    let args = args_for(&compatible(), &request, &Encoder::Software);

    let maxrate = value_of(&args, "-maxrate").expect("no ceiling was set");
    let bufsize = value_of(&args, "-bufsize").expect("no buffer was set");
    assert_eq!(maxrate, bufsize, "buffer and ceiling disagree: {args:?}");
    assert_eq!(maxrate, "8800k", "the ceiling was computed in megabits");
}

#[test]
fn no_bitrate_floor_is_set() {
    // A floor pads easy scenes with bits they do not need — that is constant
    // bitrate behaviour, and constant bitrate lost the measurement.
    let request = ConvertRequest {
        target_kbps: Some(8_000),
        ..as_is()
    };
    let args = args_for(&compatible(), &request, &Encoder::Software);
    assert!(!has(&args, "-minrate"), "a bitrate floor was set: {args:?}");
}

#[test]
fn a_keyframe_every_second_at_any_frame_rate() {
    // A constant would be wrong here: 48 was written for 48 fps material and meant
    // "once a second", but on 24 fps it means once every two.
    for fps in [24u32, 25, 30, 48] {
        let mut source = compatible();
        source.fps = fps;
        source.video_codec = String::from("hevc"); // force a re-encode
        let args = args_for(&source, &as_is(), &Encoder::Software);
        assert_eq!(
            value_of(&args, "-g"),
            Some(fps.to_string().as_str()),
            "at {fps} fps the keyframe interval is not one second"
        );
    }
}

// ---------- encoders ----------

#[test]
fn the_chosen_encoder_is_the_one_used() {
    let mut source = compatible();
    source.video_codec = String::from("hevc");

    let hardware = Encoder::Hardware {
        name: String::from("h264_nvenc"),
    };
    let args = args_for(&source, &as_is(), &hardware);
    assert!(has_pair(&args, "-c:v", "h264_nvenc"), "{args:?}");

    let args = args_for(&source, &as_is(), &Encoder::Software);
    assert!(has_pair(&args, "-c:v", "libx264"), "{args:?}");
}

#[test]
fn quality_is_pinned_the_way_each_encoder_understands() {
    // `-crf` means nothing to the hardware encoders and `-cq` means nothing to
    // x264. Passing the wrong one is NOT an error: tried against the bundled
    // build, FFmpeg accepted `-cq` on x264 without a word and encoded at whatever
    // bitrate it liked. So the live encode below cannot catch this — these
    // assertions are the only thing that can.
    let mut source = compatible();
    source.video_codec = String::from("hevc");

    let args = args_for(&source, &as_is(), &Encoder::Software);
    assert!(has(&args, "-crf") && !has(&args, "-cq"), "{args:?}");

    let hardware = Encoder::Hardware {
        name: String::from("h264_nvenc"),
    };
    let args = args_for(&source, &as_is(), &hardware);
    assert!(has(&args, "-cq") && !has(&args, "-crf"), "{args:?}");
    // `-preset slow` is an x264 preset, and it must not reach hardware. The option name
    // itself is not the test: asked of the bundled build on 2026-08-26, `h264_nvenc` has a
    // `-preset` of its own running p1 to p7. What must not happen is one encoder being
    // handed another's dialect.
    assert!(
        !args.iter().any(|a| a == "slow"),
        "an x264 preset value was given to hardware: {args:?}"
    );

    // And the other two makes of card, which used to be handed NVIDIA's option and would
    // refuse to start on it. Asked of the bundled build the same day: `h264_amf` has no
    // `-cq` at all and pins quality with `-rc cqp -qp_i`; `h264_qsv` uses
    // `-global_quality`. This check could not be made before there was one place that knew
    // the dialects.
    let amd = Encoder::Hardware {
        name: String::from("h264_amf"),
    };
    let args = args_for(&source, &as_is(), &amd);
    assert!(
        has(&args, "-qp_i") && !has(&args, "-cq") && !has(&args, "-crf"),
        "AMD was handed somebody else's option: {args:?}"
    );

    let intel = Encoder::Hardware {
        name: String::from("h264_qsv"),
    };
    let args = args_for(&source, &as_is(), &intel);
    assert!(
        has(&args, "-global_quality") && !has(&args, "-cq") && !has(&args, "-crf"),
        "Intel was handed somebody else's option: {args:?}"
    );
}

// ---------- filters ----------

#[test]
fn a_downscale_keeps_the_aspect_ratio_and_even_dimensions() {
    // H.264 in yuv420p cannot encode odd dimensions at all, and `-1` produces them
    // on plenty of real sources.
    let request = ConvertRequest {
        height: Some(720),
        ..as_is()
    };
    let args = args_for(&compatible(), &request, &Encoder::Software);
    let filter = value_of(&args, "-vf").expect("no filter for a resize");
    assert!(filter.contains("scale=-2:720"), "got: {filter}");
}

#[test]
fn high_dynamic_range_is_brought_down_and_the_format_is_fixed_last() {
    // The pixel format must come after everything that could have changed it;
    // put earlier, the tonemapper undoes it and the file stays 10-bit.
    let mut source = compatible();
    source.color_transfer = Some(String::from("smpte2084"));

    let args = args_for(&source, &as_is(), &Encoder::Software);
    let filter = value_of(&args, "-vf").expect("no filter for high dynamic range");

    assert!(filter.contains("tonemap"), "no tonemapping: {filter}");
    assert!(
        filter.ends_with("format=yuv420p"),
        "the pixel format is not the last step: {filter}"
    );
}

#[test]
fn only_one_filter_argument_is_ever_passed() {
    // FFmpeg accepts a single `-vf`; a second one silently replaces the first
    // rather than adding to it, so a tonemap plus a resize would lose one of them.
    let mut source = compatible();
    source.color_transfer = Some(String::from("smpte2084"));
    let request = ConvertRequest {
        height: Some(720),
        ..as_is()
    };
    let args = args_for(&source, &request, &Encoder::Software);

    assert_eq!(
        args.iter().filter(|a| *a == "-vf").count(),
        1,
        "more than one filter argument: {args:?}"
    );
    let filter = value_of(&args, "-vf").unwrap();
    assert!(
        filter.contains("tonemap") && filter.contains("scale=-2:720"),
        "{filter}"
    );
}

// ---------- progress ----------

#[test]
fn progress_is_read_in_microseconds() {
    // FFmpeg prints `out_time_ms` holding microseconds — a long-standing mistake
    // upstream, not a different unit. Reading it as milliseconds puts progress a
    // thousand times off, and the bar sits at zero for the whole encode.
    assert_eq!(
        convert::progress_position("out_time_us=1500000"),
        Some(1_500_000)
    );
    assert_eq!(
        convert::progress_position("out_time_ms=1500000"),
        Some(1_500_000)
    );
}

#[test]
fn other_progress_lines_are_ignored() {
    assert_eq!(convert::progress_position("frame=120"), None);
    assert_eq!(convert::progress_position("progress=continue"), None);
    assert_eq!(convert::progress_position("nonsense"), None);
}

// ---------- does FFmpeg actually accept this? ----------

/// Encode a short clip for real.
///
/// Catches what string assertions cannot: a command that reads well and that
/// FFmpeg refuses — an unparseable filter chain, an option in the wrong place,
/// a codec that cannot take the pixel format it was handed.
///
/// **What it does NOT catch, measured rather than assumed:** an option the encoder
/// does not understand is *silently ignored*, not refused. Handing `-cq` to x264
/// was tried here on purpose and FFmpeg encoded happily, producing a file at
/// whatever bitrate it liked. That whole class is caught only by the string
/// assertions above — which is why they are not redundant with this check, and
/// why removing them would leave nothing guarding the quality settings.
///
/// Needs the bundled FFmpeg. Without it the check says so out loud instead of
/// quietly passing.
#[test]
fn the_command_is_one_ffmpeg_accepts() {
    use vrcast_studio_lib::media::ffmpeg;

    let (Ok(ff), Ok(_)) = (ffmpeg::locate("ffmpeg"), ffmpeg::locate("ffprobe")) else {
        eprintln!(
            "SKIPPED: no bundled FFmpeg. Run `npm run ffmpeg` for this check to check anything."
        );
        return;
    };

    let dir = std::env::temp_dir().join(format!("vrcast-encode-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).expect("could not make a working directory");
    let src = dir.join("source.mp4");
    let out = dir.join("ready.mp4");

    // A two-second clip in a deliberately wrong format, so every branch of the
    // command has something to do: re-encode the video, downmix the audio, resize.
    let made = std::process::Command::new(&ff)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=640x360:rate=24",
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
            "ac3",
            "-ac",
            "6",
        ])
        .arg(&src)
        .output()
        .expect("could not run the bundled FFmpeg");
    assert!(made.status.success(), "could not prepare a source clip");

    let source = {
        let mut s = compatible();
        s.path = src.to_string_lossy().into_owned();
        s.width = 640;
        s.height = 360;
        s.duration_s = 2.0;
        s.video_codec = String::from("h264");
        s.audio_tracks = vec![track("ac3", 6)];
        s
    };
    let request = ConvertRequest {
        audio_track: 0,
        target_kbps: Some(2_000),
        height: Some(240),
    };
    let plan = plan::plan(&source, &request).expect("the plan did not come together");
    let args = convert::build_args(&ConvertJob {
        source: &source,
        plan: &plan,
        encoder: &Encoder::Software,
        out_path: &out.to_string_lossy(),
    });

    let done = std::process::Command::new(&ff)
        .args(&args)
        .output()
        .expect("could not run the bundled FFmpeg");

    assert!(
        done.status.success(),
        "FFmpeg refused the command we build:\n{}\n\nargs: {args:?}",
        String::from_utf8_lossy(&done.stderr)
    );
    assert!(out.exists(), "FFmpeg reported success but produced no file");

    let _ = std::fs::remove_dir_all(&dir);
}
