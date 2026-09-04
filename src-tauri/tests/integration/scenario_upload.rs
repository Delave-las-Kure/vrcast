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
///
/// **Two by default and thirty on request** (T474). SC-003 names thirty gigabytes, and the
/// figure is not decoration: at that size a break lands in the middle of a transfer that has
/// already been running for a quarter of an hour, and what is resumed from is an offset no
/// short run ever reaches. Thirty every time would make this untouchable, so the ordinary run
/// stays at two and the acceptance run asks for the rest:
///
/// ```text
/// VRCAST_UPLOAD_GB=30 cargo test --features integration --test integration ///   -- --ignored --nocapture upload_scenario
/// ```
fn gigabytes() -> usize {
    std::env::var("VRCAST_UPLOAD_GB")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2)
}

/// How many times to break the connection. As many as the quickstart scenario has.
const BREAKS: usize = 5;

/// Wait until the staged file has passed a mark, saying what the task thinks it is doing.
///
/// **Where the break lands is the point.** A break at a tenth of a percent proves nothing
/// about resuming from two thirds of the way through, and the waiter this replaced only asked
/// whether anything had moved at all.
async fn wait_until_past_watching(
    server: &TestServer,
    name: &str,
    mark: u64,
    seconds: u64,
    watching: Option<(&vrcast_studio_lib::commands::AppState, &str)>,
) -> u64 {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut furthest = staged_size(server, name);
    let mut moved_at = Instant::now();
    loop {
        let at = staged_size(server, name);
        if at >= mark {
            return at;
        }
        if at > furthest {
            furthest = at;
            moved_at = Instant::now();
        }
        // ⚠ **A transfer that has stopped is a failure, not a wait.** This used to give back
        // whatever it had reached when its time ran out, and the run carried on: the next
        // break fired at the very same offset, and the one after that, and five breaks were
        // reported against a transfer that had not moved a byte since the first. Three hours
        // of a run said "five breaks survived" about nothing at all.
        if let Some((state, task)) = watching {
            if let Ok(Some(rec)) = state.tasks.get(task) {
                println!(
                    "    at {at} B — task {:?} {:.1}% stage {:?} err {:?}",
                    rec.state,
                    rec.progress * 100.0,
                    rec.stage,
                    rec.error.as_ref().map(|e| e.code)
                );
            }
        }
        assert!(
            moved_at.elapsed() < Duration::from_secs(180),
            "the transfer has not moved past {furthest} bytes for three minutes while waiting              for {mark}. It is stopped, not slow — and a check that waits it out would report              breaks it never made"
        );
        assert!(
            Instant::now() < deadline,
            "the transfer did not reach {mark} bytes within {seconds}s; it got to {furthest}"
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

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

    // ⚠ **Where the file goes is told, not assumed, and the room is checked before a byte is
    // written.** The temporary directory is on the system drive, and a thirty-gigabyte run put
    // there filled it — 931 GB down to 2.1 free, with three abandoned copies left behind by
    // runs that were interrupted. Writing until the disk stops you is the worst way to find
    // out you had no room: it takes the machine with it. The application refuses a build it
    // cannot fit (FR-149); a check of the application has no business behaving worse.
    let root = std::env::var("VRCAST_UPLOAD_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    let dir = root.join(format!("vrcast-scn-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).expect("could not create the working directory");
    let local = dir.join(NAME);

    let gigabytes = gigabytes();
    let need = gigabytes as u64 * 1024 * 1024 * 1024;
    let free = vrcast_studio_lib::media::local_disk::usage(&dir)
        .map(|u| u.free_bytes)
        .unwrap_or(u64::MAX);
    assert!(
        free > need + need / 10,
        "this needs {need} bytes for the film and there are {free} free under {}. Point \
         VRCAST_UPLOAD_DIR at a drive with room — filling the disk instead would take the \
         whole machine down with it",
        dir.display()
    );
    println!("preparing a file of {gigabytes} GB…");
    let started = Instant::now();
    make_big_file(&local, gigabytes);
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

    // ---- five forced breaks, spread along the transfer ----
    //
    // ⚠ **Spread by progress, not by the clock.** They used to be made one after another with
    // two seconds between, which on a two-gigabyte file lands them across a fair part of it —
    // and on thirty gigabytes puts all five inside the first tenth of a percent (measured:
    // breaks at 7, 15, 19, 27 and 31 MB of 32 GB, and then twenty-nine gigabytes carried
    // through untouched). That is not what SC-003 asks. The interesting break is the late one:
    // resuming from an offset no short run ever reaches.
    let total = std::fs::metadata(&local).map(|m| m.len()).unwrap_or(0);
    for n in 1..=BREAKS {
        let want = total * (n as u64) * 15 / 100;
        let before =
            wait_until_past_watching(&server, NAME, want, 3600, Some((&state, &task))).await;
        break_connections(&server);
        println!(
            "break {n}/{BREAKS} at {before} B ({:.1}% of {total})",
            100.0 * before as f64 / total.max(1) as f64
        );
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
    // Fifteen minutes for the ordinary two gigabytes, and room for the rest in proportion:
    // a thirty-gigabyte run is an acceptance run and takes as long as it takes.
    let deadline = Instant::now() + Duration::from_secs(900 + gigabytes as u64 * 600);
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

#[tokio::test]
#[ignore = "kills sessions on a container: run by hand"]
async fn a_command_in_flight_does_not_outlive_the_connection_it_runs_on() {
    // ⚠ **The same question as T483, asked of the other half.** The write to the server had no
    // bound on it and a broken connection froze the transfer for ever; `exec` is the path
    // everything else takes — the health readings, the deploy steps, the following of viewers,
    // the listing of the library. If it hangs the same way, every one of those hangs with it,
    // and a hang is worse than a failure because nothing says it happened.
    //
    // Evidence before a fix: this asks whether it hangs, and says which.
    use vrcast_studio_lib::ssh::{fingerprint, Connection, Credentials, ServerAddress};

    let server = TestServer::start().expect("the container would not come up");
    let address = ServerAddress::new(server.host(), server.port);
    let fp = fingerprint::probe(&address)
        .await
        .expect("the fingerprint was not obtained");
    let conn = Connection::connect(
        address,
        "root",
        Credentials::Key {
            path: super::fixture::key_path(),
            passphrase: Some(super::fixture::KEY_PASSPHRASE.to_owned()),
        },
        &fp,
    )
    .await
    .expect("connecting failed");

    let running = tokio::spawn({
        let conn = conn.clone();
        async move { conn.exec("sleep 600; echo done").await }
    });
    tokio::time::sleep(Duration::from_secs(5)).await;
    break_connections(&server);

    // The keepalives are ninety seconds; three minutes is twice that and then some.
    let outcome = tokio::time::timeout(Duration::from_secs(180), running).await;
    match outcome {
        Ok(Ok(Ok(out))) => println!("the command came back on its own: ok={}", out.ok()),
        Ok(Ok(Err(e))) => println!("the command came back as an error, which is right: {e}"),
        Ok(Err(e)) => panic!("the task running the command panicked: {e}"),
        Err(_) => panic!(
            "a command still in flight outlived the connection it ran on by three minutes. \
             Everything that talks to a server goes through here, so everything that talks to \
             a server can hang the same way — and a hang says nothing at all"
        ),
    }
}
