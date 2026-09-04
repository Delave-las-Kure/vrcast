//! T365 — scenario 3 of the quickstart, walked (FR-021, FR-024, FR-028, SC-010).
//!
//! ⚠ **Nobody had walked it.** Scenarios 1, 2, 7 and 10 each have a task that walks them;
//! 4, 5, 6 and 8 were walked on the stand. Three had none — and it is the one scenario that
//! needs no server at all, so nothing stood in its way except that it was never written down.
//!
//! What is here and what is elsewhere. Steps 3 and 6 are already held: the drift between
//! sound and picture by `audio_sync`, and killing the application mid-encode by
//! `convert_kill`. Repeating them would say nothing new. This walks the rest — the tracks a
//! person is offered, the track they choose, the video that must not be re-encoded, the
//! playback check, aiming at a bitrate, and a file damaged after the fact.
//!
//! Ignored by default: it needs a real film with several sound tracks and it encodes.
//!
//! ```text
//! VRCAST_PREPARE_SOURCE=F:/films/three-tracks.mkv \
//!   cargo test --features integration --test integration scenario_prepare -- --ignored --nocapture
//! ```

use std::time::Instant;

use vrcast_studio_lib::commands::convert::{api as convert, ConvertStart};

/// A film with more than one sound track, at least one of them in a format the target will
/// not take. Colour bars and a sine wave would answer every question here the same way.
const SOURCE: &str = "VRCAST_PREPARE_SOURCE";

fn source() -> String {
    std::env::var(SOURCE).unwrap_or_else(|_| panic!("{SOURCE} is not set: name the film"))
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::var("VRCAST_PREPARE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    std::fs::create_dir_all(&dir).expect("the working directory would not be made");
    dir.join(name)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "encodes a real film: run by hand"]
async fn scenario_3_a_file_is_prepared_checked_and_offered() {
    let path = source();
    let state = vrcast_studio_lib::commands::AppState::with_db(
        std::sync::Arc::new(vrcast_studio_lib::store::db::Db::open_in_memory().unwrap()),
        std::sync::Arc::new(vrcast_studio_lib::store::secrets::InMemorySecretStore::new()),
    )
    .expect("the application state would not assemble");

    // --- 1. every track, with what a person needs to tell them apart --------------------
    let probed = vrcast_studio_lib::commands::api::source_probe(&path)
        .await
        .expect("the source would not be examined");
    println!("{} sound track(s):", probed.audio_tracks.len());
    for t in &probed.audio_tracks {
        println!(
            "  {} — {} {}ch, {:?} / {:?}",
            t.index, t.codec, t.channels, t.language, t.title
        );
    }
    assert!(
        probed.audio_tracks.len() >= 3,
        "the scenario needs a film with three sound tracks; this one has {}",
        probed.audio_tracks.len()
    );
    // **Naming them is the whole point of the step.** A list of "track 1, track 2, track 3"
    // makes somebody open the file in another program to find out which is which.
    let named = probed
        .audio_tracks
        .iter()
        .filter(|t| t.language.is_some() || t.title.is_some())
        .count();
    assert_eq!(
        named,
        probed.audio_tracks.len(),
        "{} of {} tracks came back with neither a language nor a title",
        probed.audio_tracks.len() - named,
        probed.audio_tracks.len()
    );

    // --- 2. the second track, and no re-encoding of the picture -------------------------
    let out = scratch("prepared.mp4");
    let _ = std::fs::remove_file(&out);
    let began = Instant::now();
    let preview = convert::convert_preview(&ConvertStart {
        path: path.clone(),
        audio_track: 1,
        target_kbps: None,
        height: None,
        out_path: out.to_string_lossy().into_owned(),
        prefer_hardware: true,
    })
    .await
    .expect("the preparation would not be looked at");
    println!("what preparing would involve: {preview:?}");

    run_convert(&state, &path, 1, None, &out).await;
    println!("prepared in {:?}", began.elapsed());

    let made = vrcast_studio_lib::commands::api::source_probe(&out.to_string_lossy())
        .await
        .expect("the result would not be examined");
    // **The picture is carried across, not made again.** Same codec, same frame, same count
    // of frames: anything else means an hour of encoding spent to arrive where we started.
    assert_eq!(
        (made.video_codec.to_lowercase(), made.width, made.height),
        (
            probed.video_codec.to_lowercase(),
            probed.width,
            probed.height
        ),
        "the picture came out different from the one that went in"
    );
    assert_eq!(
        made.audio_tracks.len(),
        1,
        "one track was chosen and {} came out",
        made.audio_tracks.len()
    );
    assert!(
        made.audio_tracks[0].codec.to_lowercase().contains("aac"),
        "the sound was left as {} rather than brought to the target format",
        made.audio_tracks[0].codec
    );

    // --- 4. the check that decides whether it may be offered ----------------------------
    let verdict = convert::convert_validate(&out.to_string_lossy())
        .await
        .expect("the check would not run");
    println!("the check says: {verdict:?}");
    assert!(
        verdict.ok,
        "a file that was just prepared did not pass its own check: {verdict:?}"
    );

    // --- 5. aiming at a bitrate ---------------------------------------------------------
    let capped = scratch("prepared-capped.mp4");
    let _ = std::fs::remove_file(&capped);
    run_convert(&state, &path, 1, Some(2000), &capped).await;
    let peaks = vrcast_studio_lib::commands::diag::api::diag_bitrate(&capped.to_string_lossy())
        .await
        .expect("the result would not be measured");
    let average = peaks.average_bps as f64 / 1e6;
    println!(
        "asked for 2.0 Mbit/s, got {average:.2} on average; the worst ten seconds {:?}",
        peaks.wide
    );
    // A tenth either way: the encoder aims, it does not obey.
    assert!(
        (1.6..=2.6).contains(&average),
        "asked for two megabits and got {average:.2}"
    );

    // --- 7. and a file damaged after it was made is not offered again --------------------
    let damaged = scratch("damaged.mp4");
    std::fs::copy(&out, &damaged).expect("the copy would not be made");
    scribble_over(&damaged);
    let verdict = convert::convert_validate(&damaged.to_string_lossy())
        .await
        .expect("the check would not run on the damaged file");
    println!("the damaged file: {verdict:?}");
    assert!(
        !verdict.ok,
        "a file with its middle overwritten passed the check — and would then be offered for \
         upload as ready: {verdict:?}"
    );
}

/// Prepare the file and wait for it, failing with the reason if it will not.
async fn run_convert(
    state: &vrcast_studio_lib::commands::AppState,
    path: &str,
    track: usize,
    kbps: Option<u32>,
    out: &std::path::Path,
) {
    let request = ConvertStart {
        path: path.to_owned(),
        audio_track: track,
        target_kbps: kbps,
        height: None,
        out_path: out.to_string_lossy().into_owned(),
        prefer_hardware: true,
    };
    let task = match convert::convert_start(state, request).await {
        Ok(id) => id,
        Err(e) => panic!("the preparation would not start: {e:?}"),
    };
    // The task is where the work happens, so waiting on it is waiting on the work.
    use vrcast_studio_lib::tasks::state::TaskState;
    let deadline = Instant::now() + std::time::Duration::from_secs(3600);
    loop {
        let rec =
            vrcast_studio_lib::commands::api::task_get(state, &task).expect("the task vanished");
        match rec.state {
            TaskState::Completed => break,
            TaskState::Failed | TaskState::Cancelled => {
                panic!("the preparation did not finish: {:?}", rec.error)
            }
            _ => {}
        }
        assert!(Instant::now() < deadline, "the preparation never finished");
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    assert!(
        out.exists(),
        "the preparation reported success and left no file at {}",
        out.display()
    );
}

/// Overwrite a stretch in the middle, the way a half-finished copy or a bad disk would.
fn scribble_over(path: &std::path::Path) {
    use std::io::{Seek, SeekFrom, Write};
    let size = std::fs::metadata(path).expect("no such file").len();
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("the file would not open for writing");
    f.seek(SeekFrom::Start(size / 2))
        .expect("could not seek into the file");
    f.write_all(&vec![0u8; 2 * 1024 * 1024])
        .expect("could not write over the middle");
}
