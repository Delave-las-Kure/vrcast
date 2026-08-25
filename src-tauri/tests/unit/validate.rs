//! T120 — telling a broken file from a noisy one (FR-027).
//!
//! The messages here are the real ones, copied from this project's own runs.
//! Inventing them would defeat the purpose: the whole difficulty is that a good
//! file and a broken one both produce output on stderr, and only the wording
//! tells them apart.

use vrcast_studio_lib::media::validate;

#[test]
fn a_clean_decode_passes() {
    let v = validate::classify("");
    assert!(v.ok);
    assert!(v.problems.is_empty());
    assert!(v.ignored.is_empty());
}

#[test]
fn muxer_timestamp_noise_does_not_condemn_a_good_file() {
    // The message that made the naive check unusable. It comes from the null
    // muxer, which insists on monotonic timestamps; plenty of real sources have
    // duplicate DTS and decode perfectly. Every file from one supplier used here
    // has that defect, and it survives re-encoding — so counting it as failure
    // would reject an entire library of working files.
    let stderr = "[null @ 000001d4] Application provided invalid, non monotonically \
                  increasing dts to muxer in stream 0: 47 >= 47";

    let v = validate::classify(stderr);
    assert!(v.ok, "a good file was rejected over muxer noise");
    // Kept rather than dropped: the complaint is shown to a person as something
    // deliberately forgiven. Silently swallowing it would leave whoever wonders
    // later why a warning-laden file was accepted with nothing to answer them.
    assert_eq!(v.ignored.len(), 1, "the complaint was silently dropped");
    assert!(
        v.ignored[0].contains("non monotonically increasing dts"),
        "the muxer's own words were lost: {:?}",
        v.ignored
    );
}

#[test]
fn a_decoder_complaint_fails_the_file() {
    // This is the class the rule was written for: an encoder orphaned mid-write
    // leaves a file that opens, reports the right duration, and falls apart where
    // someone is watching.
    let stderr = "[h264 @ 000001d4] Invalid NAL unit size (-56 > 271).";

    let v = validate::classify(stderr);
    assert!(!v.ok, "a broken file passed");
    assert_eq!(v.problems.len(), 1);
    assert!(
        v.problems[0].contains("Invalid NAL unit size"),
        "the decoder's own words were lost: {:?}",
        v.problems
    );
}

#[test]
fn audio_problems_count_too() {
    // The known workaround for the muxer trap drops audio entirely. That would
    // leave a silent-but-broken track undetected, so audio is decoded here and
    // its complaints count.
    let v = validate::classify("[aac @ 000001d4] channel element 0.0 is not allocated");
    assert!(!v.ok, "a broken audio track passed");
}

#[test]
fn noise_and_a_real_problem_together_still_fail() {
    // The dangerous case: a file that has both. Deciding on the first line, or on
    // "did anything complain at all", gets this wrong in one direction or the other.
    let stderr = "[null @ 000001d4] Application provided invalid, non monotonically increasing dts to muxer in stream 0: 47 >= 47\n\
                  [h264 @ 000001d4] Invalid NAL unit size (-56 > 271).\n\
                  [null @ 000001d4] Application provided invalid, non monotonically increasing dts to muxer in stream 0: 48 >= 48";

    let v = validate::classify(stderr);
    assert!(!v.ok, "a real decoder complaint was drowned out by noise");
    assert_eq!(v.problems.len(), 1);
    assert_eq!(v.ignored.len(), 2);
}

#[test]
fn similar_wording_from_a_decoder_is_not_excused() {
    // Only the null muxer's timestamp complaint is excused. The same words from a
    // decoder mean something is actually wrong with the data, and matching on the
    // wording alone would wave it through.
    let v = validate::classify("[h264 @ 000001d4] non monotonically increasing dts");
    assert!(!v.ok, "a decoder complaint was excused as muxer noise");
}

#[test]
fn unknown_complaints_are_treated_as_problems() {
    // Anything not recognised as harmless counts against the file. The opposite
    // default would mean every message FFmpeg invents in a future release is
    // silently forgiven.
    let v = validate::classify("Something nobody has seen before happened");
    assert!(!v.ok, "an unrecognised complaint was forgiven");
}

#[test]
fn blank_lines_are_not_complaints() {
    let v = validate::classify("\n   \n\n");
    assert!(v.ok);
    assert!(v.problems.is_empty());
}

// ---------- against real files ----------

/// Decode a real file, and a deliberately damaged one.
///
/// The classifier above decides on text; this proves the text it decides on is
/// the text FFmpeg actually produces. Needs the bundled FFmpeg — without it the
/// check says so out loud rather than quietly passing.
#[tokio::test]
async fn a_real_file_passes_and_a_damaged_one_does_not() {
    use vrcast_studio_lib::media::ffmpeg;

    let Ok(ff) = ffmpeg::locate("ffmpeg") else {
        eprintln!(
            "SKIPPED: no bundled FFmpeg. Run `npm run ffmpeg` for this check to check anything."
        );
        return;
    };

    let dir =
        std::env::temp_dir().join(format!("vrcast-validate-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).expect("could not make a working directory");
    let good = dir.join("good.mp4");

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
            "3",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-ac",
            "2",
        ])
        .arg(&good)
        .output()
        .expect("could not run the bundled FFmpeg");
    assert!(made.status.success(), "could not prepare a clip");

    let clean = validate::validate(&good)
        .await
        .expect("validation did not run");
    assert!(
        clean.ok,
        "a freshly encoded file was rejected: {:?}",
        clean.problems
    );

    // Damage the middle of the stream, leaving the container intact — exactly how
    // a half-written file from an orphaned encoder looks: it opens, reports the
    // right duration, and falls apart inside.
    let damaged = dir.join("damaged.mp4");
    let mut bytes = std::fs::read(&good).expect("could not read the clip");
    let from = bytes.len() / 3;
    let to = (from + bytes.len() / 4).min(bytes.len());
    for b in &mut bytes[from..to] {
        *b = 0x5A;
    }
    std::fs::write(&damaged, &bytes).expect("could not write the damaged clip");

    let broken = validate::validate(&damaged)
        .await
        .expect("validation did not run");
    assert!(
        !broken.ok,
        "a damaged file passed validation — it would have been offered for upload"
    );
    assert!(
        !broken.problems.is_empty(),
        "the file was rejected without saying why"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
