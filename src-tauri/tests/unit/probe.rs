//! T116 — reading `ffprobe`'s answer.
//!
//! Checked against a **real** answer, taken from a file built on the spot by the bundled
//! FFmpeg (`tests/fixtures/ffprobe-sample.json`), and against invented cases for the
//! subtleties the sample does not hold. Inventing the whole answer will not do: half the
//! traps in it are things you do not expect until you have seen them.

use vrcast_studio_lib::media::probe::{self, ProbeError};

fn sample() -> String {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ffprobe-sample.json");
    std::fs::read_to_string(path).expect("the ffprobe sample answer could not be read")
}

#[test]
fn a_real_answer_is_read_whole() {
    let src = probe::parse(&sample(), "F:/video/sample.mp4").expect("the answer would not parse");

    assert_eq!(src.width, 640);
    assert_eq!(src.height, 360);
    assert_eq!(src.fps, 24);
    assert_eq!(src.video_codec, "h264");
    assert_eq!(src.pix_fmt, "yuv420p");
    assert!(!src.is_hdr());

    assert_eq!(src.audio_tracks.len(), 1);
    let t = &src.audio_tracks[0];
    assert_eq!(t.codec, "aac");
    assert_eq!(t.channels, 2);
    assert_eq!(t.language.as_deref(), Some("rus"));
    assert!(t.is_default);
}

#[test]
fn numbers_arrive_as_strings_and_are_read_all_the_same() {
    // `ffprobe` prints the size, the duration and the bitrate as strings. Trying to read
    // them as numbers would fail to parse the very first file.
    let src = probe::parse(&sample(), "sample.mp4").unwrap();
    assert!(src.size_bytes > 0, "the size was not read");
    assert!(
        src.duration_s > 1.9 && src.duration_s < 2.1,
        "duration {}",
        src.duration_s
    );
    assert!(src.bitrate_bps > 0, "the bitrate was not read");
    assert_eq!(
        src.audio_tracks[0].bitrate_bps,
        Some(128_018),
        "the track's bitrate was not read — and it is needed to decide whether to carry it across"
    );
}

#[test]
fn the_frame_rate_rounds_up() {
    // 24000/1001 is 23.976, that is, 24-frame material. Rounding down would give 23 and
    // understate the compatibility level, and a strict decoder may refuse an understated one.
    let json = answer_with_rate("24000/1001");
    assert_eq!(probe::parse(&json, "x").unwrap().fps, 24);

    let json = answer_with_rate("48000/1001");
    assert_eq!(probe::parse(&json, "x").unwrap().fps, 48);
}

#[test]
fn a_missing_frame_rate_does_not_break_the_parse() {
    // Zero frames per second does not happen; a guess overstates the level, and an
    // overstated one is always safe.
    let json = answer_with_rate("0/0");
    let fps = probe::parse(&json, "x").unwrap().fps;
    assert!(fps > 0, "got {fps} frames per second");
}

#[test]
fn the_language_und_counts_as_missing() {
    // `und` means "not stated" rather than the name of a language. Showing it to a person
    // means offering them a choice between "und" and "und".
    let json = answer_with_tracks(
        r#"
        {"index":1,"codec_type":"audio","codec_name":"aac","channels":2,
         "tags":{"language":"und"},"disposition":{"default":1}},
        {"index":2,"codec_type":"audio","codec_name":"ac3","channels":6,
         "tags":{"language":"eng","title":"Original"},"disposition":{"default":0}}
    "#,
    );
    let src = probe::parse(&json, "x").unwrap();

    assert_eq!(
        src.audio_tracks[0].language, None,
        "\"und\" was taken for a language"
    );
    // What probing actually produces is the data: the index from zero, as ffmpeg counts,
    // and the channel count. Turning that into a caption belongs to the interface, where
    // the wording differs between languages while the numbers do not; it is checked there.
    assert_eq!(src.audio_tracks[0].index, 0);
    assert_eq!(src.audio_tracks[0].channels, 2);
    assert_eq!(src.audio_tracks[1].language.as_deref(), Some("eng"));
}

#[test]
fn tracks_are_numbered_among_the_audio_ones_rather_than_among_all() {
    // The index goes into `-map 0:a:<N>`. Taking the overall stream index means missing the
    // track on any file where the audio does not come first — and it almost never does.
    let json = answer_with_tracks(
        r#"
        {"index":1,"codec_type":"audio","codec_name":"aac","channels":2,
         "disposition":{"default":1}},
        {"index":2,"codec_type":"audio","codec_name":"ac3","channels":6,
         "disposition":{"default":0}},
        {"index":3,"codec_type":"subtitle","codec_name":"subrip"}
    "#,
    );
    let src = probe::parse(&json, "x").unwrap();

    assert_eq!(src.audio_tracks.len(), 2, "subtitles were taken for audio");
    assert_eq!(src.audio_tracks[0].index, 0);
    assert_eq!(src.audio_tracks[1].index, 1);
}

#[test]
fn high_dynamic_range_is_recognised_by_the_colour_transfer() {
    let json = sample().replace(
        r#""pix_fmt": "yuv420p","#,
        r#""pix_fmt": "yuv420p10le","color_transfer": "smpte2084","#,
    );
    let src = probe::parse(&json, "x").unwrap();
    assert!(src.is_hdr(), "HDR was not recognised");
}

#[test]
fn a_file_with_no_video_is_a_trouble_of_its_own() {
    // An audio file instead of a video is an ordinary human mistake, and it must be named
    // outright rather than as a failure to parse.
    let json = r#"{"streams":[{"index":0,"codec_type":"audio","codec_name":"aac","channels":2}],
                   "format":{"duration":"10.0","size":"100","bit_rate":"80"}}"#;
    assert!(matches!(
        probe::parse(json, "x").expect_err("a file with no video was accepted"),
        ProbeError::NoVideo
    ));
}

#[test]
fn rubbish_instead_of_an_answer_does_not_bring_the_application_down() {
    let err = probe::parse("this is not a parser's answer", "x").expect_err("rubbish parsed");
    assert!(matches!(err, ProbeError::Unreadable(_)));
}

// ---------- helpers ----------

fn answer_with_rate(rate: &str) -> String {
    format!(
        r#"{{"streams":[{{"index":0,"codec_type":"video","codec_name":"h264",
             "width":1920,"height":1080,"pix_fmt":"yuv420p","r_frame_rate":"{rate}",
             "avg_frame_rate":"{rate}","bit_rate":"9000000"}},
            {{"index":1,"codec_type":"audio","codec_name":"aac","channels":2,
              "disposition":{{"default":1}}}}],
           "format":{{"duration":"100.0","size":"1000","bit_rate":"9000000"}}}}"#
    )
}

fn answer_with_tracks(tracks: &str) -> String {
    format!(
        r#"{{"streams":[{{"index":0,"codec_type":"video","codec_name":"h264",
             "width":1920,"height":1080,"pix_fmt":"yuv420p","r_frame_rate":"24/1",
             "bit_rate":"9000000"}},{tracks}],
           "format":{{"duration":"100.0","size":"1000","bit_rate":"9000000"}}}}"#
    )
}
