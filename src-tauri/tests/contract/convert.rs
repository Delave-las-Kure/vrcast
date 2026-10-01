//! T112 — contract tests for the file-preparation commands.
//!
//! Contract: `contracts/ipc-commands.md`, "Preparing files".
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
use vrcast_studio_lib::commands::error::{DetailCode, ErrorCode};
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
        confirmed: false,
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
    // The refusal names what is wrong, so the interface has something to say beyond
    // the general "that will not do" the code alone carries.
    assert!(!err.details.is_empty(), "a refusal with nothing to say");
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
    let detail = err
        .details
        .iter()
        .find(|d| d.key == DetailCode::PlanNoSuchTrack)
        .unwrap_or_else(|| panic!("the missing track is not named: {err}"));
    // Numbered from one for people: "track 0 is missing" reads like a bug report.
    // The conversion happens in the core, once, so no catalogue has to remember it.
    assert_eq!(
        detail.params.get("number").and_then(|v| v.as_u64()),
        Some(8),
        "the track number is not the one a person sees: {err}"
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
        err.says(DetailCode::ConvertOutOverwritesSource),
        "it does not say what is at stake: {err}"
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

// ---------- a finished result, and one output path per preparation (T662) ----------

/// A clip long and heavy enough that re-encoding it is still running a second or two in.
fn long_clip(work: &Workspace, name: &str) -> String {
    let out = work.path(name);
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
            "aac",
        ])
        .arg(&out)
        .output()
        .expect("could not run the bundled FFmpeg");
    assert!(made.status.success(), "could not prepare a long clip");
    out
}

/// A request that re-encodes the picture on the processor: slow enough to be caught mid-way.
fn slow_request(src: &str, out: &str) -> ConvertStart {
    let mut ask = request(src, out);
    ask.height = Some(360);
    ask.target_kbps = Some(1_500);
    ask.prefer_hardware = false;
    ask
}

/// Attempt files a preparation left beside `out`.
fn attempts_beside(out: &str) -> Vec<String> {
    let out = std::path::Path::new(out);
    let prefix = format!("{}.", out.file_name().unwrap().to_string_lossy());
    std::fs::read_dir(out.parent().unwrap())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(&prefix) && n.ends_with(".vrcast-part"))
        .collect()
}

async fn wait_final(
    state: &AppState,
    task: &str,
    within: std::time::Duration,
) -> vrcast_studio_lib::tasks::state::TaskState {
    let deadline = std::time::Instant::now() + within;
    loop {
        if let Some(record) = state.tasks.get(task).ok().flatten() {
            if record.state.is_final() {
                return record.state;
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the task did not end in time"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn a_finished_result_is_not_replaced_without_a_yes() {
    if !has_ffmpeg() {
        return;
    }
    // QA-24B-03: a second "prepare" into the same place used to overwrite a finished
    // result from its first second, with no word said.
    let work = Workspace::new();
    let src = work.clip("source.mp4");
    let out = work.path("ready.mp4");
    std::fs::write(&out, b"hours of work").unwrap();
    let state = state();

    let err = convert::convert_start(&state, request(&src, &out))
        .await
        .expect_err("a finished result was about to be replaced without asking");

    assert_eq!(err.code, ErrorCode::ConfirmationRequired);
    assert!(
        err.says(DetailCode::ConvertOutExists),
        "it does not say what is at stake: {err}"
    );
    assert_eq!(
        std::fs::read(&out).unwrap(),
        b"hours of work",
        "the refusal touched the file it refused to replace"
    );
    assert!(
        state.tasks.list().unwrap().is_empty(),
        "a task was created for a start that was refused"
    );
}

#[tokio::test]
async fn a_confirmed_replacement_puts_the_new_result_in_place_once_it_is_checked() {
    if !has_ffmpeg() {
        return;
    }
    let work = Workspace::new();
    let src = work.clip("source.mp4");
    let out = work.path("ready.mp4");
    std::fs::write(&out, b"the old result").unwrap();
    let state = state();

    let mut ask = request(&src, &out);
    ask.confirmed = true;
    let task = convert::convert_start(&state, ask)
        .await
        .expect("a confirmed replacement did not start");
    let ended = wait_final(&state, &task, std::time::Duration::from_secs(120)).await;
    assert_eq!(
        ended,
        vrcast_studio_lib::tasks::state::TaskState::Completed,
        "{:?}",
        state.tasks.get(&task).unwrap().and_then(|t| t.error)
    );

    assert_ne!(
        std::fs::read(&out).unwrap(),
        b"the old result",
        "the task succeeded and the old result is still in place"
    );
    let verdict = convert::convert_validate(&out).await.unwrap();
    assert!(
        verdict.ok,
        "what was put in place does not play: {verdict:?}"
    );
    assert!(
        attempts_beside(&out).is_empty(),
        "the attempt's own file was left behind: {:?}",
        attempts_beside(&out)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_a_new_attempt_leaves_the_old_result_alone() {
    if !has_ffmpeg() {
        return;
    }
    // QA-24B-03, the half that destroyed work: cancelling the new attempt removed
    // `out_path` — which was the old, finished result.
    let work = Workspace::new();
    let src = long_clip(&work, "long.mp4");
    let out = work.path("ready.mp4");
    std::fs::write(&out, b"the old result, hours of it").unwrap();
    let state = state();

    let mut ask = slow_request(&src, &out);
    ask.confirmed = true;
    let task = convert::convert_start(&state, ask)
        .await
        .expect("the new attempt did not start");

    // Wait until the attempt is really writing, so the cancel lands mid-encode.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while attempts_beside(&out).is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "the new attempt never started writing"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    state.tasks.cancel(&task).expect("the cancel was refused");
    let ended = wait_final(&state, &task, std::time::Duration::from_secs(60)).await;
    assert_eq!(ended, vrcast_studio_lib::tasks::state::TaskState::Cancelled);

    assert_eq!(
        std::fs::read(&out).expect("the old result is gone"),
        b"the old result, hours of it",
        "cancelling the new attempt touched the old result"
    );
    assert!(
        attempts_beside(&out).is_empty(),
        "the cancelled attempt left its own file behind: {:?}",
        attempts_beside(&out)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_preparation_into_the_same_file_is_refused_while_the_first_is_running() {
    if !has_ffmpeg() {
        return;
    }
    let work = Workspace::new();
    let src = long_clip(&work, "long.mp4");
    let out = work.path("ready.mp4");
    let state = state();

    let first = convert::convert_start(&state, slow_request(&src, &out))
        .await
        .expect("the first preparation did not start");

    // The same file, spelled differently where the platform allows it — and confirmed,
    // so that nothing but the busy path can be what refuses it.
    let same = if cfg!(windows) {
        out.replace('\\', "/").to_uppercase()
    } else {
        out.clone()
    };
    let mut second = slow_request(&src, &same);
    second.confirmed = true;
    let err = convert::convert_start(&state, second)
        .await
        .expect_err("two preparations were let write one result");
    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert!(
        err.says(DetailCode::ConvertOutBusy),
        "it does not say the path is taken: {err}"
    );

    // And once the first has ended, the path is free again — the hold is not a leak.
    state.tasks.cancel(&first).unwrap();
    wait_final(&state, &first, std::time::Duration::from_secs(60)).await;
    let mut again = slow_request(&src, &out);
    again.confirmed = true;
    let third = convert::convert_start(&state, again)
        .await
        .expect("the path stayed held after the preparation that held it ended");
    state.tasks.cancel(&third).unwrap();
    wait_final(&state, &third, std::time::Duration::from_secs(60)).await;
}

// ---------- T670(3): an attempt a crashed run left behind ----------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_running_preparation_writes_down_where_its_attempt_goes() {
    if !has_ffmpeg() {
        return;
    }
    // Start-up finds abandoned attempts in the task journal, not on the disk — so the
    // journal has to say where each one went before the application can die mid-encode.
    let work = Workspace::new();
    let src = long_clip(&work, "long.mp4");
    let out = work.path("ready.mp4");
    let state = state();

    let task = convert::convert_start(&state, slow_request(&src, &out))
        .await
        .expect("the preparation did not start");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while attempts_beside(&out).is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "the attempt never started writing"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let attempt = attempts_beside(&out).remove(0);
    let noted = state.tasks.get(&task).unwrap().unwrap().resume_token;
    state.tasks.cancel(&task).unwrap();
    wait_final(&state, &task, std::time::Duration::from_secs(60)).await;

    let noted = noted.expect("the task did not write down where its attempt goes");
    assert_eq!(
        std::path::Path::new(&noted),
        std::path::Path::new(&work.path(&attempt)),
        "what the journal names is not the file being written"
    );
}

#[test]
fn only_the_attempt_shape_is_an_attempt() {
    use convert::is_attempt_name;
    assert!(is_attempt_name("film.mp4.0a1b2c3d.vrcast-part"));
    let made = convert::attempt_path("F:/out/film.mp4");
    assert!(is_attempt_name(made.file_name().unwrap().to_str().unwrap()));
    for not_one in [
        "film.mp4",
        "film.mp4.vrcast-part",
        "film.mp4.0a1b2c3.vrcast-part",
        "film.mp4.0a1b2c3dd.vrcast-part",
        "film.mp4.0A1B2C3D.vrcast-part",
        "film.mp4.zzzzzzzz.vrcast-part",
        ".0a1b2c3d.vrcast-part",
        "film.mp4.0a1b2c3d.vrcast-part.mp4",
    ] {
        assert!(
            !is_attempt_name(not_one),
            "{not_one} was taken for an attempt"
        );
    }
}

#[test]
fn start_up_removes_only_the_attempts_of_preparations_nobody_is_running() {
    use vrcast_studio_lib::tasks::state::{TaskKind, TaskState};
    use vrcast_studio_lib::tasks::store::{save_resume_token, save_state, upsert, TaskRecord};

    let work = Workspace::new();
    let file = |name: &str, body: &[u8]| {
        let p = work.path(name);
        std::fs::write(&p, body).unwrap();
        p
    };
    let source = file("source.mp4", b"the source");
    let result = file("ready.mp4", b"a finished result");
    let crashed = file("ready.mp4.0a1b2c3d.vrcast-part", b"half an encode");
    let crashed_paused = file("other.mp4.1b2c3d4e.vrcast-part", b"half an encode");
    let kept_by_failure = file("ready.mp4.2c3d4e5f.vrcast-part", b"failed its check");
    let live = file("live.mp4.3d4e5f60.vrcast-part", b"still being written");
    let stray = file("stray.mp4.4e5f6071.vrcast-part", b"nobody named it");

    let db = Arc::new(Db::open_in_memory().unwrap());
    let task = |id: &str, kind: TaskKind, token: &str| {
        upsert(&db, &TaskRecord::new(id, kind, None)).unwrap();
        save_resume_token(&db, id, token).unwrap();
    };
    // The application died mid-encode: the row still says running, under an owner that is
    // no longer there.
    task("crashed", TaskKind::Convert, &crashed);
    save_state(&db, "crashed", TaskState::Running, None).unwrap();
    db.with_conn(|c| {
        c.execute(
            "UPDATE tasks SET owner_pid = 4000000000, owner_identity = 'gone' WHERE id = 'crashed'",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    // A previous start-up already moved one like it to paused; no owner was ever stamped.
    task("crashed-paused", TaskKind::Convert, &crashed_paused);
    save_state(&db, "crashed-paused", TaskState::Paused, None).unwrap();
    // Finished: the attempt is kept on purpose and the task's error names it.
    task("failed", TaskKind::Convert, &kept_by_failure);
    save_state(&db, "failed", TaskState::Failed, None).unwrap();
    // Running in an instance that is alive (this process): its file is being written.
    task("live", TaskKind::Convert, &live);
    save_state(&db, "live", TaskState::Running, None).unwrap();
    // Abandoned, but naming a result and a source — not attempts, never touched.
    task("names-result", TaskKind::Convert, &result);
    task("names-source", TaskKind::Convert, &source);
    // Another kind of task: its marker means something else entirely.
    task(
        "upload",
        TaskKind::Upload,
        &file("up.mp4.5f607182.vrcast-part", b"x"),
    );

    // Start-up, as the application assembles itself.
    let _state = AppState::with_db(db.clone(), Arc::new(InMemorySecretStore::new())).unwrap();

    let there = |p: &str| std::path::Path::new(p).exists();
    assert!(!there(&crashed), "a crashed run's attempt was left behind");
    assert!(
        !there(&crashed_paused),
        "a paused abandoned attempt was left behind"
    );
    assert!(
        there(&kept_by_failure),
        "an attempt a finished task kept was removed"
    );
    assert!(
        there(&live),
        "an attempt a live instance is writing was removed"
    );
    assert!(
        there(&stray),
        "a file no task names was removed — the disk was searched"
    );
    assert_eq!(std::fs::read(&result).unwrap(), b"a finished result");
    assert_eq!(std::fs::read(&source).unwrap(), b"the source");
    assert!(there(&work.path("up.mp4.5f607182.vrcast-part")));

    // And once more changes nothing: repeating is safe.
    assert_eq!(convert::tidy_abandoned_attempts(&db), 0);
}
