//! T194, T195 — what a rung needs before it can be cut, and the keyframe rule.

use vrcast_studio_lib::domain::convert_plan::VideoAction;
use vrcast_studio_lib::domain::ladder::{Quality, Rung};
use vrcast_studio_lib::domain::ladder_build::{
    file_name, keyframes_line_up, shared_gop, stranded, sub_name, work_for,
};
use vrcast_studio_lib::domain::source::{AudioTrack, SourceFile};
use vrcast_studio_lib::domain::wording::DetailCode;
use vrcast_studio_lib::media::keyframes::from_times;

fn source(width: u32, height: u32, fps: u32, bitrate_bps: u64, codec: &str) -> SourceFile {
    SourceFile {
        path: String::from("F:/films/film.mp4"),
        size_bytes: 4_000_000_000,
        duration_s: 3600.0,
        width,
        height,
        fps,
        bitrate_bps,
        peak_bps: None,
        video_codec: codec.to_owned(),
        pix_fmt: String::from("yuv420p"),
        color_transfer: None,
        audio_tracks: vec![AudioTrack {
            index: 0,
            codec: String::from("aac"),
            profile: Some(String::from("LC")),
            channels: 2,
            bitrate_bps: Some(192_000),
            language: None,
            title: None,
            is_default: true,
        }],
    }
}

fn rung(index: usize, bitrate_bps: u64, height: u32) -> Rung {
    Rung {
        index,
        bitrate_bps,
        maxrate_bps: bitrate_bps * 11 / 10,
        bufsize_bps: bitrate_bps * 11 / 10,
        width: height * 16 / 9,
        height,
        level: String::from("5.1"),
        reasons: Vec::new(),
        quality: Quality::MeasuredHere { vmaf_x100: 9500 },
    }
}

// ---------- names ----------

#[test]
fn a_variant_is_named_by_its_whole_megabits_as_everything_here_always_has_been() {
    // A person looking at `v22` beside `film_22.mp4` knows they belong together without
    // being told, and every file this project has ever made is named this way.
    assert_eq!(sub_name(&rung(0, 22_000_000, 2160)), "v22");
    assert_eq!(file_name("film", &rung(0, 22_000_000, 2160)), "film_22.mp4");
    // Never a zero: a rung under a megabit cannot exist, but if one did it would collide
    // with every other such rung in the same directory.
    assert_eq!(sub_name(&rung(0, 400_000, 480)), "v1");
}

// ---------- the keyframe rule ----------

#[test]
fn keyframes_line_up_when_their_spacing_divides_a_segment() {
    // Not "the same number as ours": a source with a keyframe every second and segments of
    // four seconds line up perfectly well, and demanding equality would re-encode it for
    // nothing.
    assert!(keyframes_line_up(1.0, 24, 4));
    assert!(keyframes_line_up(2.0, 24, 4));
    assert!(keyframes_line_up(4.0, 24, 4));
    // 23.976 frames a second gives a keyframe every 1.001 s. That is not one second and is
    // nevertheless exactly right: the variants encoded from this source get the same.
    assert!(keyframes_line_up(1.001, 24, 4));
    assert!(keyframes_line_up(2.002, 24, 4));
}

#[test]
fn keyframes_that_do_not_divide_a_segment_do_not_line_up() {
    // Five seconds against four: the boundaries agree once every twenty seconds and
    // disagree the rest of the time. A viewer changing quality then waits for the next
    // point at which the two meet, and what they see is a stall.
    assert!(!keyframes_line_up(5.0, 24, 4));
    assert!(!keyframes_line_up(3.0, 24, 4));
    assert!(!keyframes_line_up(10.0, 24, 4));

    // **The one that looks fine and is not.** Twenty-three frames between keyframes is
    // 0.958 s — within a frame of a second, and a whole second out after twenty-four
    // intervals. Counting in seconds with a frame of slack would have let this through,
    // and the fault would have shown up as an occasional stutter nobody could reproduce.
    assert!(!keyframes_line_up(23.0 / 24.0, 24, 4));

    // Nonsense is not "probably fine".
    assert!(!keyframes_line_up(0.0, 24, 4));
    assert!(!keyframes_line_up(-1.0, 24, 4));
    assert!(!keyframes_line_up(f64::NAN, 24, 4));
    assert!(!keyframes_line_up(1.0, 0, 4));
}

#[test]
fn the_spacing_is_the_middle_gap_rather_than_the_average() {
    // A film regularly has an extra keyframe at a hard cut. One such quarter-second gap
    // drags an average down far enough to make a stream look finer-grained than it is —
    // and that is the direction that ends in a copy which should never have been allowed.
    let with_a_cut = "0.0\n2.0\n4.0\n4.25\n6.0\n8.0\n10.0\n";
    let spacing = from_times(with_a_cut).expect("nothing was worked out");
    assert!(
        (spacing - 2.0).abs() < 0.01,
        "the odd gap at the cut moved the answer to {spacing}"
    );

    // One keyframe says nothing about spacing, and nothing is the honest answer.
    assert_eq!(from_times("0.0\n"), None);
    assert_eq!(from_times(""), None);
    assert_eq!(from_times("not a number\n"), None);
}

// ---------- what each rung needs ----------

#[test]
fn every_variant_gets_the_same_keyframe_spacing_and_it_follows_the_frame_rate() {
    // A constant would be wrong twice over: 48 means "once a second" on 48-frame material
    // and "once every two" on 24-frame, and two rungs given different numbers stop agreeing
    // about where a segment may begin.
    assert_eq!(shared_gop(&source(3840, 2160, 24, 60_000_000, "h264")), 24);
    assert_eq!(shared_gop(&source(3840, 2160, 48, 60_000_000, "h264")), 48);

    let src = source(3840, 2160, 24, 60_000_000, "h264");
    let work = work_for(
        "film",
        &[
            rung(0, 22_000_000, 2160),
            rung(1, 12_000_000, 1440),
            rung(2, 6_000_000, 1080),
        ],
        &src,
        0,
        Some(1.0),
        4,
    );
    assert_eq!(work.len(), 3);
    let spacings: Vec<u32> = work.iter().map(|w| w.plan.gop).collect();
    assert_eq!(
        spacings,
        vec![24, 24, 24],
        "the variants were given different keyframe spacings and their segments will not meet"
    );
}

#[test]
fn a_rung_that_needs_no_change_of_quality_is_carried_across_untouched() {
    // FR-045. Minutes instead of hours, and no loss at all.
    let src = source(3840, 2160, 24, 22_000_000, "h264");
    let work = work_for("film", &[rung(0, 22_000_000, 2160)], &src, 0, Some(1.0), 4);
    assert_eq!(work[0].plan.video, VideoAction::Copy);
    assert!(work[0].lossless);
    assert!(work[0].notices.is_empty());
}

#[test]
fn a_copy_is_taken_away_when_the_keyframes_would_not_line_up_and_the_person_is_told() {
    // The one place a copy is refused for a reason that has nothing to do with quality.
    // "This rung will take hours after all" is not something to discover from a progress
    // bar.
    let src = source(3840, 2160, 24, 22_000_000, "h264");
    let work = work_for("film", &[rung(0, 22_000_000, 2160)], &src, 0, Some(5.0), 4);

    assert_ne!(
        work[0].plan.video,
        VideoAction::Copy,
        "a stream whose keyframes sit in the wrong places was carried across anyway"
    );
    assert!(!work[0].lossless);
    assert_eq!(
        work[0].notices.iter().map(|n| n.key).collect::<Vec<_>>(),
        vec![DetailCode::NoticeReencodedForKeyframes],
        "the rung is being re-encoded and nothing says why"
    );
}

#[test]
fn not_knowing_where_the_keyframes_are_is_not_permission_to_copy() {
    // Guessing that they line up is the one guess in a ladder that a viewer pays for.
    let src = source(3840, 2160, 24, 22_000_000, "h264");
    let work = work_for("film", &[rung(0, 22_000_000, 2160)], &src, 0, None, 4);
    assert_ne!(work[0].plan.video, VideoAction::Copy);
}

#[test]
fn a_lower_rung_is_re_encoded_because_its_quality_really_does_change() {
    // Nothing to do with keyframes: half the height and a quarter of the bitrate cannot be
    // carried across whatever the keyframes do.
    let src = source(3840, 2160, 24, 22_000_000, "h264");
    let work = work_for("film", &[rung(0, 6_000_000, 1080)], &src, 0, Some(1.0), 4);
    assert_ne!(work[0].plan.video, VideoAction::Copy);
    assert!(
        work[0].notices.is_empty(),
        "a rung re-encoded for its quality was blamed on the keyframes"
    );
}

// ---------- a rung that stops being served (T467, T469) ----------
//
// From a real loss on the production server, 2026-08-29: a set was rebuilt without one of its
// rungs and that quality vanished for viewers. The shell script loses it by deleting the
// directory; this application loses it by no longer mentioning it in the master, which is
// quieter and therefore worse — the file and the segments are still there, taking up the
// disk, serving nobody, and nothing says so.

fn on_server(names: &[(&str, bool)]) -> Vec<(String, bool)> {
    names.iter().map(|(n, d)| ((*n).to_owned(), *d)).collect()
}

#[test]
fn a_variant_left_out_of_the_rebuild_is_named() {
    // Exactly what happened: 7/4/2/1 on the server, 7/2/1 rebuilt over it.
    let src = source(3840, 2160, 24, 60_000_000, "h264");
    let wanted = work_for(
        "film",
        &[
            rung(0, 7_000_000, 2160),
            rung(1, 2_000_000, 1080),
            rung(2, 1_000_000, 720),
        ],
        &src,
        0,
        None,
        4,
    );
    let there = on_server(&[
        ("v7", true),
        ("v4", true),
        ("v2", true),
        ("v1", true),
        ("master.m3u8", false),
    ]);
    assert_eq!(
        stranded(&there, &wanted),
        vec![String::from("v4")],
        "the rung that stops being served was not named"
    );
}

#[test]
fn a_rebuild_of_the_same_set_strands_nothing() {
    // The negative control, and it earns its place: "name everything on the server" would
    // pass the test above and turn every ordinary rebuild into a complaint about itself.
    let src = source(3840, 2160, 24, 60_000_000, "h264");
    let wanted = work_for(
        "film",
        &[rung(0, 7_000_000, 2160), rung(1, 2_000_000, 1080)],
        &src,
        0,
        None,
        4,
    );
    let there = on_server(&[("v7", true), ("v2", true), ("master.m3u8", false)]);
    assert!(stranded(&there, &wanted).is_empty());
}

#[test]
fn the_prepared_file_beside_a_variant_is_not_a_variant() {
    // `film_4.mp4` is what a variant is made *from*; it is never served, and naming it would
    // report a loss on every build that ever ran.
    let src = source(3840, 2160, 24, 60_000_000, "h264");
    let wanted = work_for("film", &[rung(0, 7_000_000, 2160)], &src, 0, None, 4);
    let there = on_server(&[("v7", true), ("film_4.mp4", false), ("film_7.mp4", false)]);
    assert!(stranded(&there, &wanted).is_empty());
}

#[test]
fn somebody_elses_directory_is_not_ours_to_complain_about() {
    // A show's directory may hold things this application did not put there. Only `v` and
    // digits is a variant of ours.
    let src = source(3840, 2160, 24, 60_000_000, "h264");
    let wanted = work_for("film", &[rung(0, 7_000_000, 2160)], &src, 0, None, 4);
    let there = on_server(&[
        ("v7", true),
        ("subtitles", true),
        ("v", true),
        ("vold", true),
    ]);
    assert!(stranded(&there, &wanted).is_empty());
}

#[test]
fn several_stranded_variants_are_all_named_and_in_order() {
    // A person who dropped three rungs needs all three, and in an order they can read.
    let src = source(3840, 2160, 24, 60_000_000, "h264");
    let wanted = work_for("film", &[rung(0, 7_000_000, 2160)], &src, 0, None, 4);
    let there = on_server(&[("v7", true), ("v4", true), ("v2", true), ("v1", true)]);
    assert_eq!(
        stranded(&there, &wanted),
        vec![String::from("v1"), String::from("v2"), String::from("v4")]
    );
}

// ---------- T660: a variant goes out in blocks, not whole ----------

mod streaming {
    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::{Context, Poll};
    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
    use vrcast_studio_lib::store::db::Db;
    use vrcast_studio_lib::tasks::engine::TaskContext;
    use vrcast_studio_lib::tasks::ladder_build::{stream_blocks, StreamError, SEND_BLOCK};

    /// A file of `left` bytes that exists nowhere: it is produced as it is read, so the test
    /// itself never holds it — which is what lets it be larger than any block by a lot.
    struct Endless {
        left: u64,
    }

    impl AsyncRead for Endless {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            let n = (buf.remaining() as u64).min(self.left) as usize;
            buf.put_slice(&vec![0x5A; n]);
            self.left -= n as u64;
            Poll::Ready(Ok(()))
        }
    }

    /// A server that keeps nothing: it counts, and remembers the largest single write it was
    /// handed — the most the sender ever held at once.
    #[derive(Default)]
    struct Counting {
        total: u64,
        largest: usize,
        writes: usize,
        /// Cancel the task after this many writes, to land a cancel mid-way.
        cancel_after: Option<(usize, TaskContext)>,
    }

    impl AsyncWrite for Counting {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            self.total += buf.len() as u64;
            self.largest = self.largest.max(buf.len());
            self.writes += 1;
            if let Some((after, ctx)) = &self.cancel_after {
                // Raised through the token, which is what the engine raises too.
                if self.writes >= *after {
                    ctx.cancel_token().cancel();
                }
            }
            Poll::Ready(Ok(buf.len()))
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    fn ctx() -> TaskContext {
        TaskContext::detached(Arc::new(Db::open_in_memory().expect("no database")))
    }

    #[tokio::test]
    async fn memory_does_not_grow_with_the_size_of_the_variant() {
        // QA-24B-01: the whole variant was read into memory before a byte was sent.
        let size: u64 = 64 * 1024 * 1024 + 12_345;
        let mut from = Endless { left: size };
        let mut into = Counting::default();

        let sent = stream_blocks(&mut from, &mut into, size, &ctx())
            .await
            .expect("the sending failed");

        assert_eq!(sent, size);
        assert_eq!(into.total, size, "not every byte arrived");
        assert!(
            into.largest <= SEND_BLOCK,
            "a write of {} bytes was handed over at once — more than one block",
            into.largest
        );
        assert!(
            into.writes as u64 >= size / SEND_BLOCK as u64,
            "it went across in {} writes, which is not block by block",
            into.writes
        );
    }

    #[tokio::test]
    async fn a_cancel_mid_way_stops_between_blocks() {
        let size: u64 = 32 * SEND_BLOCK as u64;
        let task = ctx();
        let mut from = Endless { left: size };
        let mut into = Counting {
            cancel_after: Some((3, task.clone())),
            ..Default::default()
        };

        let outcome = stream_blocks(&mut from, &mut into, size, &task).await;

        assert!(
            matches!(outcome, Err(StreamError::Cancelled)),
            "a cancel mid-way did not stop the sending: {outcome:?}"
        );
        assert!(
            into.total < size,
            "the whole variant was sent after the cancel"
        );
        assert!(
            into.total <= 4 * SEND_BLOCK as u64,
            "the cancel was not looked at between blocks: {} bytes went after it",
            into.total
        );
    }

    #[tokio::test]
    async fn an_already_cancelled_task_sends_nothing() {
        let task = ctx();
        task.cancel_token().cancel();
        let mut from = Endless { left: 10 * 1024 };
        let mut into = Counting::default();
        let outcome = stream_blocks(&mut from, &mut into, 10 * 1024, &task).await;
        assert!(matches!(outcome, Err(StreamError::Cancelled)));
        assert_eq!(into.total, 0);
    }
}

// ---------- T670(4): a long pause lets go of what it holds ----------

mod holding {
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::mpsc;
    use vrcast_studio_lib::store::db::Db;
    use vrcast_studio_lib::tasks::engine::{TaskContext, TaskEngine};
    use vrcast_studio_lib::tasks::ladder_build::{stream_blocks_holding, StreamError, SEND_BLOCK};
    use vrcast_studio_lib::tasks::state::TaskKind;

    /// Run `body` inside a real task, so that pause and resume are the engine's own.
    async fn in_task<F, Fut>(body: F) -> (TaskEngine, String, mpsc::UnboundedReceiver<String>)
    where
        F: FnOnce(TaskContext, mpsc::UnboundedSender<String>) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let engine = TaskEngine::new(Arc::new(Db::open_in_memory().unwrap()));
        let (said, heard) = mpsc::unbounded_channel();
        let id = engine
            .submit(TaskKind::BuildLadder, None, move |ctx| async move {
                body(ctx, said).await;
                Ok(())
            })
            .await
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while engine.get(&id).unwrap().unwrap().state
            != vrcast_studio_lib::tasks::state::TaskState::Running
        {
            assert!(std::time::Instant::now() < deadline, "the task never ran");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        (engine, id, heard)
    }

    #[tokio::test]
    async fn a_pause_longer_than_the_hold_gives_up_with_how_far_it_got() {
        let (engine, id, mut heard) = in_task(|ctx, said| async move {
            // Paused before the copy reaches its first block — what the person does mid-send.
            while !ctx.is_paused() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            let size = 8 * SEND_BLOCK as u64;
            let mut from = &vec![7u8; size as usize][..];
            let mut into = Vec::new();
            let already = 3 * SEND_BLOCK as u64;
            let outcome = stream_blocks_holding(
                &mut from,
                &mut into,
                already,
                size,
                &ctx,
                Some(Duration::from_millis(200)),
            )
            .await;
            let _ = said.send(format!("{outcome:?}|{}", into.len()));
        })
        .await;
        engine.pause(&id).unwrap();

        let answer = tokio::time::timeout(Duration::from_secs(10), heard.recv())
            .await
            .expect("a long pause held on for ever")
            .unwrap();
        assert_eq!(
            answer,
            format!(
                "{:?}|0",
                Err::<u64, StreamError>(StreamError::PausedTooLong {
                    sent: 3 * SEND_BLOCK as u64
                })
            ),
            "it did not say where it stopped, counting from the start of the file"
        );
        engine.cancel(&id).unwrap();
    }

    #[tokio::test]
    async fn a_pause_let_go_of_within_the_hold_carries_on_in_the_same_copy() {
        let (engine, id, mut heard) = in_task(|ctx, said| async move {
            while !ctx.is_paused() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            let _ = said.send(String::from("paused"));
            let size = 4 * SEND_BLOCK as u64;
            let mut from = &vec![7u8; size as usize][..];
            let mut into = Vec::new();
            let outcome = stream_blocks_holding(
                &mut from,
                &mut into,
                0,
                size,
                &ctx,
                Some(Duration::from_secs(30)),
            )
            .await;
            let _ = said.send(format!("{outcome:?}|{}", into.len()));
        })
        .await;
        engine.pause(&id).unwrap();
        assert_eq!(heard.recv().await.unwrap(), "paused");
        tokio::time::sleep(Duration::from_millis(100)).await;
        engine.resume(&id).unwrap();

        let size = 4 * SEND_BLOCK;
        let answer = tokio::time::timeout(Duration::from_secs(10), heard.recv())
            .await
            .expect("the copy did not carry on")
            .unwrap();
        assert_eq!(answer, format!("Ok({size})|{size}"));
    }

    #[tokio::test]
    async fn without_a_hold_a_pause_is_waited_out_as_before() {
        // `stream_blocks` (T660) keeps its old behaviour: no limit, so a pause longer than any
        // hold still ends in the same copy rather than in `PausedTooLong`.
        let (engine, id, mut heard) = in_task(|ctx, said| async move {
            while !ctx.is_paused() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            let _ = said.send(String::from("paused"));
            let size = 2 * SEND_BLOCK as u64;
            let mut from = &vec![7u8; size as usize][..];
            let mut into = Vec::new();
            let outcome = vrcast_studio_lib::tasks::ladder_build::stream_blocks(
                &mut from, &mut into, size, &ctx,
            )
            .await;
            let _ = said.send(format!("{outcome:?}"));
        })
        .await;
        engine.pause(&id).unwrap();
        assert_eq!(heard.recv().await.unwrap(), "paused");
        tokio::time::sleep(Duration::from_millis(400)).await;
        engine.resume(&id).unwrap();
        let answer = tokio::time::timeout(Duration::from_secs(10), heard.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(answer, format!("Ok({})", 2 * SEND_BLOCK));
    }
}

// ---------- a rung named around a medium's own file (T677) ----------

fn two_rung_work() -> Vec<vrcast_studio_lib::domain::ladder_build::VariantWork> {
    let src = source(1920, 1080, 24, 30_000_000, "h264");
    work_for(
        "film",
        &[rung(0, 9_000_000, 1080), rung(1, 4_000_000, 720)],
        &src,
        0,
        Some(1.0),
        4,
    )
}

#[test]
fn a_rung_whose_name_a_medium_claims_takes_the_next_free_one_and_the_file_is_not_touched() {
    use vrcast_studio_lib::domain::ladder_build::choose_files;
    let work = two_rung_work();
    // The medium's own single file is `film_9.mp4`: the 9 Mbit/s rung becomes `film_9v.mp4`,
    // the other keeps its first name.
    assert_eq!(
        choose_files("film", &work, &["film_9.mp4"], &[]),
        vec!["film_9v.mp4", "film_4.mp4"]
    );
    // That one claimed too: `v2`, and so on. A claimed name is never given out.
    assert_eq!(
        choose_files(
            "film",
            &work,
            &["film_9.mp4", "film_9v.mp4", "film_9v2.mp4"],
            &[]
        ),
        vec!["film_9v3.mp4", "film_4.mp4"]
    );
    // Nothing claimed: every rung its first name, as always.
    assert_eq!(
        choose_files("film", &work, &[], &[]),
        vec!["film_9.mp4", "film_4.mp4"]
    );
}

#[test]
fn carrying_on_finds_the_rung_under_the_name_it_was_given() {
    use vrcast_studio_lib::domain::ladder_build::{choose_files, parse_prepared, prepared_text};
    let mut work = two_rung_work();
    let first = choose_files("film", &work, &["film_9.mp4"], &[]);
    for (w, f) in work.iter_mut().zip(&first) {
        w.file = f.clone();
    }
    let stored = parse_prepared(&prepared_text(&work));
    assert_eq!(
        stored,
        vec![
            (String::from("v9"), String::from("film_9v.mp4")),
            (String::from("v4"), String::from("film_4.mp4")),
        ]
    );
    // After a restart the medium's file was moved elsewhere meanwhile: the rung is still
    // found under the name it was begun under, not made again under `film_9.mp4`.
    let again = two_rung_work();
    assert_eq!(
        choose_files("film", &again, &[], &stored),
        vec!["film_9v.mp4", "film_4.mp4"]
    );
    // A record that names a file somebody claims now, or a name of another bitrate, is not
    // followed.
    assert_eq!(
        choose_files("film", &again, &["film_9v.mp4"], &stored),
        vec!["film_9.mp4", "film_4.mp4"]
    );
    let wrong = vec![(String::from("v9"), String::from("film_4v.mp4"))];
    assert_eq!(
        choose_files("film", &again, &[], &wrong),
        vec!["film_9.mp4", "film_4.mp4"]
    );
}

#[test]
fn every_name_a_rung_may_take_is_known_as_a_rung_of_its_set_and_nothing_else_is() {
    use vrcast_studio_lib::domain::ladder_build::{file_names, rung_mbit_of};
    let names: Vec<String> = file_names("film", &rung(0, 9_000_000, 1080))
        .take(4)
        .collect();
    assert_eq!(
        names,
        vec!["film_9.mp4", "film_9v.mp4", "film_9v2.mp4", "film_9v3.mp4"]
    );
    for n in &names {
        assert_eq!(rung_mbit_of("film", n), Some(9), "{n}");
    }
    for no in [
        "film_9v1.mp4",
        "film_9v02.mp4",
        "film_9x.mp4",
        "film_v.mp4",
        "film_9vv.mp4",
        "film-2_9v.mp4",
        "film_9v.mkv",
        "film.mp4",
    ] {
        assert_eq!(rung_mbit_of("film", no), None, "{no}");
    }
}

// ---------- a medium's single file of the same length is not a rung (T681, QA-25 №2) ----------

/// Two real films of the same length — a red picture and a blue one — the way QA-25 found it:
/// the blue one lies on the server as a medium's single file `film_4.mp4`, the red one is
/// being built into a set whose 4 Mbit/s rung would have that name.
#[tokio::test]
async fn a_claimed_single_file_of_the_same_length_is_never_taken_for_a_rung() {
    use vrcast_studio_lib::domain::ladder_build::choose_files;
    use vrcast_studio_lib::media::ffmpeg;
    let Ok(ff) = ffmpeg::locate("ffmpeg") else {
        eprintln!("no bundled FFmpeg: skipped");
        return;
    };
    let dir = std::env::temp_dir().join(format!("vrcast-t681-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, color) in [("source.mp4", "red"), ("film_4.mp4", "blue")] {
        let out = std::process::Command::new(&ff)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
            ])
            .arg(format!("color=c={color}:s=1280x720:r=24:d=2"))
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=2"])
            .args([
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-c:a",
                "aac",
                "-shortest",
            ])
            .arg(dir.join(name))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let probe = |n: &str| {
        let p = dir.join(n).to_string_lossy().into_owned();
        async move {
            vrcast_studio_lib::commands::api::source_probe(&p)
                .await
                .unwrap()
        }
    };
    let source = probe("source.mp4").await;
    let other = probe("film_4.mp4").await;
    // What `variant_already_there` asked, and all it asked: the same length.
    assert!(other.duration_s > 0.0 && (other.duration_s - source.duration_s).abs() < 1.0);

    let mut r = rung(0, 4_000_000, 720);
    r.width = 1280;
    let work = work_for("film", &[r], &source, 0, Some(1.0), 4);
    // The medium's file is claimed: the rung takes the next free name, whatever length the
    // file is — one length is not a film.
    let names = choose_files("film", &work, &["film_4.mp4"], &[]);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(names, vec!["film_4v.mp4"]);
}

/// The set's own record of what it made (T681): the only thing that lets a rung file be
/// taken as done — this source, this height, the bitrate within a tenth, this sound track.
#[test]
fn a_rung_is_done_only_when_the_sets_record_says_it_made_it_from_this_source() {
    use vrcast_studio_lib::domain::ladder_build::{
        made_here, parse_made, parse_prepared, parse_rung_facts, prepared_text_with, MadeRung,
    };
    let src = source(1920, 1080, 24, 30_000_000, "h264");
    let work = two_rung_work();
    let made = vec![MadeRung::of(&work[0], &src, 0)];
    let text = prepared_text_with(&work, &made);
    // The names read as before; the line of what was made is not a name.
    assert_eq!(
        parse_prepared(&text),
        vec![
            (String::from("v9"), String::from("film_9.mp4")),
            (String::from("v4"), String::from("film_4.mp4")),
        ]
    );
    let back = parse_made(&text);
    assert_eq!(back, made);
    assert!(made_here(&back, &work[0], &src, 0));
    // The other rung was not made.
    assert!(!made_here(&back, &work[1], &src, 0));
    // Another sound track, another film (size or length), another height: not this rung.
    assert!(!made_here(&back, &work[0], &src, 1));
    let mut other = src.clone();
    other.size_bytes += 1;
    assert!(!made_here(&back, &work[0], &other, 0));
    let mut longer = src.clone();
    longer.duration_s += 2.0;
    assert!(!made_here(&back, &work[0], &longer, 0));
    let mut lower = work[0].clone();
    lower.rung.height = 720;
    assert!(!made_here(&back, &lower, &src, 0));
    // Under another name: not this file.
    let mut moved = work[0].clone();
    moved.file = String::from("film_9v.mp4");
    assert!(!made_here(&back, &moved, &src, 0));
    // Without a record — a set built before T681, or a file nobody made here — nothing is.
    assert!(!made_here(&[], &work[0], &src, 0));
    // A bitrate within a tenth is the same rung.
    let mut near = work[0].clone();
    near.rung.bitrate_bps = 9_500_000;
    assert!(made_here(&back, &near, &src, 0));
    // What ffprobe says of a file on the server.
    assert_eq!(
        parse_rung_facts("height=1080\nduration=3600.040000\n"),
        (Some(3600.04), Some(1080))
    );
    assert_eq!(parse_rung_facts(""), (None, None));
}

// ---------- a stage's own progress, not the whole build's (T689, QA-25 №10) ----------

#[test]
fn the_cutting_and_the_check_begin_at_their_own_beginning() {
    use vrcast_studio_lib::domain::ladder_build::share_of;
    // The cutting goes rung by rung: four rungs, none cut yet, is nought — not 4/5 of the set.
    assert_eq!(share_of(0, 4), 0.0);
    assert_eq!(share_of(1, 4), 0.25);
    assert_eq!(share_of(4, 4), 1.0);
    assert_eq!(share_of(5, 4), 1.0);
    assert_eq!(share_of(0, 0), 1.0);

    // And the build says so: no stage of the cutting or the check is reported from the share
    // of the whole set, nor from a fixed 0.99.
    let text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tasks/ladder_build.rs"),
    )
    .unwrap();
    let code: String = text
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !code.contains("0.99, DetailCode::StageVerifyingLadder"),
        "the check still begins at 99%"
    );
    assert!(
        code.contains("report_important(0.0, DetailCode::StageVerifyingLadder)"),
        "the check does not begin at nought"
    );
    assert!(
        code.contains("report_important(0.0, DetailCode::StageCuttingSegments)")
            && code.contains("share_of(p.cut.len(), work.len())"),
        "the cutting is not reported by the rungs it has cut"
    );
}

// ---------- T693: a checked set keeps only its segments ----------

const FINISHED: &str = "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4.000,\nseg_00000.ts\n\
    #EXTINF:4.000,\nseg_00001.ts\n#EXT-X-ENDLIST\n";

fn facts(height: u32, segs: &[f64]) -> String {
    let mut out = format!(
        "sub=v9\nwidth={}\nheight={height}\nfps=24.000\nlevel=40\ncodec=h264\n",
        height * 16 / 9
    );
    for s in segs {
        out.push_str(&format!("seg {s} 1000\n"));
    }
    out
}

fn read_both(playlist: &str, facts: &str) -> String {
    format!(
        "{playlist}\n{}\n{facts}",
        vrcast_studio_lib::domain::ladder_build::CUT_FACTS_MARK
    )
}

#[test]
fn a_rung_cut_whole_is_known_by_its_segments_once_its_prepared_file_is_gone() {
    use vrcast_studio_lib::domain::ladder_build::cut_is_whole;
    // Two segments of four seconds, a film of eight: cut whole.
    assert!(cut_is_whole(
        &read_both(FINISHED, &facts(1080, &[4.0, 4.0])),
        8.0,
        1080
    ));
    // A playlist that is not finished — a cutting stopped halfway — is not.
    assert!(!cut_is_whole(
        &read_both(
            &FINISHED.replace("#EXT-X-ENDLIST\n", ""),
            &facts(1080, &[4.0, 4.0])
        ),
        8.0,
        1080
    ));
    // Another height: a rung cut from something else.
    assert!(!cut_is_whole(
        &read_both(FINISHED, &facts(720, &[4.0, 4.0])),
        8.0,
        1080
    ));
    // Shorter than the film by more than a second and a half: cut short.
    assert!(!cut_is_whole(
        &read_both(FINISHED, &facts(1080, &[4.0])),
        8.0,
        1080
    ));
    // No facts at all, nothing on the server, a film of unknown length.
    assert!(!cut_is_whole(&read_both(FINISHED, ""), 8.0, 1080));
    assert!(!cut_is_whole("", 8.0, 1080));
    assert!(!cut_is_whole(
        &read_both(FINISHED, &facts(1080, &[4.0, 4.0])),
        0.0,
        1080
    ));
}

#[test]
fn only_prepared_files_the_set_made_and_nobody_claims_are_removed() {
    use vrcast_studio_lib::domain::ladder_build::{removable_files, MadeRung};
    let src = source(1920, 1080, 24, 30_000_000, "h264");
    let work = two_rung_work();
    let made: Vec<MadeRung> = work.iter().map(|w| MadeRung::of(w, &src, 0)).collect();
    assert_eq!(
        removable_files(&work, &made, &[]),
        vec!["film_9.mp4", "film_4.mp4"]
    );
    // A rung the record does not say it made stays: not certainly ours.
    assert_eq!(removable_files(&work, &made[..1], &[]), vec!["film_9.mp4"]);
    // A name a medium claims meanwhile stays (T577, part b).
    assert_eq!(
        removable_files(&work, &made, &["film_9.mp4"]),
        vec!["film_4.mp4"]
    );
    // A record of another name under the same rung is not this file.
    let mut other = made.clone();
    other[1].file = String::from("film_4v.mp4");
    assert_eq!(removable_files(&work, &other, &[]), vec!["film_9.mp4"]);
}

/// The build asks a rung's segments only after the record (T693) and removes the prepared
/// files only after the check — never before: before it they are what carrying on cuts from.
#[test]
fn the_prepared_files_go_only_after_the_set_is_checked() {
    let code = include_str!("../../src/tasks/ladder_build.rs");
    let check = code
        .find("hls_verify::verify(job.master_url")
        .expect("the build no longer checks the set");
    let removal = code
        .find("remove_prepared(job, &work, &made)")
        .expect("the build no longer removes its prepared files");
    let refusal = code
        .find("return Err(BuildError::Incomplete(verdict.broken()));")
        .expect("the build no longer refuses an incomplete set");
    assert!(check < refusal && refusal < removal);
    // And the cutting does not stop on a missing prepared file of a rung already cut whole.
    let script = vrcast_studio_lib::domain::hls_package::script_text();
    let cut_whole = script.find("grep -q ENDLIST").unwrap();
    let no_such = script.find("no such file").unwrap();
    assert!(cut_whole < no_such, "{script}");
}

// ---------- T698: the top rung of an HEVC or HDR source ----------

/// QA-26 no. 8 — the top rung of an HEVC source has the source's numbers and is re-encoded
/// all the same: to the rung's own bitrate, under its ceiling — not at a pinned quality that
/// lands wherever it lands.
#[test]
fn a_top_rung_that_cannot_be_carried_across_is_held_to_its_own_bitrate() {
    for (codec, pix_fmt, transfer) in [
        ("hevc", "yuv420p10le", None),
        ("h264", "yuv420p10le", None),
        ("h264", "yuv420p", Some("smpte2084")),
    ] {
        let mut src = source(3840, 2160, 24, 22_000_000, codec);
        src.pix_fmt = pix_fmt.to_owned();
        src.color_transfer = transfer.map(str::to_owned);
        let top = rung(0, 22_000_000, 2160);
        let work = work_for("film", std::slice::from_ref(&top), &src, 0, Some(1.0), 4);
        match &work[0].plan.video {
            VideoAction::ReencodeCapped {
                target_kbps,
                maxrate_kbps,
                bufsize_kbps,
                ..
            } => {
                assert_eq!(*target_kbps, 22_000, "{codec} {pix_fmt}");
                assert_eq!(*maxrate_kbps as u64, top.maxrate_bps / 1000);
                assert_eq!(*bufsize_kbps as u64, top.bufsize_bps / 1000);
            }
            other => panic!("{codec} {pix_fmt} {transfer:?}: {other:?}"),
        }
        assert!(!work[0].lossless);
        // HDR is brought down to the ordinary range on the way.
        assert_eq!(work[0].plan.tonemap, transfer.is_some());
        // And the plan's time counts it.
        assert!(!vrcast_studio_lib::domain::video::is_copy(&top, &src));
    }
    // A plain H.264 source of the same numbers is still carried across.
    let src = source(3840, 2160, 24, 22_000_000, "h264");
    let top = rung(0, 22_000_000, 2160);
    assert!(vrcast_studio_lib::domain::video::is_copy(&top, &src));
    let work = work_for("film", &[top], &src, 0, Some(1.0), 4);
    assert_eq!(work[0].plan.video, VideoAction::Copy);
}

/// What a rung is called a copy for and what the build carries across are one test.
#[test]
fn a_stream_is_copyable_exactly_when_the_plan_would_carry_it_across() {
    use vrcast_studio_lib::domain::convert_plan::{plan, stream_copyable, ConvertRequest};
    for (codec, pix_fmt, transfer) in [
        ("h264", "yuv420p", None),
        ("hevc", "yuv420p", None),
        ("h264", "yuv420p10le", None),
        ("h264", "yuv420p", Some("arib-std-b67")),
        ("av1", "yuv420p", None),
    ] {
        let mut src = source(1920, 1080, 24, 8_000_000, codec);
        src.pix_fmt = pix_fmt.to_owned();
        src.color_transfer = transfer.map(str::to_owned);
        let carried = plan(
            &src,
            &ConvertRequest {
                audio_track: 0,
                target_kbps: None,
                height: None,
            },
        )
        .unwrap()
        .video
            == VideoAction::Copy;
        assert_eq!(
            stream_copyable(&src),
            carried,
            "{codec} {pix_fmt} {transfer:?}"
        );
    }
}
