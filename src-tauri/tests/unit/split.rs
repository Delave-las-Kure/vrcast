//! T457, T458, T459 — cutting into pieces, and putting them back.

use std::path::Path;
use vrcast_studio_lib::media::split::{concat_list, cut_args, differs, join_args, Differs, Shape};

fn shape() -> Shape {
    Shape {
        codec: String::from("h264"),
        width: 3840,
        height: 2160,
        pix_fmt: String::from("yuv420p"),
        frame_rate: String::from("24/1"),
        time_base: String::from("1/12288"),
    }
}

#[test]
fn the_pieces_are_copied_rather_than_encoded() {
    // A piece re-encoded on the way out means the converter sees second-generation material
    // before it starts. Nothing here may say anything but copy.
    let args = cut_args(
        Path::new("F:/films/a.mkv"),
        &[60.0, 120.0],
        "F:/work/p%03d.mp4",
    );
    let joined = args.join(" ");
    assert!(joined.contains("-c copy"));
    assert!(!joined.contains("libx264") && !joined.contains("nvenc"));
}

#[test]
fn the_audio_does_not_go_to_the_converter() {
    // T458. Sending it would re-encode the track once per piece and leave a seam at every
    // join — for a tool that does nothing to sound.
    let args = cut_args(Path::new("F:/films/a.mkv"), &[60.0], "F:/work/p%03d.mp4");
    assert!(
        args.iter().any(|a| a == "-an"),
        "the audio was sent along: {args:?}"
    );
}

#[test]
fn every_piece_starts_at_nought() {
    // Without this the third piece carries timestamps beginning two hundred seconds in, and
    // some tools read that as a stream that starts late.
    let args = cut_args(Path::new("F:/films/a.mkv"), &[60.0], "F:/work/p%03d.mp4");
    let joined = args.join(" ");
    assert!(joined.contains("-reset_timestamps 1"));
}

#[test]
fn only_the_first_video_stream_is_cut() {
    // A file with a cover image has two video streams. `-map 0` would put both in every
    // piece and turn the cover into a slideshow.
    let args = cut_args(Path::new("F:/films/a.mkv"), &[60.0], "F:/work/p%03d.mp4");
    let joined = args.join(" ");
    assert!(joined.contains("-map 0:v:0"));
    assert!(!joined.contains("-map 0 "));
}

#[test]
fn the_times_are_written_where_ffmpeg_looks_for_them() {
    let args = cut_args(Path::new("F:/films/a.mkv"), &[60.5, 121.25], "out%03d.mp4");
    let at = args
        .iter()
        .position(|a| a == "-segment_times")
        .expect("no times");
    assert_eq!(args[at + 1], "60.500,121.250");
}

#[test]
fn a_piece_of_another_shape_is_named_rather_than_joined() {
    // **What `concat` does otherwise.** It joins them, the file opens, it plays, and it falls
    // apart at the seam — halfway through, on somebody else's screen, hours after anybody
    // could connect it to this. Refusing costs one message.
    let first = shape();

    let mut other = shape();
    other.codec = String::from("hevc");
    assert_eq!(differs(&first, &other), Some(Differs::Codec));

    let mut other = shape();
    other.width = 1920;
    assert_eq!(differs(&first, &other), Some(Differs::Frame));

    let mut other = shape();
    other.pix_fmt = String::from("yuv420p10le");
    assert_eq!(differs(&first, &other), Some(Differs::PixelFormat));

    let mut other = shape();
    other.frame_rate = String::from("30000/1001");
    assert_eq!(differs(&first, &other), Some(Differs::FrameRate));
}

#[test]
fn a_different_time_base_at_the_same_frame_rate_is_still_caught() {
    // The one nobody would think to check. Two pieces at 24 frames a second with different
    // time bases join into a file whose timestamps drift, and the drift shows as sound
    // sliding out of step minutes later — long after the seam it came from.
    let first = shape();
    let mut other = shape();
    other.time_base = String::from("1/24000");
    assert_eq!(differs(&first, &other), Some(Differs::TimeBase));
}

#[test]
fn pieces_of_the_same_shape_are_joined() {
    // The negative control. "Refuse when they differ" is satisfied by refusing always, and
    // then nothing would ever join.
    assert_eq!(differs(&shape(), &shape()), None);
}

#[test]
fn a_name_with_an_apostrophe_in_it_survives_the_list() {
    // `Assassin's Creed` is enough to break this, and a list that reads as a different file —
    // or as none — fails somewhere far from the apostrophe.
    let list = concat_list(&[String::from("F:/films/Assassin's Creed/p001.mp4")]);
    assert!(list.contains(r"Assassin'\''s Creed"), "{list}");
    assert!(list.starts_with("file '"));
    assert!(list.ends_with("'\n"));
}

#[test]
fn the_audio_comes_back_from_the_original() {
    // T458's other half. The video is the joined pieces; the sound is the film's own, never
    // having been through the converter at all.
    let args = join_args(
        Path::new("F:/work/list.txt"),
        Path::new("F:/films/a.mkv"),
        Path::new("F:/work/out.mp4"),
    );
    let joined = args.join(" ");
    assert!(joined.contains("-map 0:v:0"), "{joined}");
    assert!(joined.contains("-map 1:a?"), "{joined}");
    assert!(joined.contains("-c copy"));
    // Cutting the video to the audio would hide a piece that came back short instead of
    // showing it.
    assert!(!joined.contains("-shortest"), "{joined}");
}
