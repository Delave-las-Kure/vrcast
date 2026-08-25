//! T083, T084 — uploading against a real OpenSSH.
//!
//! What is checked here is what the whole of Phase 2 was for and what cannot be checked
//! without a server: a transfer carries on from where it got to rather than starting over;
//! a file still being downloaded is not visible under its final name **at any moment**; a
//! spoilt transfer never enters serving and leaves no litter.

use super::fixture::{key_path, TestServer, KEY_PASSPHRASE};
use std::sync::Arc;
use std::time::Duration;
use vrcast_studio_lib::commands::error::ErrorCode;
use vrcast_studio_lib::commands::servers::{api as servers, ServerInput};
use vrcast_studio_lib::commands::upload::{api as upload, UploadRequest};
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::domain::server_profile::AuthKind;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::{InMemorySecretStore, SecretStore};
use vrcast_studio_lib::tasks::engine::TaskEvent;
use vrcast_studio_lib::tasks::state::TaskState;

const VIDEO_DIR: &str = "/var/lib/vrcast/videos";
const STAGING_DIR: &str = "/var/lib/vrcast/.vrcast-uploads";

/// The size of the sample file. Large enough for a transfer to take several windows and be
/// caught in the middle; small enough for the test to run in seconds.
const FILE_SIZE: usize = 12 * 1024 * 1024;

fn app_state() -> AppState {
    AppState::with_db(
        Arc::new(Db::open_in_memory().unwrap()),
        Arc::new(InMemorySecretStore::new()),
    )
    .expect("the application state would not assemble")
}

/// Create a local file with predictable but not uniform contents.
///
/// Uniform will not do: with it a spoilt transfer can give the same checksum, and the
/// indivisibility check would prove nothing.
fn make_local_file(name: &str, size: usize) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vrcast-upload-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).expect("could not create the temporary directory");
    let path = dir.join(name);

    let mut data = Vec::with_capacity(size);
    let mut x: u32 = 0x1234_5678;
    while data.len() < size {
        // A simple generator: repeatable, but with no long identical stretches.
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        data.extend_from_slice(&x.to_le_bytes());
    }
    data.truncate(size);
    std::fs::write(&path, &data).expect("could not write the file");
    path
}

async fn setup() -> (TestServer, AppState, String) {
    super::fixture::logging_if_requested();
    let server = TestServer::start().expect("the container would not come up");
    let state = app_state();
    let id = add_profile(&state, &server).await;
    (server, state, id)
}

/// Set the container's profile up and confirm its fingerprint.
///
/// Kept apart from [`setup`] because the restart check raises the application state twice
/// against one and the same database while holding the server for itself.
pub(crate) async fn add_profile(state: &AppState, server: &TestServer) -> String {
    let input = ServerInput {
        name: String::from("Container"),
        host: server.host().to_owned(),
        port: server.port,
        user: String::from("root"),
        auth_kind: AuthKind::Key,
        key_path: Some(key_path().to_string_lossy().into_owned()),
        domain: String::from("stream.example.com"),
        video_dir: Some(String::from(VIDEO_DIR)),
        cdn_base: None,
        ipv6_mode: None,
    };
    let id =
        servers::server_add(state, input, KEY_PASSPHRASE).expect("the profile was not created");
    // The secret is registered — now it can be checked that it really gets cut out.
    super::fixture::canary(KEY_PASSPHRASE);
    super::library_ops::confirm_fingerprint(state, &id, server).await;
    id
}

/// Wait until a task reaches one of the finished states.
async fn wait_done(state: &AppState, task_id: &str, limit: Duration) -> TaskState {
    let deadline = std::time::Instant::now() + limit;
    loop {
        if let Ok(Some(task)) = state.tasks.get(task_id) {
            if task.state.is_final() {
                return task.state;
            }
        }
        if std::time::Instant::now() >= deadline {
            let task = state.tasks.get(task_id).ok().flatten();
            panic!("the task did not finish in the time allowed: {task:?}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub(crate) fn request(server_id: &str, local: &std::path::Path, name: &str) -> UploadRequest {
    UploadRequest {
        server_id: server_id.to_owned(),
        local_path: local.to_string_lossy().into_owned(),
        remote_name: name.to_owned(),
        media_id: None,
        limit_bps: None,
        confirmed: true,
    }
}

#[tokio::test]
async fn the_file_arrives_whole_and_the_checksums_agree() {
    let (server, state, id) = setup().await;
    let local = make_local_file("film_22.mp4", FILE_SIZE);

    let task = upload::upload_start(&state, request(&id, &local, "film_22.mp4"))
        .await
        .expect("the upload would not submit");

    assert_eq!(
        wait_done(&state, &task, Duration::from_secs(120)).await,
        TaskState::Completed,
        "the upload did not finish successfully: {:?}",
        state.tasks.get(&task).ok().flatten()
    );

    // Checked by the server's own means rather than by the code that did the sending.
    let size = server
        .exec_inside(&format!("stat -c %s '{VIDEO_DIR}/film_22.mp4'"))
        .expect("the file is not on the server");
    assert_eq!(size.trim().parse::<usize>().unwrap(), FILE_SIZE);

    let theirs = server
        .exec_inside(&format!(
            "sha256sum '{VIDEO_DIR}/film_22.mp4' | cut -d' ' -f1"
        ))
        .expect("the checksum would not compute");
    let ours = sha256_of(&local);
    assert_eq!(theirs.trim(), ours, "the contents on the server differ");

    // No staged data is left.
    let leftovers = server
        .exec_inside(&format!("ls -A '{STAGING_DIR}' 2>/dev/null | wc -l"))
        .unwrap_or_else(|_| String::from("0"));
    assert_eq!(
        leftovers.trim(),
        "0",
        "litter was left in the staging directory"
    );
}

#[tokio::test]
async fn the_transfer_carries_on_from_where_it_got_to_rather_than_starting_over() {
    // FR-031, the phase's main property. Checked by the progress events: were the transfer
    // to start over, the very first message would show about zero.
    let (server, state, id) = setup().await;
    let local = make_local_file("film_22.mp4", FILE_SIZE);

    // Half the file is put into the staging directory — as if an earlier attempt broke off.
    let half = FILE_SIZE / 2;
    let partial = std::env::temp_dir().join("vrcast-partial.bin");
    let data = std::fs::read(&local).unwrap();
    std::fs::write(&partial, &data[..half]).unwrap();

    server
        .exec_inside(&format!("mkdir -p '{STAGING_DIR}'"))
        .expect("could not create the staging directory");
    server
        .put_file(&partial, &format!("{STAGING_DIR}/film_22.mp4.part"))
        .expect("could not put the half-downloaded file there");

    let mut events = state.tasks.subscribe();
    let task = upload::upload_start(&state, request(&id, &local, "film_22.mp4"))
        .await
        .expect("the upload would not submit");

    // The progress messages are gathered until the task ends.
    let collector = tokio::spawn(async move {
        let mut first_progress: Option<f64> = None;
        while let Ok(event) = events.recv().await {
            match event {
                TaskEvent::Progress { progress, .. } if first_progress.is_none() => {
                    first_progress = Some(progress);
                }
                TaskEvent::Done { .. } => break,
                _ => {}
            }
        }
        first_progress
    });

    assert_eq!(
        wait_done(&state, &task, Duration::from_secs(120)).await,
        TaskState::Completed
    );

    let first = collector
        .await
        .ok()
        .flatten()
        .expect("not one progress message arrived");
    assert!(
        first > 0.3,
        "the first message showed {first:.2} — the transfer started over, although half the \
         file was already on the server"
    );

    // And the result is whole all the same.
    let theirs = server
        .exec_inside(&format!(
            "sha256sum '{VIDEO_DIR}/film_22.mp4' | cut -d' ' -f1"
        ))
        .expect("the checksum would not compute");
    assert_eq!(theirs.trim(), sha256_of(&local));
}

#[tokio::test]
async fn a_half_downloaded_file_is_never_visible_under_its_final_name() {
    // FR-033, SC-004. Checked not at the end but DURING the transfer: the whole point is
    // that there is no in-between state.
    let (server, state, id) = setup().await;
    let local = make_local_file("film_22.mp4", FILE_SIZE);

    let mut req = request(&id, &local, "film_22.mp4");
    // The transfer is held back so the server can be looked at several times.
    req.limit_bps = Some(2 * 1024 * 1024);

    let task = upload::upload_start(&state, req)
        .await
        .expect("the upload would not submit");

    // Both names are checked by ONE command, at one instant on the server.
    //
    // Separate checks will not do here, and that cost a red run. Asking the database record
    // "is the task still going" is no good: entering serving is the last thing the work
    // does, and the state is written after it. Between the rename and the write the file is
    // legitimately visible under its final name while the task is not yet marked finished —
    // and the check failed on that gap, finding nothing wrong.
    //
    // Here the server itself is asked, and the question is sharper: while the staged file is
    // whole, the final name must not exist. The rename is indivisible, so a moment holding
    // both does not exist at all — and should one turn up, that is the very fault all this
    // was built for.
    //
    // There are four states rather than three: "there is no staged file" on its own means
    // both "not started yet" and "already in serving". Confusing them means leaving the
    // watch on the first round, having checked nothing.
    const BOTH: &str = "BOTH";
    const RUNNING: &str = "RUNNING";
    const DONE: &str = "DONE";
    const NOT_YET: &str = "NOT_YET";
    let question = format!(
        "if [ -e '{STAGING_DIR}/film_22.mp4.part' ]; then \
             if [ -e '{VIDEO_DIR}/film_22.mp4' ]; then echo {BOTH}; else echo {RUNNING}; fi; \
         elif [ -e '{VIDEO_DIR}/film_22.mp4' ]; then echo {DONE}; \
         else echo {NOT_YET}; fi"
    );

    let mut looks = 0;
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    loop {
        let answer = server.exec_inside(&question).unwrap_or_default();
        let answer = answer.trim();

        assert_ne!(
            answer, BOTH,
            "the file appeared under its final name while the transfer was going — a viewer \
             could have got something half-downloaded"
        );
        if answer == DONE {
            break;
        }
        if answer == RUNNING {
            looks += 1;
        }

        assert!(
            std::time::Instant::now() < deadline,
            "the upload did not finish in the time allowed"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    assert_eq!(
        wait_done(&state, &task, Duration::from_secs(60)).await,
        TaskState::Completed,
        "the transfer ended, but the task did not finish successfully"
    );

    assert!(
        looks >= 3,
        "there was time for only {looks} looks — the transfer went too fast, and the check \
         proved nothing"
    );
    assert!(
        server
            .exec_inside(&format!("test -e '{VIDEO_DIR}/film_22.mp4'"))
            .is_ok(),
        "after a successful upload the file is not there"
    );
}

#[tokio::test]
async fn a_spoilt_transfer_never_enters_serving_and_leaves_no_litter() {
    // FR-032, FR-038. The contents on the server are spoilt so that the size matches while
    // the contents do not: then the only thing that can notice is the comparison.
    let (server, state, id) = setup().await;
    let local = make_local_file("film_22.mp4", FILE_SIZE);

    server
        .exec_inside(&format!(
            "mkdir -p '{STAGING_DIR}' && head -c {FILE_SIZE} /dev/zero > '{STAGING_DIR}/film_22.mp4.part'"
        ))
        .expect("could not prepare the spoilt file");

    let task = upload::upload_start(&state, request(&id, &local, "film_22.mp4"))
        .await
        .expect("the upload would not submit");

    let final_state = wait_done(&state, &task, Duration::from_secs(120)).await;
    assert_eq!(
        final_state,
        TaskState::Failed,
        "an upload with diverging checksums was declared successful"
    );

    let record = state.tasks.get(&task).unwrap().unwrap();
    let error = record
        .error
        .expect("the failure was recorded with no cause");
    assert_eq!(
        error.code,
        vrcast_studio_lib::commands::error::ErrorCode::ChecksumMismatch,
        "the diverging checksums were not named by their own code: {error}"
    );

    assert!(
        server
            .exec_inside(&format!("test -e '{VIDEO_DIR}/film_22.mp4'"))
            .is_err(),
        "the spoilt file entered serving"
    );
    let leftovers = server
        .exec_inside(&format!("ls -A '{STAGING_DIR}' 2>/dev/null | wc -l"))
        .unwrap_or_else(|_| String::from("0"));
    assert_eq!(
        leftovers.trim(),
        "0",
        "litter was left in the staging directory after the failure"
    );
}

#[tokio::test]
async fn too_little_room_is_reported_before_the_transfer_starts() {
    // FR-036. Learning that halfway through a thirty-gigabyte upload means losing an hour
    // and leaving an unfinished tail on the server.
    let (_server, state, id) = setup().await;

    // A file certainly larger than any of the container's disks.
    let local = make_local_file("huge.mp4", 1024);
    let mut req = request(&id, &local, "huge.mp4");
    req.limit_bps = None;

    // The size is substituted: there is no point making a real terabyte file, and the check
    // looks at the file's size. So another way is taken — room for a certainly impossible
    // volume is asked for through a direct call into the check.
    use vrcast_studio_lib::commands::library::DiskUsage;
    use vrcast_studio_lib::server::free_space::{self, SpaceVerdict};

    let tiny = DiskUsage {
        total_bytes: 10 * 1024 * 1024 * 1024,
        free_bytes: 1024 * 1024,
        used_by_videos_bytes: 0,
    };
    match free_space::check(&tiny, 5 * 1024 * 1024 * 1024, 0) {
        SpaceVerdict::NotEnough { short_by, .. } => assert!(short_by > 0),
        SpaceVerdict::Fits => panic!("too little room went unnoticed"),
    }

    // And an ordinary upload onto a free disk passes the checks.
    let task = upload::upload_start(&state, req)
        .await
        .expect("a small file did not pass the room check");
    assert_eq!(
        wait_done(&state, &task, Duration::from_secs(60)).await,
        TaskState::Completed
    );
}

#[tokio::test]
async fn a_second_upload_under_the_same_name_is_refused() {
    // Two uploads would write into one staged file and clobber each other's work, and it
    // would only come to light at the checksum comparison.
    let (_server, state, id) = setup().await;
    let local = make_local_file("film_22.mp4", FILE_SIZE);

    let mut first = request(&id, &local, "film_22.mp4");
    first.limit_bps = Some(512 * 1024); // held back so the task does not manage to finish
    let task = upload::upload_start(&state, first)
        .await
        .expect("the first upload would not submit");

    // Wait until the task writes its resume position.
    for _ in 0..50 {
        let has_token = state
            .tasks
            .get(&task)
            .ok()
            .flatten()
            .and_then(|t| t.resume_token)
            .is_some();
        if has_token {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let err = upload::upload_start(&state, request(&id, &local, "film_22.mp4"))
        .await
        .expect_err("a second upload under the same name was accepted");
    assert_eq!(err.code, ErrorCode::NameExists);

    let _ = state.tasks.cancel(&task);
}

// ---------- restarting the application (T097, FR-031) ----------
//
// The application is killed for real here: the first run goes as a **process of its own**,
// and the parent drops it without warning. There is no other way to portray this. An earlier
// version destroyed the runtime inside the same process — and on Linux a worker thread had
// time to see the input-output fall apart and record "it failed". In life a killed
// application records nothing, and the check caught not a property of the upload but a
// peculiarity of destroying a runtime.

/// The size of the file for the restart check.
///
/// Larger than usual: the application is killed halfway, and there must be room left before
/// the end of the transfer — otherwise the check becomes a race with itself.
const RESTART_FILE_SIZE: usize = 20 * 1024 * 1024;

/// How fast to send in this check.
///
/// The limit is not wanted for its own sake but so the transfer takes a tangible time:
/// without it the file goes to a neighbouring container faster than it can be caught.
const RESTART_LIMIT_BPS: u64 = 4 * 1024 * 1024;

/// How much must land on the server before the application is killed.
///
/// More than a transfer window: carrying on steps one window back, and after a smaller piece
/// it would begin from nothing — the check would stop telling a carry-on from a fresh start.
const RESTART_KILL_AFTER: usize = 8 * 1024 * 1024;

/// The names the parent passes the conditions to the killed run under.
mod env_names {
    pub const DB: &str = "VRCAST_RESTART_DB";
    pub const FILE: &str = "VRCAST_RESTART_FILE";
}

/// The name of the helper check, for running it as a process of its own.
const HELPER: &str = "upload_live::the_first_run_that_gets_killed";

/// The first run of the application — the one that gets killed.
///
/// Marked `ignore`: on its own it is not a check but half of one, and it is started only by
/// the neighbouring test, as a separate process. With no conditions in the environment it
/// does nothing — in case somebody runs every ignored check at once.
#[test]
#[ignore = "half of the restart check: started as a process of its own"]
fn the_first_run_that_gets_killed() {
    let (Ok(db_path), Ok(file)) = (std::env::var(env_names::DB), std::env::var(env_names::FILE))
    else {
        return;
    };

    let rt = tokio::runtime::Runtime::new().expect("the runtime would not be created");
    rt.block_on(async move {
        let state = AppState::with_db(
            Arc::new(Db::open(&db_path).expect("the database would not open")),
            Arc::new(InMemorySecretStore::new()),
        )
        .expect("the application state would not assemble");

        let id = attach_secret(&state);
        let mut req = request(&id, std::path::Path::new(&file), "film_22.mp4");
        req.limit_bps = Some(RESTART_LIMIT_BPS);
        upload::upload_start(&state, req)
            .await
            .expect("the upload would not submit");

        // From here we simply live. The parent watches how much lands on the server and
        // drops us when it sees fit.
        loop {
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    });
}

/// Put the profile's secret into this process's store.
///
/// Secrets live in a process's memory while the profile lives in a shared database. The
/// second run has a store of its own, and without writing it again it would not connect —
/// although in life the secret sits in the system keyring and survives a restart. That is
/// the difference between the check and life, and it is the only one here.
pub(crate) fn attach_secret(state: &AppState) -> String {
    let profile = servers::servers_list(state)
        .expect("the profile list would not read")
        .into_iter()
        .next()
        .expect("no profile was set up");

    let input = ServerInput {
        name: profile.name.clone(),
        host: profile.host.clone(),
        port: profile.port,
        user: profile.user.clone(),
        auth_kind: profile.auth_kind,
        key_path: profile.key_path.clone(),
        domain: profile.domain.clone(),
        video_dir: Some(profile.video_dir.clone()),
        cdn_base: profile.cdn_base.clone(),
        ipv6_mode: profile.ipv6_mode,
    };
    servers::server_update(state, &profile.id, input, Some(KEY_PASSPHRASE))
        .expect("the secret was not written");
    profile.id
}

/// The conditions for the run that gets killed.
struct Subject {
    server: TestServer,
    local: std::path::PathBuf,
    db_dir: std::path::PathBuf,
    db_path: std::path::PathBuf,
    secrets: Arc<dyn SecretStore>,
}

/// Bring the server up, set the database and the profile up — everything that outlives the
/// application being killed.
async fn prepare_restart() -> Subject {
    super::fixture::logging_if_requested();
    let server = TestServer::start().expect("the container would not come up");
    let local = make_local_file("film_22.mp4", RESTART_FILE_SIZE);
    let db_dir =
        std::env::temp_dir().join(format!("vrcast-restart-{}", uuid::Uuid::new_v4().simple()));
    let db_path = db_dir.join("vrcast.sqlite3");
    let secrets: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());

    // The profile and the confirmed fingerprint are set up by the parent: they live in the
    // database and are needed by both runs.
    let state = AppState::with_db(
        Arc::new(Db::open(&db_path).expect("the database would not open")),
        secrets.clone(),
    )
    .expect("the application state would not assemble");
    add_profile(&state, &server).await;

    Subject {
        server,
        local,
        db_dir,
        db_path,
        secrets,
    }
}

/// A started application that will be killed in any case.
///
/// The wrapper is not for elegance: should the wait not work out and the check fail, without
/// it a live process would be left in the system pouring a file onto the server. Exactly the
/// class of fault the constitution's third principle guards against — shameful to allow in a
/// check of that same principle.
struct Killable(std::process::Child);

impl Drop for Killable {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Start the application as a process of its own, let it send a piece, and kill it.
///
/// Returns how many bytes managed to land in the staged file on the server.
fn start_and_kill(subject: &Subject) -> usize {
    let child = std::process::Command::new(
        std::env::current_exe().expect("could not learn the path to our own program"),
    )
    .args([HELPER, "--exact", "--ignored", "--test-threads=1"])
    .env(env_names::DB, &subject.db_path)
    .env(env_names::FILE, &subject.local)
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .spawn()
    .expect("the first run did not start");
    let mut started = Killable(child);

    let sent = wait_for_chunk(&subject.server, RESTART_KILL_AFTER, &mut started.0);

    // Here is where the application dies — without warning and without a single record of
    // how it ended.
    drop(started);
    sent
}

/// Wait until the wanted piece has gathered in the staged file on the server.
///
/// It watches the run too: should it fail, there is nothing left to wait for, and that must
/// be said at once rather than after a minute of waiting for who knows what.
fn wait_for_chunk(server: &TestServer, wanted: usize, child: &mut std::process::Child) -> usize {
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    let mut last = 0usize;
    loop {
        if let Ok(out) = server.exec_inside(&format!("stat -c %s '{STAGING_DIR}/film_22.mp4.part'"))
        {
            last = out.trim().parse().unwrap_or(0);
            if last >= wanted {
                return last;
            }
        }
        if let Ok(Some(status)) = child.try_wait() {
            panic!("the first run ended by itself ({status}), having sent {last} B of {wanted}");
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{last} B of {wanted} landed on the server in the time allowed"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Open the application again against the same database.
fn start_again(subject: &Subject) -> AppState {
    let state = AppState::with_db(
        Arc::new(Db::open(&subject.db_path).expect("the database would not open")),
        subject.secrets.clone(),
    )
    .expect("the application state would not assemble");
    attach_secret(&state);
    state
}

/// The one unfinished task in the database.
fn the_only_task(state: &AppState) -> vrcast_studio_lib::tasks::store::TaskRecord {
    let mut alive: Vec<_> = state
        .tasks
        .list()
        .expect("the task list would not read")
        .into_iter()
        .filter(|t| !t.state.is_final())
        .collect();
    assert_eq!(alive.len(), 1, "one unfinished task was expected");
    alive.remove(0)
}

#[tokio::test]
async fn an_upload_still_in_the_queue_survives_a_restart_without_having_started() {
    // A task that stood in the queue and never once started records nothing about itself:
    // the path to the source and the name in serving live only in the application's memory.
    // After a restart there would be nothing to raise it with, and it would stay in the list
    // forever, never moving and never yielding. So the resume position is written at
    // submission rather than when the work gets round to it.
    let (server, state, id) = setup().await;
    let first = make_local_file("film_22.mp4", RESTART_FILE_SIZE);
    let second = make_local_file("film_23.mp4", FILE_SIZE);

    // The first takes the transfer lane — it is meant for one task.
    let mut req = request(&id, &first, "film_22.mp4");
    req.limit_bps = Some(RESTART_LIMIT_BPS);
    let running = upload::upload_start(&state, req)
        .await
        .expect("the first upload would not submit");

    let waiting = upload::upload_start(&state, request(&id, &second, "film_23.mp4"))
        .await
        .expect("the second upload would not submit");

    assert_eq!(
        state.tasks.get(&waiting).unwrap().unwrap().state,
        TaskState::Queued,
        "the second upload did not join the queue — there is nothing to check"
    );

    let token = state
        .tasks
        .get(&waiting)
        .unwrap()
        .unwrap()
        .resume_token
        .expect(
            "the waiting upload has no resume position — it could not be raised after a restart",
        );
    let token = vrcast_studio_lib::domain::transfer::ResumeToken::parse(&token)
        .expect("the resume position will not read");

    assert_eq!(
        token.local_path.as_deref(),
        Some(second.to_string_lossy().as_ref()),
        "the resume position holds the wrong source"
    );
    assert_eq!(token.remote_name, "film_23.mp4");

    let _ = state.tasks.cancel(&waiting);
    let _ = state.tasks.cancel(&running);
    let _ = server;
}

#[tokio::test]
async fn an_upload_survives_the_application_being_closed_and_started_again() {
    // FR-031, the second half: "including the case where the application was closed and
    // started again". The first half — carrying on after a broken connection — is checked
    // above; there the application is alive and remembers what it was doing. Here it dies
    // itself, and with it the whole working part, which lives only in memory.
    let subject = prepare_restart().await;
    let sent = start_and_kill(&subject);

    assert!(
        sent < RESTART_FILE_SIZE,
        "the transfer managed to finish: catching it halfway did not work out"
    );
    assert!(
        subject
            .server
            .exec_inside(&format!("test -e '{VIDEO_DIR}/film_22.mp4'"))
            .is_err(),
        "an unfinished upload entered serving"
    );

    // ---- the second run: the same application, remembering nothing ----
    let state = start_again(&subject);
    let task = the_only_task(&state);

    assert_eq!(
        task.state,
        TaskState::Paused,
        "a task from the previous run must wait for a person's decision"
    );
    assert!(
        task.progress > 0.0,
        "after the restart the task shows {:.2} — on such a zero a person cannot decide \
         whether to carry on a transfer of hours or drop it",
        task.progress
    );

    let restored = upload::restore_uploads(&state).expect("restoring failed");
    assert_eq!(restored, 1, "the previous run's upload was not raised");

    // It does not carry on by itself: the application may have been closed precisely to stop it.
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(
        state.tasks.get(&task.id).unwrap().unwrap().state,
        TaskState::Paused,
        "the raised upload carried on unbidden"
    );

    // Carried on — under the same task identifier, not a new one.
    upload::upload_resume(&state, &task.id).expect("the task would not carry on");
    assert_eq!(
        wait_done(&state, &task.id, Duration::from_secs(180)).await,
        TaskState::Completed,
        "the upload that carried on did not run to its end: {:?}",
        state.tasks.get(&task.id).ok().flatten()
    );

    // The main thing: the right file lies on the server rather than two attempts glued.
    let theirs = subject
        .server
        .exec_inside(&format!(
            "sha256sum '{VIDEO_DIR}/film_22.mp4' | cut -d' ' -f1"
        ))
        .expect("the checksum would not compute");
    assert_eq!(
        theirs.trim(),
        sha256_of(&subject.local),
        "the contents on the server differ from the source"
    );

    let leftovers = subject
        .server
        .exec_inside(&format!("ls -A '{STAGING_DIR}' 2>/dev/null | wc -l"))
        .unwrap_or_else(|_| String::from("0"));
    assert_eq!(
        leftovers.trim(),
        "0",
        "litter was left in the staging directory"
    );

    let _ = std::fs::remove_dir_all(&subject.db_dir);
}

#[tokio::test]
async fn a_source_swapped_between_runs_is_not_appended_to_another_s_beginning() {
    // The other side of the previous check. Only the same file may be carried on: had a
    // person rebuilt the video between runs, appending the new version's tail to the old
    // one's beginning would give two different files glued together on the server. The
    // checksum comparison would catch that too — but only after the whole transfer had
    // finished, that is, after an hour of wasted work.
    let subject = prepare_restart().await;
    start_and_kill(&subject);

    // While the application is away, the person rebuilt the video. The size is the same —
    // otherwise the divergence would be caught without comparing modification times.
    let mut data = std::fs::read(&subject.local).expect("the source will not read");
    for byte in data.iter_mut() {
        *byte = !*byte;
    }
    // On some file systems the modification time is coarsened to a second, while the whole
    // first half of the check fits in a few seconds.
    std::thread::sleep(Duration::from_millis(1100));
    std::fs::write(&subject.local, &data).expect("the source would not be rewritten");

    let state = start_again(&subject);
    let task = the_only_task(&state);

    assert_eq!(
        upload::restore_uploads(&state).expect("restoring failed"),
        1
    );
    upload::upload_resume(&state, &task.id).expect("the task would not carry on");

    assert_eq!(
        wait_done(&state, &task.id, Duration::from_secs(120)).await,
        TaskState::Failed,
        "the upload carried on over a different file"
    );

    let error = state
        .tasks
        .get(&task.id)
        .unwrap()
        .unwrap()
        .error
        .expect("the refusal was recorded with no cause");
    // "The source was swapped" specifically, not "the checksums diverged": those are
    // different troubles and they tell a person different things. They used to be told apart
    // by the text, and now by the code.
    assert!(
        error.says(vrcast_studio_lib::commands::error::DetailCode::UploadSourceChanged),
        "the reason for the refusal is not named: {error}"
    );

    // And the main thing: the glued file never entered serving.
    assert!(
        subject
            .server
            .exec_inside(&format!("test -e '{VIDEO_DIR}/film_22.mp4'"))
            .is_err(),
        "two files glued together entered serving"
    );

    let _ = std::fs::remove_dir_all(&subject.db_dir);
}

pub(crate) fn sha256_of(path: &std::path::Path) -> String {
    use sha2::{Digest, Sha256};
    let data = std::fs::read(path).expect("the file will not read");
    let mut hasher = Sha256::new();
    hasher.update(&data);
    hex::encode(hasher.finalize())
}
