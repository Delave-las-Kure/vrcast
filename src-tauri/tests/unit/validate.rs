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
fn a_dangling_chapter_reference_does_not_condemn_a_file() {
    // The complaint that cost an owner a two-hour preparation on 2026-08-28. It comes from
    // the container reader about a track reference, says nothing whatever about the picture
    // or the sound, and FFmpeg exits 0 after printing it — the file plays.
    //
    // The cause is fixed where it belongs, in the arguments (`chapters_are_not_carried_over`),
    // so files made from now on will not carry it. This rule is for the ones already made:
    // there are hours of work in them and they are not broken.
    let stderr = "[in#0 @ 0000023717f6fa80] Referenced QT chapter track not found";

    let v = validate::classify(stderr);
    assert!(
        v.ok,
        "a playable file was condemned over a note about a chapter reference: {:?}",
        v.problems
    );
    assert_eq!(
        v.ignored.len(),
        1,
        "the complaint was swallowed instead of being shown as forgiven"
    );
}

#[test]
fn the_same_words_from_a_decoder_are_not_excused() {
    // The forgiveness above is by wording, so it has to be held to the same rule as the
    // muxer one: a decoder saying it would mean something is wrong with the data.
    let v = validate::classify("[h264 @ 000001d4] Referenced QT chapter track not found");
    assert!(
        !v.ok,
        "a decoder complaint was excused as a note about the container"
    );
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

// ---------- T663: the check answers cancel and pause, and a mute decoder is stopped ----------

mod watched {
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use vrcast_studio_lib::commands::error::{AppError, ErrorCode};
    use vrcast_studio_lib::domain::wording::DetailCode;
    use vrcast_studio_lib::media::{ffmpeg, validate};
    use vrcast_studio_lib::store::db::Db;
    use vrcast_studio_lib::tasks::engine::{TaskEngine, TaskEvent};
    use vrcast_studio_lib::tasks::state::{TaskKind, TaskState};

    use crate::proc_check::long_running;

    fn engine() -> TaskEngine {
        TaskEngine::new(Arc::new(Db::open_in_memory().expect("no database")))
    }

    async fn ended(e: &TaskEngine, id: &str, within: Duration) -> Option<TaskState> {
        let deadline = Instant::now() + within;
        while Instant::now() < deadline {
            if let Ok(Some(rec)) = e.get(id) {
                if rec.state.is_final() {
                    return Some(rec.state);
                }
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        None
    }

    /// What `watch_decoder` came back with, as a word — so a task can hand it out.
    type Seen = Arc<Mutex<Option<String>>>;

    fn said(outcome: &validate::Result<validate::Decoded>) -> String {
        match outcome {
            Ok(_) => String::from("finished"),
            Err(validate::ValidateError::Cancelled) => String::from("cancelled"),
            Err(validate::ValidateError::Stalled(_)) => String::from("stalled"),
            Err(e) => format!("other: {e}"),
        }
    }

    #[tokio::test]
    async fn a_mute_decoder_is_stopped_for_saying_nothing() {
        // QA-24B-04: "if FFmpeg hangs, the task never ends". A program that prints no
        // position at all is, to the watch, a decoder that has stopped moving.
        let (prog, args) = long_running();
        let watch = validate::Watch {
            ctx: None,
            duration_s: 0.0,
            stall: Duration::from_millis(1_200),
        };
        let started = Instant::now();
        let outcome = validate::watch_decoder(prog, &args, &watch).await;
        let took = started.elapsed();

        assert_eq!(said(&outcome), "stalled");
        assert!(
            took < Duration::from_secs(10),
            "a mute decoder held the check for {took:?} — the limit on silence did not fire"
        );
    }

    #[test]
    fn the_limit_is_on_silence_and_is_long_by_default() {
        // Not a short limit on the whole decode: a two-hour film takes many minutes to decode,
        // and any total would be wrong for some film.
        assert!(validate::STALL_LIMIT >= Duration::from_secs(60));
        assert_eq!(validate::Watch::unattended().stall, validate::STALL_LIMIT);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelling_during_the_check_ends_the_task() {
        // QA-24B-04: a cancel pressed during the decode waited for the whole film.
        let e = engine();
        let (prog, args) = long_running();
        let seen: Seen = Arc::new(Mutex::new(None));
        let keep = seen.clone();

        let id = e
            .submit(TaskKind::Convert, None, move |ctx| async move {
                let watch = validate::Watch {
                    ctx: Some(&ctx),
                    duration_s: 0.0,
                    // Far longer than the test: only the cancel can end this.
                    stall: Duration::from_secs(600),
                };
                let outcome = validate::watch_decoder(prog, &args, &watch).await;
                *keep.lock().unwrap() = Some(said(&outcome));
                Err(AppError::new(ErrorCode::TaskCancelled))
            })
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(600)).await;
        let asked = Instant::now();
        e.cancel(&id).unwrap();

        let state = ended(&e, &id, Duration::from_secs(10)).await;
        assert_eq!(
            state,
            Some(TaskState::Cancelled),
            "the task did not end after the cancel"
        );
        assert!(asked.elapsed() < Duration::from_secs(5));
        assert_eq!(seen.lock().unwrap().as_deref(), Some("cancelled"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn time_spent_paused_is_not_taken_for_a_hang() {
        // Pause freezes the decoder where it is (the decision, T663); a pause longer than the
        // limit on silence must not come back as "the decoder hung".
        let e = engine();
        let (prog, args) = long_running();
        let seen: Seen = Arc::new(Mutex::new(None));
        let keep = seen.clone();

        let id = e
            .submit(TaskKind::Convert, None, move |ctx| async move {
                let watch = validate::Watch {
                    ctx: Some(&ctx),
                    duration_s: 0.0,
                    stall: Duration::from_millis(1_500),
                };
                let outcome = validate::watch_decoder(prog, &args, &watch).await;
                *keep.lock().unwrap() = Some(said(&outcome));
                Ok(())
            })
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(300)).await;
        e.pause(&id).unwrap();
        // Twice the limit, paused.
        tokio::time::sleep(Duration::from_millis(3_000)).await;
        assert!(
            seen.lock().unwrap().is_none(),
            "the check ended while it was paused: {:?}",
            seen.lock().unwrap()
        );

        // Let go: now the silence counts again, and it ends as stalled rather than never.
        e.resume(&id).unwrap();
        let state = ended(&e, &id, Duration::from_secs(10)).await;
        assert!(state.is_some(), "the check never ended after it was let go");
        assert_eq!(seen.lock().unwrap().as_deref(), Some("stalled"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_real_decode_reports_its_own_progress_and_passes() {
        let Ok(ff) = ffmpeg::locate("ffmpeg") else {
            eprintln!(
                "SKIPPED: no bundled FFmpeg. Run `npm run ffmpeg` for this to check anything."
            );
            return;
        };
        let dir =
            std::env::temp_dir().join(format!("vrcast-t663-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let clip = dir.join("clip.mp4");
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
                "-t",
                "4",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(&clip)
            .output()
            .expect("could not run the bundled FFmpeg");
        assert!(made.status.success());

        let e = engine();
        let mut events = e.subscribe();
        let path = clip.clone();
        let verdict: Arc<Mutex<Option<bool>>> = Arc::new(Mutex::new(None));
        let keep = verdict.clone();
        let id = e
            .submit(TaskKind::Convert, None, move |ctx| async move {
                let v = validate::validate_in_task(&path, &ctx, 4.0)
                    .await
                    .map_err(|e| AppError::new(ErrorCode::Internal).with_cause(e))?;
                *keep.lock().unwrap() = Some(v.ok);
                Ok(())
            })
            .await
            .unwrap();
        assert_eq!(
            ended(&e, &id, Duration::from_secs(60)).await,
            Some(TaskState::Completed)
        );
        assert_eq!(
            *verdict.lock().unwrap(),
            Some(true),
            "a clean clip failed its check"
        );

        let mut shares: Vec<f64> = Vec::new();
        while let Ok(ev) = events.try_recv() {
            if let TaskEvent::Progress {
                id: which,
                stage: Some(DetailCode::StageValidating),
                progress,
                ..
            } = ev
            {
                if which == id {
                    shares.push(progress);
                }
            }
        }
        assert!(
            shares.iter().any(|p| *p > 0.5),
            "the check reported no progress of its own: {shares:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
