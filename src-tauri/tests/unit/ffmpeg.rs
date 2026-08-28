//! T115 — checking the bundled FFmpeg.
//!
//! Parsing the answers is checked against recorded strings and always runs. The bundled
//! file itself weighs a hundred and forty megabytes, never reaches the repository and is
//! absent from continuous integration — the checks that need it say honestly that they were
//! skipped rather than pretending to have passed.

use vrcast_studio_lib::media::ffmpeg::{self, FfmpegError};

/// A real answer from the bundled build, shortened to the point.
const VERSION_ANSWER: &str = "\
ffmpeg version n8.1.2-44-g7c533d0f86-20260825 Copyright (c) 2000-2026 the FFmpeg developers
built with gcc 15.2.0 (crosstool-NG 1.28.0.23_185f348)
configuration: --enable-gpl --enable-version3 --enable-libx264 --enable-ffnvcodec
";

/// A piece of a real encoder listing: the same shape the program prints.
const LISTING: &str = "\
Encoders:
 V..... = Video
 ------
 V....D libx264              libx264 H.264 / AVC / MPEG-4 AVC
 V....D libx265              libx265 H.265 / HEVC
 V....D h264_nvenc           NVIDIA NVENC H.264 encoder (codec h264)
 V....D hevc_nvenc           NVIDIA NVENC hevc encoder (codec hevc)
 A....D aac                  AAC (Advanced Audio Coding)
";

#[test]
fn the_version_is_read_out_of_the_answer() {
    let v = ffmpeg::parse_version(VERSION_ANSWER).expect("the version would not read");
    assert!(v.contains("n8.1.2"), "got: {v}");
}

#[test]
fn another_program_under_the_name_ffmpeg_is_not_accepted() {
    // Anything at all may stand behind that name in a system — from a package manager's
    // wrapper to a message saying the program is not installed. Accepting such a thing as
    // FFmpeg means learning the truth halfway through a preparation.
    let answer = "Command 'ffmpeg' not found, but can be installed with:\nsudo apt install ffmpeg";
    let err =
        ffmpeg::parse_version(answer).expect_err("another program's answer passed for a version");
    assert!(matches!(err, FfmpegError::Unexpected(_)));
}

#[test]
fn an_empty_answer_is_not_a_version() {
    let err = ffmpeg::parse_version("   \n\n  ").expect_err("emptiness passed for a version");
    assert!(matches!(err, FfmpegError::Unexpected(_)));
}

#[test]
fn an_encoder_is_looked_for_as_a_whole_word() {
    assert!(ffmpeg::encoder_present(LISTING, "libx264"));
    assert!(ffmpeg::encoder_present(LISTING, "h264_nvenc"));
    assert!(ffmpeg::encoder_present(LISTING, "aac"));
}

#[test]
fn another_name_inside_ours_does_not_count_as_a_match() {
    // This is why the search goes by words rather than by substring. The listing holds
    // `hevc_nvenc`, and a substring search would declare `nvenc` present in general — while
    // what matters is whether the H.264 encoder in particular is there. The fault is silent:
    // the application would decide hardware acceleration was available and fall over as soon
    // as a preparation started.
    assert!(
        !ffmpeg::encoder_present(LISTING, "nvenc"),
        "a piece of another name passed for an encoder"
    );
    assert!(
        !ffmpeg::encoder_present(LISTING, "x264"),
        "\"x264\" was found inside \"libx264\""
    );
    assert!(!ffmpeg::encoder_present(LISTING, "h264_qsv"));
    assert!(!ffmpeg::encoder_present(LISTING, "h264_amf"));
    assert!(!ffmpeg::encoder_present(LISTING, "h264_vaapi"));
}

#[test]
fn a_missing_bundled_file_is_named_as_a_trouble_of_its_own() {
    // Not "it will not start" and not "it answers with the wrong thing": fixing it goes
    // another way, and a person needs the path that was searched.
    let err =
        ffmpeg::locate("no-such-program").expect_err("something that does not exist was found");
    match err {
        FfmpegError::NotFound(searched) => {
            assert!(!searched.is_empty(), "it does not say where it looked");
        }
        other => panic!("a missing file was named otherwise: {other}"),
    }
}

#[tokio::test]
async fn the_bundled_build_can_do_what_it_was_bundled_for() {
    // Needs the file itself. It weighs a hundred and forty megabytes, never reaches the
    // repository and is downloaded by `npm run ffmpeg`; continuous integration has none.
    // The check cannot pass quietly in its absence — then it would mean nothing — so the
    // skip is announced out loud.
    let Ok(path) = ffmpeg::locate("ffmpeg") else {
        eprintln!(
            "SKIPPED: there is no bundled FFmpeg. Run `npm run ffmpeg` so that this check \
             checks something."
        );
        return;
    };
    eprintln!("checking the bundled build: {}", path.display());

    let info = ffmpeg::probe_self()
        .await
        .expect("the bundled build does not answer");

    assert!(
        info.version.contains("ffmpeg version"),
        "a version of an unexpected shape: {}",
        info.version
    );
    assert!(
        info.has_x264,
        "the bundled build has no software H.264 encoder — on a machine without a suitable \
         graphics card there would be nothing to prepare files with"
    );
}

#[tokio::test]
async fn the_bundled_build_is_examined_once_however_often_it_is_asked_about() {
    // `probe_self` starts three programs: `-version`, `-encoders`, `-filters`. It is called
    // from `pick_encoder`, from `convert_preview`, which the preparation screen recomputes
    // on every change to a field. So choosing a file and then changing the audio track ran
    // six FFmpeg processes to learn twice over what cannot change: the build ships beside
    // the application. Eighty milliseconds a time, and on Windows a console window flashing
    // on the desktop for each one.
    //
    // **Calibrated here rather than assumed**, and against a single start rather than
    // against a first call of its own. The first draft timed one call and then another and
    // required the second to be ten times faster — which is only true when this test is the
    // first in the binary to ask. It shares a process with four hundred others and it is
    // not. That check passed on one run and failed on the next, which is the worst thing a
    // check can do.
    use std::time::Instant;
    use vrcast_studio_lib::media::ffmpeg;
    use vrcast_studio_lib::tasks::process::quiet;

    let Ok(ff) = ffmpeg::locate("ffmpeg") else {
        eprintln!("SKIPPED: no bundled FFmpeg. Run `npm run ffmpeg` for this to check anything.");
        return;
    };

    // Whatever went before, the answer is known by now.
    let first = ffmpeg::probe_self().await.expect("the build would not answer");

    // What one process start costs on this machine, right now.
    let started = Instant::now();
    let _ = quiet(&ff)
        .args(["-hide_banner", "-version"])
        .output()
        .await
        .expect("the bundled build would not run");
    let one_start = started.elapsed();

    let started = Instant::now();
    let second = ffmpeg::probe_self().await.expect("the build would not answer twice");
    let asking_again = started.elapsed();

    assert_eq!(first, second, "the same build described two different ways");
    assert!(
        asking_again * 5 < one_start,
        "asking again cost {asking_again:?}, against {one_start:?} for a single process          start — so the three of them were started all over again"
    );
}
