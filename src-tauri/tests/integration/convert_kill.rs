//! T113 — killing the application mid-encode (FR-028, constitution principle III).
//!
//! The rule this proves is not an abstraction. An encoder that outlives the
//! application keeps writing into the output file: the file grows, ends up with
//! a plausible size, and looks finished. Nothing about it says it is garbage —
//! and the next step would happily upload it.
//!
//! The application is killed for real, from outside, with no chance to clean up.
//! Anything gentler would be testing code that does not run when it matters:
//! a crash, a power cut and Task Manager do not send a polite request.

use super::proc_check;
use std::time::{Duration, Instant};

/// How the parent tells the helper what to encode.
mod env {
    pub const SOURCE: &str = "VRCAST_KILL_SOURCE";
    pub const OUT: &str = "VRCAST_KILL_OUT";
}

const HELPER: &str = "convert_kill::the_run_that_gets_killed";

/// The application that will be killed.
///
/// Marked `ignore`: on its own it is not a check but half of one, started by the
/// test below as a separate process. Without the environment it does nothing, in
/// case someone runs all the ignored checks at once.
#[test]
#[ignore = "half of the kill check: started as a separate process"]
fn the_run_that_gets_killed() {
    let (Ok(source), Ok(out)) = (std::env::var(env::SOURCE), std::env::var(env::OUT)) else {
        return;
    };

    let rt = tokio::runtime::Runtime::new().expect("no runtime");
    rt.block_on(async move {
        use std::sync::Arc;
        use vrcast_studio_lib::commands::AppState;
        use vrcast_studio_lib::store::db::Db;
        use vrcast_studio_lib::store::secrets::InMemorySecretStore;

        let state = AppState::with_db(
            Arc::new(Db::open_in_memory().expect("no database")),
            Arc::new(InMemorySecretStore::new()),
        )
        .expect("could not assemble the application state");

        let request = vrcast_studio_lib::commands::convert::ConvertStart {
            path: source,
            audio_track: 0,
            target_kbps: Some(4_000),
            height: Some(360),
            out_path: out,
            // The processor on purpose: hardware encoders are not present on every
            // machine, and this check is about killing, not about speed.
            prefer_hardware: false,
        };

        vrcast_studio_lib::commands::convert::api::convert_start(&state, request)
            .await
            .expect("the conversion did not start");

        // Now just live. The parent watches the output file and kills us.
        loop {
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    });
}

/// Wait until the output file exists and has been written to.
fn wait_for_output(out: &std::path::Path, child: &mut std::process::Child) -> u64 {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if let Ok(meta) = std::fs::metadata(out) {
            if meta.len() > 0 {
                return meta.len();
            }
        }
        if let Ok(Some(status)) = child.try_wait() {
            panic!("the run ended by itself ({status}) before writing anything");
        }
        assert!(
            Instant::now() < deadline,
            "nothing was written to {} within the time allowed",
            out.display()
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
fn killing_the_application_leaves_no_encoder_behind() {
    let ff = match vrcast_studio_lib::media::ffmpeg::locate("ffmpeg") {
        Ok(p) => p,
        Err(_) => {
            eprintln!(
                "SKIPPED: no bundled FFmpeg. Run `npm run ffmpeg` for this check to check anything."
            );
            return;
        }
    };

    let dir = std::env::temp_dir().join(format!("vrcast-kill-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).expect("could not make a working directory");
    let source = dir.join("source.mp4");
    let out = dir.join("ready.mp4");

    // Long enough that the encode is still running when we kill it, and heavy
    // enough that it does not finish in a blink.
    let made = std::process::Command::new(&ff)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=1280x720:rate=30",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            "60",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
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

    let mut child = std::process::Command::new(
        std::env::current_exe().expect("could not find our own program"),
    )
    .args([HELPER, "--exact", "--ignored", "--test-threads=1"])
    .env(env::SOURCE, &source)
    .env(env::OUT, &out)
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .spawn()
    .expect("the run did not start");

    let size_before_kill = wait_for_output(&out, &mut child);
    let pid = child.id();
    let encoders = proc_check::children_of(pid);
    assert!(
        !encoders.is_empty(),
        "the run has no child processes — nothing is encoding, so there is nothing to prove"
    );

    // And here the application dies. No warning, no chance to tidy up.
    child.kill().expect("could not kill the run");
    let _ = child.wait();

    // Give the system a moment to reap what died with it.
    std::thread::sleep(Duration::from_secs(2));

    for pid in &encoders {
        assert!(
            !proc_check::alive(*pid),
            "an encoder ({pid}) outlived the application and is still writing into the file"
        );
    }

    // The size check is the one that matters to a person: an orphaned encoder
    // would keep growing the file, and a file with a plausible size looks finished.
    let after_kill = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
    std::thread::sleep(Duration::from_secs(3));
    let later = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);

    assert_eq!(
        after_kill, later,
        "the output file grew from {after_kill} to {later} after the application was killed — \
         something is still writing to it"
    );
    assert!(
        size_before_kill > 0,
        "nothing had been written before the kill, so the check proved nothing"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Size of the output file, or zero while it does not exist yet.
fn size_of(path: &std::path::Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

/// Wait until the file has grown past `from`, or give up.
///
/// Sleeps the runtime's way, never the thread's. A `#[tokio::test]` runs on a
/// single-threaded runtime: blocking the thread starves the very task being
/// watched, and it sits in the queue for the whole timeout without ever starting.
/// Caught exactly that way — the first version of this check blamed the encoder
/// for never writing when nothing had been allowed to run.
async fn grew_past(path: &std::path::Path, from: u64, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if size_of(path) > from {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

/// Pausing a conversion must actually stop it (FR-083a, debt T067/T070).
///
/// The check is the file, not the state in the database. A pause that only changed
/// a label would look identical from the outside while the encoder carried on
/// burning the machine and the task gave up its place in the queue — which is
/// exactly the defect this closes.
#[tokio::test]
async fn pausing_a_conversion_actually_stops_the_encoder() {
    use std::sync::Arc;
    use vrcast_studio_lib::commands::convert::{api as convert, ConvertStart};
    use vrcast_studio_lib::commands::AppState;
    use vrcast_studio_lib::store::db::Db;
    use vrcast_studio_lib::store::secrets::InMemorySecretStore;

    let Ok(ff) = vrcast_studio_lib::media::ffmpeg::locate("ffmpeg") else {
        eprintln!(
            "SKIPPED: no bundled FFmpeg. Run `npm run ffmpeg` for this check to check anything."
        );
        return;
    };

    let dir = std::env::temp_dir().join(format!("vrcast-pause-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).expect("could not make a working directory");
    let source = dir.join("source.mp4");
    let out = dir.join("ready.mp4");

    // Long enough to still be encoding when we pause it.
    let made = std::process::Command::new(&ff)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=1280x720:rate=30",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            "90",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
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
            target_kbps: Some(6_000),
            height: None,
            out_path: out.to_string_lossy().into_owned(),
            prefer_hardware: false,
        },
    )
    .await
    .expect("the conversion did not start");

    if !grew_past(&out, 0, Duration::from_secs(60)).await {
        let record = state.tasks.get(&task).ok().flatten();
        panic!("nothing was ever written — there was nothing to pause. task: {record:?}");
    }

    state.tasks.pause(&task).expect("the task would not pause");

    // Let the pause reach the encoder, then take a reading and see if it moves.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let frozen_at = size_of(&out);
    tokio::time::sleep(Duration::from_secs(3)).await;
    let still = size_of(&out);

    assert_eq!(
        frozen_at, still,
        "the file grew from {frozen_at} to {still} while the task was paused — \
         the pause freed the task's place in the queue without stopping any work"
    );

    // And it must come back to life, or "pause" would mean "abandon".
    state
        .tasks
        .resume(&task)
        .expect("the task would not resume");
    assert!(
        grew_past(&out, still, Duration::from_secs(30)).await,
        "the encoder never came back after resume"
    );

    let _ = state.tasks.cancel(&task);
    tokio::time::sleep(Duration::from_secs(2)).await;
    let _ = std::fs::remove_dir_all(&dir);
}
