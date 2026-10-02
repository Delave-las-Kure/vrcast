//! T682 (QA-25 №3) — a stop pressed during the server's cutting, and the application killed
//! before the server confirmed it: after the next start the cutting on the server is stopped
//! for real before the video reads «cancelled», and nothing of another copy's cutting is
//! touched.
//!
//! **The application is killed by dropping the cutting's future** in the middle of its
//! watch: nothing of the stop runs, exactly as when the process dies — the cutting on the
//! server is a process group of its own and goes on (T605). What the killed run left in the
//! database is what `Cutting::run` itself wrote: the note of its start (`remote_runs`).

use std::sync::Arc;
use std::time::{Duration, Instant};

use vrcast_studio_lib::commands::video::api as video;
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::domain::hls_package::ToCut;
use vrcast_studio_lib::domain::video::{VideoStage, VideoState};
use vrcast_studio_lib::server::hls_package::Cutting;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::remote_runs;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;
use vrcast_studio_lib::store::videos::{self as rows, VideoRow};
use vrcast_studio_lib::tasks::engine::TaskContext;
use vrcast_studio_lib::tasks::state::{TaskKind, TaskState};
use vrcast_studio_lib::tasks::store::{self as tasks, Batch, TaskRecord};

use super::fixture::TestServer;
use super::hls_cutting_cancel::make_slow_film;
use super::hls_fixture::VIDEO_DIR;
use super::ssh_live::connect;
use super::upload_live::add_profile;

async fn running(cutting: &Cutting<'_>, want: bool, limit: Duration) {
    let deadline = Instant::now() + limit;
    loop {
        if cutting.still_running().await.unwrap_or(!want) == want {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the cutting never became running={want}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cutting_left_stopping_by_a_killed_application_is_stopped_after_the_next_start() {
    let server = TestServer::start().expect("the container would not come up");
    make_slow_film(&server, "stop_restart.mp4").expect("the fixture film would not encode");

    let db = Arc::new(Db::open_in_memory().unwrap());
    let secrets = Arc::new(InMemorySecretStore::new());
    let first = AppState::with_db(db.clone(), secrets.clone()).unwrap();
    let server_id = add_profile(&first, &server).await;

    // The video and its build, as the run that is about to be killed has them: «Cancel»
    // pressed during the cutting. `TaskContext::detached` reports as task "detached".
    let slug = "stoprestart";
    let mut row = VideoRow::new("v_stop", &server_id, "C:/nowhere/film.mp4", "Film", slug);
    row.state = VideoState::Cancelling;
    row.stage = VideoStage::Cutting;
    row.task_id = Some(String::from("detached"));
    rows::save(&db, &row).unwrap();
    let mut t = TaskRecord::new("detached", TaskKind::BuildLadder, Some(server_id.clone()));
    t.state = TaskState::Running;
    t.batch = Some(Batch {
        id: row.id.clone(),
        label: row.title.clone(),
    });
    t.resume_token = Some(slug.to_owned());
    tasks::upsert(&db, &t).unwrap();

    let conn = connect(&server).await;
    let variants = vec![ToCut {
        sub: String::from("v1"),
        file: String::from("stop_restart.mp4"),
    }];
    let cutting = Cutting {
        conn: &conn,
        video_dir: VIDEO_DIR,
        owner: "root:root",
        base: slug,
        variants: &variants,
    };
    let ctx = TaskContext::detached(db.clone());
    // The application dies three seconds into the cutting: the future is dropped mid-watch.
    let cut = tokio::time::timeout(Duration::from_secs(3), cutting.run(&ctx, |_| {})).await;
    assert!(cut.is_err(), "the slow cutting finished before the kill");
    running(&cutting, true, Duration::from_secs(5)).await;
    let noted = remote_runs::get(&db, "detached")
        .unwrap()
        .expect("the cutting's start was not written down");
    assert!(noted.mark.starts_with(&format!("{slug}:")));

    // Another copy of the application cutting another set on the same server.
    let other_variants = vec![ToCut {
        sub: String::from("v1"),
        file: String::from("stop_restart.mp4"),
    }];
    let other = Cutting {
        conn: &conn,
        video_dir: VIDEO_DIR,
        owner: "root:root",
        base: "othercopy",
        variants: &other_variants,
    };
    let other_started = other.start().await.expect("the other copy would not start");
    running(&other, true, Duration::from_secs(5)).await;

    // The killed run's build is a row that start-up finds `paused` (`recover_after_start`).
    tasks::save_state(&db, "detached", TaskState::Paused, None).unwrap();

    // The next start.
    let next = AppState::with_db(db.clone(), secrets.clone()).unwrap();
    video::restore_videos(&next).unwrap();
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let v = video::video_get(&next, &row.id).unwrap();
        if v.state == VideoState::Cancelled {
            break;
        }
        assert_eq!(v.state, VideoState::Cancelling, "{v:?}");
        // Until it is cancelled the build's row stays unfinished: the set is busy.
        assert!(!tasks::get(&db, "detached")
            .unwrap()
            .unwrap()
            .state
            .is_final());
        assert!(Instant::now() < deadline, "the stop was never confirmed");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    // «Cancelled» only once nothing of that cutting is alive on the server.
    assert!(
        !cutting
            .still_running()
            .await
            .expect("the server would not say"),
        "the video reads cancelled and its cutting is still running"
    );
    assert_eq!(
        tasks::get(&db, "detached").unwrap().unwrap().state,
        TaskState::Cancelled
    );
    assert!(remote_runs::get(&db, "detached").unwrap().is_none());

    // The other copy's cutting was not touched.
    assert!(
        other
            .still_running()
            .await
            .expect("the server would not say"),
        "the stop reached another copy's cutting"
    );
    other
        .stop(&other_started)
        .await
        .expect("the other copy's cutting would not stop");
    conn.close().await;
}
