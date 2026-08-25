//! T034a — tests for parsing an MP4 header (R-19, FR-012).
//!
//! The reference files are real ones: built by ffmpeg and kept in `tests/fixtures/mp4`.
//! That is a matter of principle — parsing home-made blanks would check the code's agreement
//! with our own notions of the format rather than with what really arrives from a server.
//! There are blanks too, but only for the cases ffmpeg does not produce: eight-byte length
//! fields and deliberately corrupted data.

use vrcast_studio_lib::domain::moov::{self, MoovOutcome};

fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/mp4")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()))
}

/// Find a top-level atom's bounds — to cut the file exactly where needed.
fn top_level_box(data: &[u8], want: &[u8; 4]) -> Option<(usize, usize)> {
    let mut off = 0usize;
    while off + 8 <= data.len() {
        let size = u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]);
        let typ = &data[off + 4..off + 8];
        let len = match size {
            0 => data.len() - off,
            1 => u64::from_be_bytes(data[off + 8..off + 16].try_into().ok()?) as usize,
            n => n as usize,
        };
        if typ == want {
            return Some((off, off + len));
        }
        if len < 8 {
            return None;
        }
        off += len;
    }
    None
}

// ---------- real files ----------

#[test]
fn a_file_prepared_for_serving_parses_whole() {
    let data = fixture("faststart.mp4");
    let outcome = moov::parse(&data, Some(data.len() as u64));

    let params = match &outcome {
        MoovOutcome::Parsed(p) => p,
        other => panic!("the header was not parsed: {other:?}"),
    };

    assert_eq!(params.width, Some(128));
    assert_eq!(params.height, Some(96));
    assert_eq!(params.video_codec.as_deref(), Some("h264"));
    assert_eq!(params.audio_codec.as_deref(), Some("aac"));

    let duration = params.duration_s.expect("the duration was not read");
    assert!(
        (duration - 1.0).abs() < 0.05,
        "duration {duration} instead of about 1 s"
    );

    // The average bitrate is the size divided by the duration. Compared against the value
    // ffprobe gives for this same file: 112144.
    let bitrate = params.bitrate_bps.expect("the bitrate was not worked out");
    assert!(
        (bitrate as i64 - 112_144).abs() < 2_000,
        "bitrate {bitrate} disagrees with what ffprobe counts"
    );

    assert_eq!(outcome.faststart_ok(), Some(true));
}

#[test]
fn a_file_with_its_header_at_the_end_is_recognised_as_unfit() {
    // FR-012: the parameters stay unknown, but a person learns the main thing — a viewer
    // will only start watching such a file after downloading its tail.
    let data = fixture("moov_at_end.mp4");
    let outcome = moov::parse(&data, Some(data.len() as u64));

    assert_eq!(
        outcome,
        MoovOutcome::MoovAfterData,
        "a file with no preparation passed for a prepared one"
    );
    assert_eq!(outcome.faststart_ok(), Some(false));
    assert!(
        outcome.params().is_none(),
        "parameters were handed out that there is nowhere to take from"
    );
}

#[test]
fn the_verdict_on_an_unfit_file_is_reached_from_the_beginning_rather_than_the_whole_file() {
    // An important property: `mdat` runs to gigabytes, and reading it through for the sake
    // of a header is senseless. The first few kilobytes must be enough.
    let data = fixture("moov_at_end.mp4");
    let (mdat_start, _) = top_level_box(&data, b"mdat").expect("the file has no mdat");
    let head = &data[..mdat_start + 64];

    assert_eq!(
        moov::parse(head, Some(data.len() as u64)),
        MoovOutcome::MoovAfterData
    );
}

#[test]
fn a_truncated_header_says_how_many_bytes_were_short() {
    // This is not decoration for a message: the reading layer asks for exactly the piece
    // this number names rather than blindly doubling the size.
    let data = fixture("faststart.mp4");
    let (moov_start, moov_end) = top_level_box(&data, b"moov").expect("the file has no moov");
    let cut = moov_start + (moov_end - moov_start) / 2;

    match moov::parse(&data[..cut], Some(data.len() as u64)) {
        MoovOutcome::NeedMoreBytes { need } => assert_eq!(
            need, moov_end as u64,
            "the number of bytes asked for is not the size of the header"
        ),
        other => panic!("a truncated header was parsed as {other:?}"),
    }
}

#[test]
fn without_the_file_size_everything_but_the_bitrate_parses() {
    // The size is sometimes unknown: a directory listing may have arrived without it. That
    // is no reason to give up the resolution and the codecs.
    let data = fixture("faststart.mp4");
    let params = match moov::parse(&data, None) {
        MoovOutcome::Parsed(p) => p,
        other => panic!("the header was not parsed: {other:?}"),
    };

    assert_eq!(params.width, Some(128));
    assert!(params.duration_s.is_some());
    assert_eq!(
        params.bitrate_bps, None,
        "the bitrate was worked out from nowhere: without the file size it cannot be"
    );
}

// ---------- blanks for the cases ffmpeg does not produce ----------

fn mp4_box(typ: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(payload.len() + 8);
    v.extend_from_slice(&((payload.len() + 8) as u32).to_be_bytes());
    v.extend_from_slice(typ);
    v.extend_from_slice(payload);
    v
}

#[test]
fn the_duration_is_read_from_eight_byte_fields_too() {
    // Version one of the movie header: the times and the duration are eight bytes each. It
    // turns up on long recordings, and muddled offsets would give not an error but a
    // plausible wrong number — the worst outcome.
    let mut mvhd = vec![1u8, 0, 0, 0]; // version 1, flags
    mvhd.extend_from_slice(&[0u8; 8]); // creation time
    mvhd.extend_from_slice(&[0u8; 8]); // modification time
    mvhd.extend_from_slice(&1000u32.to_be_bytes()); // ticks per second
    mvhd.extend_from_slice(&90_000u64.to_be_bytes()); // duration

    let mut file = mp4_box(b"ftyp", b"isom\0\0\x02\0isomiso2");
    file.extend_from_slice(&mp4_box(b"moov", &mp4_box(b"mvhd", &mvhd)));

    match moov::parse(&file, Some(1_000_000)) {
        MoovOutcome::Parsed(p) => {
            let d = p.duration_s.expect("the duration was not read");
            assert!((d - 90.0).abs() < 0.001, "duration {d} instead of 90 s");
            assert_eq!(
                p.bitrate_bps,
                Some(88_889),
                "the bitrate was worked out wrongly"
            );
        }
        other => panic!("the blank was not parsed: {other:?}"),
    }
}

#[test]
fn an_unknown_duration_does_not_turn_into_zero() {
    // The "unknown" mark in a header is all ones. Taking it for a number means showing a
    // person a duration of 49 days and a bitrate of a few bits.
    let mut mvhd = vec![0u8, 0, 0, 0];
    mvhd.extend_from_slice(&[0u8; 4]);
    mvhd.extend_from_slice(&[0u8; 4]);
    mvhd.extend_from_slice(&1000u32.to_be_bytes());
    mvhd.extend_from_slice(&u32::MAX.to_be_bytes());

    let mut file = mp4_box(b"ftyp", b"isom");
    file.extend_from_slice(&mp4_box(b"moov", &mp4_box(b"mvhd", &mvhd)));

    match moov::parse(&file, Some(1_000_000)) {
        MoovOutcome::Parsed(p) => {
            assert_eq!(
                p.duration_s, None,
                "the \"unknown\" mark was taken for a number"
            );
            assert_eq!(p.bitrate_bps, None);
        }
        other => panic!("the blank was not parsed: {other:?}"),
    }
}

#[test]
fn a_tick_rate_of_zero_does_not_break_the_parse_by_dividing_by_zero() {
    let mut mvhd = vec![0u8, 0, 0, 0];
    mvhd.extend_from_slice(&[0u8; 4]);
    mvhd.extend_from_slice(&[0u8; 4]);
    mvhd.extend_from_slice(&0u32.to_be_bytes()); // ticks per second: zero
    mvhd.extend_from_slice(&1000u32.to_be_bytes());

    let mut file = mp4_box(b"ftyp", b"isom");
    file.extend_from_slice(&mp4_box(b"moov", &mp4_box(b"mvhd", &mvhd)));

    match moov::parse(&file, Some(1000)) {
        MoovOutcome::Parsed(p) => assert_eq!(p.duration_s, None),
        other => panic!("the blank was not parsed: {other:?}"),
    }
}

// ---------- corrupted data ----------

#[test]
fn rubbish_is_not_taken_for_video() {
    assert_eq!(moov::parse(b"", None), MoovOutcome::NotMp4);
    assert_eq!(moov::parse(b"\x00\x00", None), MoovOutcome::NotMp4);
    assert_eq!(
        moov::parse(b"<!DOCTYPE html><html><body>404</body></html>", None),
        MoovOutcome::NotMp4,
        "a server's error page was taken for video"
    );
}

#[test]
fn an_atom_of_zero_length_does_not_loop_the_parse() {
    // The parsing runs over data from a server, and a file may be put together any way at
    // all — including so that the walk never moves a single byte.
    let mut file = mp4_box(b"ftyp", b"isom");
    file.extend_from_slice(&[0, 0, 0, 4]); // a length shorter than the header itself
    file.extend_from_slice(b"junk");
    file.extend_from_slice(&[0u8; 64]);

    // What is checked is precisely that it ends: should the parse loop, the test never does.
    let outcome = moov::parse(&file, Some(file.len() as u64));
    assert_eq!(outcome, MoovOutcome::NotMp4);
}

#[test]
fn a_header_promising_more_than_there_is_does_not_read_past_the_piece() {
    // The atom declares a length of a gigabyte while there are a hundred bytes of data.
    // Indexing by the declared length would run past the end of the slice.
    let mut file = mp4_box(b"ftyp", b"isom");
    file.extend_from_slice(&1_000_000_000u32.to_be_bytes());
    file.extend_from_slice(b"moov");
    file.extend_from_slice(&[0u8; 32]);

    match moov::parse(&file, Some(2_000_000_000)) {
        MoovOutcome::NeedMoreBytes { need } => {
            assert!(
                need > file.len() as u64,
                "less was asked for than is already there"
            );
        }
        other => panic!("a request for data was expected, got {other:?}"),
    }
}

#[test]
fn a_file_with_no_header_at_all_does_not_ask_for_bytes_past_its_end() {
    // Otherwise it becomes an endless circle: the parse asks for data past the end of the
    // file, the reading layer hands back the same piece, the parse asks again. The file was
    // read whole — so it holds no header, and that is the final answer.
    let mut file = mp4_box(b"ftyp", b"isom");
    file.extend_from_slice(&mp4_box(b"free", &[0u8; 16]));
    let size = file.len() as u64;

    assert_eq!(
        moov::parse(&file, Some(size)),
        MoovOutcome::NotMp4,
        "bytes past the end of the file were asked for — the reading layer will loop"
    );

    // And when the file size is unknown, asking for more is legitimate: there may be more.
    assert!(matches!(
        moov::parse(&file, None),
        MoovOutcome::NeedMoreBytes { .. }
    ));
}

#[test]
fn the_piece_asked_for_is_always_enough_to_move_on() {
    // The property reading-more leans on: however many times the reading layer grants the
    // request, the parse must reach an answer rather than asking for the same thing over and
    // over.
    let data = fixture("faststart.mp4");
    let size = data.len() as u64;

    let mut have = 8usize;
    let mut steps = 0;
    loop {
        steps += 1;
        assert!(steps < 10, "the parse did not converge in ten reads");

        let head = &data[..have.min(data.len())];
        match moov::parse(head, Some(size)) {
            MoovOutcome::NeedMoreBytes { need } => {
                assert!(
                    need > have as u64,
                    "{need} bytes asked for while {have} are already read — no progress"
                );
                have = need as usize;
            }
            MoovOutcome::Parsed(_) => break,
            other => panic!("an unexpected outcome: {other:?}"),
        }
    }
}

#[test]
fn the_parse_survives_every_truncation_of_a_real_file() {
    // A property that matters more than any single case: a piece may arrive cut off at an
    // arbitrary place — on a field boundary, in the middle of an atom's name, anywhere. Not
    // one of those truncations may bring the application down.
    let data = fixture("faststart.mp4");
    for cut in (0..data.len()).step_by(7) {
        let _ = moov::parse(&data[..cut], Some(data.len() as u64));
    }
    for cut in (0..data.len()).step_by(7) {
        let _ = moov::parse(&data[..cut], None);
    }
}
