//! T653 — dropping an upload raised after a restart tidies up after it, as an action of its
//! own (QA-24A №4, FR-038, quickstart "Scenario 2" item 7).
//!
//! Only what shows without a server: the task is dropped without its transfer ever being
//! started, and with its server out of reach the duty to remove the part-file is kept on the
//! task — where a person reads it — rather than forgotten. The removal itself, against a real
//! server, is `integration/upload_live.rs::a_raised_upload_dropped_without_carrying_on_leaves_no_part_file`.

use super::support::{state, valid_input};
use std::time::Duration;
use vrcast_studio_lib::commands::error::DetailCode;
use vrcast_studio_lib::commands::servers::api as servers;
use vrcast_studio_lib::commands::upload::api::{
    self as upload, leftover_steps, settled_notices, LeftoverStep,
};
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::domain::transfer::ResumeToken;
use vrcast_studio_lib::domain::wording::Detail;
use vrcast_studio_lib::store::profiles;
use vrcast_studio_lib::tasks::state::{TaskKind, TaskState};
use vrcast_studio_lib::tasks::store::{self, TaskRecord};

fn token(name: &str) -> ResumeToken {
    ResumeToken {
        remote_temp: format!("/var/lib/vrcast/.vrcast-uploads/{name}.part"),
        remote_name: name.to_owned(),
        local_path: Some(String::from("C:/films/film.mp4")),
        media_id: None,
        source_size: 1_000,
        source_modified: None,
    }
}

/// A server nobody answers on: a confirmed fingerprint (so the attempt really goes to the
/// network), and a port on this machine nothing listens on.
fn unreachable_server(state: &AppState) -> String {
    let mut input = valid_input("Unreachable");
    input.host = String::from("127.0.0.1");
    input.port = 1;
    let id = servers::server_add(state, input, "password").expect("the profile would not set up");
    profiles::set_fingerprint(&state.db, &id, "SHA256:not-a-real-fingerprint").unwrap();
    id
}

/// An upload the previous run left paused, with its resume position.
fn left_from_previous_run(state: &AppState, id: &str, server: &str, name: &str) {
    let mut rec = TaskRecord::new(id, TaskKind::Upload, Some(server.to_owned()));
    rec.state = TaskState::Paused;
    rec.resume_token = Some(token(name).to_json());
    store::upsert(&state.db, &rec).unwrap();
}

async fn until_final(state: &AppState, id: &str) -> TaskRecord {
    tokio::time::timeout(Duration::from_secs(40), async {
        loop {
            let t = state.tasks.get(id).unwrap().unwrap();
            if t.state.is_final() {
                return t;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the dropped upload never ended")
}

#[tokio::test]
async fn a_raised_upload_dropped_with_its_server_out_of_reach_keeps_the_duty_and_says_so() {
    let state = state();
    let server = unreachable_server(&state);
    left_from_previous_run(&state, "old-upload", &server, "film_22.mp4");

    assert_eq!(upload::restore_uploads(&state).unwrap(), 1);
    state.tasks.cancel("old-upload").unwrap();
    let done = until_final(&state, "old-upload").await;

    assert_eq!(done.state, TaskState::Cancelled);
    let pending: Vec<&Detail> = done
        .notices
        .iter()
        .filter(|n| n.key == DetailCode::NoticeLeftoverPending)
        .collect();
    assert_eq!(
        pending.len(),
        1,
        "the part-file's removal was forgotten rather than kept: {:?}",
        done.notices
    );
    assert_eq!(pending[0].params["name"], "film_22.mp4");

    // And it is found again by what does the duty later.
    let owed = store::carrying_notice(
        &state.db,
        TaskKind::Upload,
        DetailCode::NoticeLeftoverPending,
    )
    .unwrap();
    assert_eq!(owed.len(), 1);
    assert_eq!(
        leftover_steps(&owed, &server),
        vec![LeftoverStep::Remove {
            task_id: String::from("old-upload"),
            remote_temp: token("film_22.mp4").remote_temp,
            name: String::from("film_22.mp4"),
        }]
    );
}

#[tokio::test]
async fn a_dropped_upload_does_not_remove_a_part_file_another_upload_is_writing() {
    // Two uploads under one name share one part-file (`remote_name::staging_file`). Dropping
    // the old one must not throw away the new one's work — and there is then nothing owed.
    let state = state();
    let server = unreachable_server(&state);
    left_from_previous_run(&state, "old-upload", &server, "film_22.mp4");
    let mut newer = TaskRecord::new("newer-upload", TaskKind::Upload, Some(server.clone()));
    newer.state = TaskState::Paused;
    newer.resume_token = Some(token("film_22.mp4").to_json());
    store::upsert(&state.db, &newer).unwrap();

    upload::restore_uploads(&state).unwrap();
    state.tasks.cancel("old-upload").unwrap();
    let done = until_final(&state, "old-upload").await;

    assert_eq!(done.state, TaskState::Cancelled);
    assert!(
        done.notices.is_empty(),
        "a duty was kept over a file another upload owns: {:?}",
        done.notices
    );
    state.tasks.cancel("newer-upload").unwrap();
}

#[test]
fn what_is_owed_is_worked_out_per_server_and_lapses_to_a_newer_upload() {
    let pending = |id: &str, server: &str, name: &str| {
        let mut t = TaskRecord::new(id, TaskKind::Upload, Some(server.to_owned()));
        t.state = TaskState::Cancelled;
        t.resume_token = Some(token(name).to_json());
        t.notices = vec![Detail::new(DetailCode::NoticeLeftoverPending).with("name", name)];
        t
    };
    let mut writing = TaskRecord::new("live", TaskKind::Upload, Some(String::from("s1")));
    writing.state = TaskState::Running;
    writing.resume_token = Some(token("b.mp4").to_json());

    let tasks = vec![
        pending("t-a", "s1", "a.mp4"),
        pending("t-b", "s1", "b.mp4"),
        pending("t-c", "s2", "c.mp4"),
        writing,
    ];
    assert_eq!(
        leftover_steps(&tasks, "s1"),
        vec![
            LeftoverStep::Remove {
                task_id: String::from("t-a"),
                remote_temp: token("a.mp4").remote_temp,
                name: String::from("a.mp4"),
            },
            LeftoverStep::Lapse {
                task_id: String::from("t-b")
            },
        ]
    );

    // Settled: the pending notice goes, "removed later" takes its place; others stay.
    let before = vec![
        Detail::new(DetailCode::NoticeCancelledAfterPublish).with("name", "a.mp4"),
        Detail::new(DetailCode::NoticeLeftoverPending).with("name", "a.mp4"),
    ];
    let after = settled_notices(&before, Some("a.mp4"));
    let keys: Vec<DetailCode> = after.iter().map(|n| n.key).collect();
    assert_eq!(
        keys,
        vec![
            DetailCode::NoticeCancelledAfterPublish,
            DetailCode::NoticeLeftoverRemoved
        ]
    );
    assert!(settled_notices(&before[1..], None).is_empty());
}
