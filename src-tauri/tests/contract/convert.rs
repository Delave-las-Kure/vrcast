//! T112 — contract tests for the file-preparation commands.
//!
//! Contract: `contracts/ipc-commands.md`, "Подготовка файлов".
//!
//! Only what is visible from outside: the shape of the answer, and which refusal
//! carries which code. The code is not a detail — it decides whether the interface
//! highlights a field or shows a failure notice, and a typo is not a failure.
//!
//! These need the bundled FFmpeg. It weighs a hundred and forty megabytes, is not
//! in the repository, and is put in place by `npm run ffmpeg`; without it each
//! check says so out loud rather than quietly passing.

use std::sync::Arc;
use vrcast_studio_lib::commands::convert::{api as convert, ConvertStart};
use vrcast_studio_lib::commands::error::ErrorCode;
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::media::ffmpeg;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;

/// Is the bundled build present? Half these checks have nothing to do without it.
fn has_ffmpeg() -> bool {
    if ffmpeg::locate("ffprobe").is_ok() && ffmpeg::locate("ffmpeg").is_ok() {
        return true;
    }
    eprintln!("SKIPPED: no bundled FFmpeg. Run `npm run ffmpeg` for this check to check anything.");
    false
}

fn state() -> AppState {
    AppState::with_db(
        Arc::new(Db::open_in_memory().unwrap()),
        Arc::new(InMemorySecretStore::new()),
    )
    .expect("could not assemble the application state")
}

/// A working directory that cleans up after itself.
struct Workspace(std::path::PathBuf);

impl Workspace {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("vrcast-c-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).expect("could not make a working directory");
        Self(dir)
    }

    fn path(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }

    /// Encode a short, deliberately incompatible clip.
    fn clip(&self, name: &str) -> String {
        let out = self.path(name);
        let ff = ffmpeg::locate("ffmpeg").expect("no bundled FFmpeg");
        let made = std::process::Command::new(ff)
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
            .arg(&out)
            .output()
            .expect("could not run the bundled FFmpeg");
        assert!(made.status.success(), "could not prepare a clip");
        out
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn request(path: &str, out_path: &str) -> ConvertStart {
    ConvertStart {
        path: path.to_owned(),
        audio_track: 0,
        target_kbps: None,
        height: None,
        out_path: out_path.to_owned(),
        prefer_hardware: true,
    }
}

// ---------- probing ----------

#[tokio::test]
async fn probing_the_bundled_build_answers_what_the_contract_promises() {
    if !has_ffmpeg() {
        return;
    }
    let info = vrcast_studio_lib::commands::api::ffmpeg_probe_self()
        .await
        .expect("the bundled build failed its own check");

    assert!(
        info.version.starts_with("ffmpeg version"),
        "{}",
        info.version
    );
    assert!(
        !info.path.is_empty(),
        "it did not say where the build lives"
    );
    assert!(
        info.has_x264,
        "the contract promises a refusal without libx264, and this reported success"
    );
}

#[tokio::test]
async fn probing_a_missing_file_is_an_input_error() {
    if !has_ffmpeg() {
        return;
    }
    // A typo in a path is not a failure of the application, and the interface is
    // meant to highlight the field rather than show a failure notice. The code is
    // the only thing that tells those apart.
    let err = vrcast_studio_lib::commands::api::source_probe("F:/no/such/file.mp4")
        .await
        .expect_err("probing a missing file succeeded");

    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert!(!err.message.is_empty(), "a refusal with nothing to read");
    assert!(!err.hint.is_empty(), "a refusal with no hint what to do");
}

#[tokio::test]
async fn probing_something_that_is_not_video_names_the_reason() {
    if !has_ffmpeg() {
        return;
    }
    let work = Workspace::new();
    let path = work.path("notes.txt");
    std::fs::write(&path, "vrcast: not a video at all").unwrap();

    let err = vrcast_studio_lib::commands::api::source_probe(&path)
        .await
        .expect_err("a text file was probed as video");

    assert_eq!(err.code, ErrorCode::InvalidInput);
    // The prober's own complaint is kept: "moov atom not found" is cryptic but
    // searchable, and "the file is bad" is neither.
    assert!(err.cause.is_some(), "the prober's own words were dropped");
}

// ---------- previewing the work ----------

#[tokio::test]
async fn the_preview_says_whether_anything_will_be_re_encoded() {
    if !has_ffmpeg() {
        return;
    }
    // Re-encoding costs hours where copying costs minutes. Knowing which one is
    // about to happen is the whole point of showing a preview at all.
    let work = Workspace::new();
    let src = work.clip("source.mp4");

    let preview = convert::convert_preview(&request(&src, &work.path("ready.mp4")))
        .await
        .expect("the preview did not come together");

    // Six-channel AC-3 against a stereo AAC target: the audio must be re-encoded.
    assert!(
        !preview.lossless,
        "a six-channel AC-3 track was called lossless"
    );
    assert_eq!(preview.source.width, 320);
    assert_eq!(preview.plan.audio_track, 0);
}

#[tokio::test]
async fn asking_for_a_track_that_is_not_there_is_refused_before_anything_starts() {
    if !has_ffmpeg() {
        return;
    }
    let work = Workspace::new();
    let src = work.clip("source.mp4");

    let mut ask = request(&src, &work.path("ready.mp4"));
    ask.audio_track = 7;

    let err = convert::convert_preview(&ask)
        .await
        .expect_err("a track that does not exist was accepted");
    assert_eq!(err.code, ErrorCode::InvalidInput);
    // Numbered from one for people: "track 0 is missing" reads like a bug report.
    assert!(
        err.message.contains("дорожки 8"),
        "the track number is not the one a person sees: {}",
        err.message
    );
}

// ---------- starting the work ----------

#[tokio::test]
async fn writing_over_the_source_is_refused() {
    if !has_ffmpeg() {
        return;
    }
    // The encoder opens the output for writing before it has read anything, so
    // this would destroy the only copy of the original with no way back.
    let work = Workspace::new();
    let src = work.clip("source.mp4");
    let state = state();

    let err = convert::convert_start(&state, request(&src, &src))
        .await
        .expect_err("the source was accepted as its own destination");

    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert!(
        err.message.contains("исходник"),
        "it does not say what is at stake: {}",
        err.message
    );
}

#[tokio::test]
async fn a_started_conversion_returns_a_task_number_at_once() {
    if !has_ffmpeg() {
        return;
    }
    // FR-080. Preparing takes minutes to hours; a command that returned when it
    // was done would freeze the interface for exactly that long.
    let work = Workspace::new();
    let src = work.clip("source.mp4");
    let out = work.path("ready.mp4");
    let state = state();

    let started = std::time::Instant::now();
    let task = convert::convert_start(&state, request(&src, &out))
        .await
        .expect("the conversion did not start");
    let took = started.elapsed();

    assert!(!task.is_empty(), "no task number came back");
    assert!(
        took < std::time::Duration::from_secs(10),
        "the command took {took:?} before answering — it waited for the work"
    );

    // Let it finish, then check the file really is playable: this is the whole
    // chain — encode, then validate — and it is what FR-027 is about.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        let record = state.tasks.get(&task).ok().flatten();
        if record.as_ref().is_some_and(|t| t.state.is_final()) {
            let record = record.unwrap();
            assert_eq!(
                record.state,
                vrcast_studio_lib::tasks::state::TaskState::Completed,
                "the conversion did not succeed: {:?}",
                record.error
            );
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the conversion did not finish in time"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    assert!(
        std::path::Path::new(&out).exists(),
        "the task succeeded but produced no file"
    );
    let verdict = convert::convert_validate(&out)
        .await
        .expect("validation did not run");
    assert!(
        verdict.ok,
        "the prepared file does not play: {:?}",
        verdict.problems
    );
}

#[tokio::test]
async fn validating_a_damaged_file_refuses_it() {
    if !has_ffmpeg() {
        return;
    }
    // FR-027: a file that does not pass must not be offered for upload. A broken
    // encode opens fine and reports the right duration — only a full decode knows.
    let work = Workspace::new();
    let src = work.clip("source.mp4");

    let damaged = work.path("damaged.mp4");
    let mut bytes = std::fs::read(&src).unwrap();
    let from = bytes.len() / 3;
    let to = (from + bytes.len() / 4).min(bytes.len());
    for b in &mut bytes[from..to] {
        *b = 0x5A;
    }
    std::fs::write(&damaged, &bytes).unwrap();

    let verdict = convert::convert_validate(&damaged)
        .await
        .expect("validation did not run");
    assert!(!verdict.ok, "a damaged file passed validation");
    assert!(
        !verdict.problems.is_empty(),
        "it was refused without saying why"
    );
}
