//! T125 — scenario 2 from the quickstart, whole, against a throwaway server.
//!
//! The upload's separate properties are checked alongside, each on its own. Here they are
//! checked TOGETHER and at scale: a file of several gigabytes, five forced breaks in the
//! connection, and the application closing mid-transfer. Not one of those properties on its
//! own answers whether a real upload will survive to the end — and that is the question.
//!
//! Marked `ignore`: it runs for minutes and takes several gigabytes of disk. To run it:
//!
//! ```text
//! cargo test --features integration --test integration -- --ignored --nocapture upload_scenario
//! ```
//!
//! The live server takes no part here and cannot (constitution, the "Way of working"
//! section): checking a setup and breaks against somebody else's files will not do, and the
//! code does not change for it.

use super::fixture::TestServer;
use std::io::Write;
use std::time::{Duration, Instant};

/// How many gigabytes to send. Fewer and the transfer ends before it can be broken five
/// times; more and the check becomes impossibly long.
const GIGABYTES: usize = 2;

/// How many times to break the connection. As many as the quickstart scenario has.
const BREAKS: usize = 5;

/// Write a large file without holding it in memory.
///
/// The gigabytes are put together in pieces: gathering them into a vector would demand as
/// much memory as the file weighs, and the check would fail beside the point.
fn make_big_file(path: &std::path::Path, gigabytes: usize) {
    let mut file =
        std::io::BufWriter::new(std::fs::File::create(path).expect("could not create the file"));
    let mut chunk = vec![0u8; 4 * 1024 * 1024];
    let mut x: u32 = 0x1234_5678;
    for slot in chunk.chunks_mut(4) {
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        slot.copy_from_slice(&x.to_le_bytes());
    }
    for _ in 0..(gigabytes * 256) {
        file.write_all(&chunk).expect("could not write a piece");
    }
    file.flush().expect("could not finish writing the file");
}

/// Break every established connection without touching the listener.
///
/// Break, specifically, rather than stop the service: the application must reconnect by
/// itself, and killing the listener would leave it facing no server at all, which checks a
/// different property.
fn break_connections(server: &TestServer) {
    // The serving sshd processes are killed; the main one goes on accepting new ones.
    let _ = server.exec_inside("pkill -f 'sshd: root' || pkill -f 'sshd-session' || true");
}

fn staged_size(server: &TestServer, name: &str) -> u64 {
    server
        .exec_inside(&format!(
            "stat -c %s '/var/lib/vrcast/.vrcast-uploads/{name}.part' 2>/dev/null || echo 0"
        ))
        .unwrap_or_default()
        .trim()
        .parse()
        .unwrap_or(0)
}

#[tokio::test]
#[ignore = "the whole scenario: several gigabytes and minutes of work"]
async fn the_upload_scenario_survives_breaks_and_a_restart() {
    use std::sync::Arc;
    use vrcast_studio_lib::commands::upload::api as upload;
    use vrcast_studio_lib::commands::AppState;
    use vrcast_studio_lib::store::db::Db;
    use vrcast_studio_lib::store::secrets::{InMemorySecretStore, SecretStore};
    use vrcast_studio_lib::tasks::state::TaskState;

    const NAME: &str = "big_film.mp4";

    let server = TestServer::start().expect("the container would not come up");

    let dir = std::env::temp_dir().join(format!("vrcast-scn-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).expect("could not create the working directory");
    let local = dir.join(NAME);

    println!("preparing a file of {GIGABYTES} GB…");
    let started = Instant::now();
    make_big_file(&local, GIGABYTES);
    let size = std::fs::metadata(&local).unwrap().len();
    println!("the file is ready: {size} B in {:?}", started.elapsed());

    let db_dir = dir.join("db");
    let db_path = db_dir.join("vrcast.sqlite3");
    let secrets: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());

    let state = AppState::with_db(
        Arc::new(Db::open(&db_path).expect("the database would not open")),
        secrets.clone(),
    )
    .expect("the application state would not assemble");
    let id = super::upload_live::add_profile(&state, &server).await;

    let mut request = super::upload_live::request(&id, &local, NAME);
    // A speed limit — so the transfer takes a tangible time and can be broken five times.
    // Without one a local container swallows gigabytes faster than one can get at them.
    request.limit_bps = Some(60 * 1024 * 1024);

    let task = upload::upload_start(&state, request)
        .await
        .expect("the upload would not submit");
    println!("the upload has begun: {task}");

    // ---- five forced breaks ----
    for n in 1..=BREAKS {
        let before = wait_growth(&server, NAME, staged_size(&server, NAME), 120).await;
        break_connections(&server);
        println!("break {n}/{BREAKS} at {before} B");
        tokio::time::sleep(Duration::from_secs(2)).await;

        let record = state
            .tasks
            .get(&task)
            .ok()
            .flatten()
            .expect("the task vanished");
        assert!(
            !record.state.is_final(),
            "break {n} killed the task instead of a reconnect: {:?} / {:?}",
            record.state,
            record.error
        );
    }

    // ---- the application closing mid-transfer ----
    let before_close = staged_size(&server, NAME);
    assert!(before_close > 0, "nothing was sent by the time of closing");
    println!("closing the application at {before_close} B");

    // The application state is dropped whole: the task's living part dies with it, as in a
    // real closing. The database and the secrets stay — they outlive it.
    drop(state);
    tokio::time::sleep(Duration::from_secs(2)).await;

    let state = AppState::with_db(
        Arc::new(Db::open(&db_path).expect("the database would not open")),
        secrets.clone(),
    )
    .expect("the application state would not assemble");
    super::upload_live::attach_secret(&state);

    let restored = upload::restore_uploads(&state).expect("restoring failed");
    assert_eq!(restored, 1, "the previous run's upload was not raised");
    println!("after the restart the task waits for a decision");

    upload::upload_resume(&state, &task).expect("the task would not carry on");

    // ---- through to the end ----
    let deadline = Instant::now() + Duration::from_secs(900);
    loop {
        let record = state
            .tasks
            .get(&task)
            .ok()
            .flatten()
            .expect("the task vanished");
        if record.state.is_final() {
            assert_eq!(
                record.state,
                TaskState::Completed,
                "the upload did not run to its end: {:?}",
                record.error
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the upload did not end in the time allowed"
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    // ---- and the thing it was all for ----
    let theirs = server
        .exec_inside(&format!(
            "sha256sum '/var/lib/vrcast/videos/{NAME}' | cut -d' ' -f1"
        ))
        .expect("the checksum would not compute");
    let ours = super::upload_live::sha256_of(&local);
    assert_eq!(
        theirs.trim(),
        ours,
        "after five breaks and a restart the wrong file lies on the server"
    );

    let leftovers = server
        .exec_inside("ls -A '/var/lib/vrcast/.vrcast-uploads' 2>/dev/null | wc -l")
        .unwrap_or_else(|_| String::from("0"));
    assert_eq!(
        leftovers.trim(),
        "0",
        "litter was left in the staging directory"
    );

    println!("the scenario passed: {size} B, {BREAKS} breaks, one closing of the application");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Wait until there is more on the server than there was.
async fn wait_growth(server: &TestServer, name: &str, from: u64, seconds: u64) -> u64 {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        let now = staged_size(server, name);
        if now > from {
            return now;
        }
        assert!(
            Instant::now() < deadline,
            "not a byte beyond {from} was sent in {seconds} s"
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}
