//! T686 (QA-25 №7) — «Replace» killed between removing the old set and building the new one
//! carries on after the next start, against a real server.
//!
//! **What a killed run leaves is made by hand**: the note `video_replace` writes before it
//! removes anything (phase `deleting`, the medium), and the old set half removed — its
//! directory gone, its prepared rungs still there — which is where a run killed in the
//! middle of the removal stands. The next start must remove the rest (the same guarded
//! removal, so a repeat is safe), build the second film whole into the same medium, and the
//! library must read the medium's set as building meanwhile, never as missing.

use std::sync::Arc;
use std::time::Duration;

use vrcast_studio_lib::commands::video::api as video;
use vrcast_studio_lib::domain::video::{SetWorkState, VideoState};
use vrcast_studio_lib::store::secrets::{InMemorySecretStore, SecretStore};
use vrcast_studio_lib::store::videos as rows;

use super::fixture::TestServer;
use super::upload_live::add_profile;
use super::video_pipeline::{
    build_one, make_film, make_film_from, no_prepared_files, origin_of, state_on, the_set,
    the_set_is_served, until, Scratch, VIDEO_DIR,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_replace_killed_between_removing_and_building_carries_on_after_a_restart() {
    super::fixture::logging_if_requested();
    let server = TestServer::start().expect("the container would not come up");
    let scratch = Scratch::new("t686-replace");
    let db = scratch.0.join("vrcast.sqlite3");
    let secrets: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let state = state_on(&db, secrets.clone(), &origin_of(&server));
    let server_id = add_profile(&state, &server).await;

    std::fs::create_dir_all(scratch.0.join("a")).unwrap();
    std::fs::create_dir_all(scratch.0.join("b")).unwrap();
    let first = scratch.0.join("a").join("Replaced.mp4");
    let second = scratch.0.join("b").join("Replaced.mp4");
    make_film(&first, "1280x720", 12);
    make_film_from(&second, "testsrc=size=1280x720:rate=24", 12);

    let done = build_one(&state, &server_id, &first).await;
    assert_eq!(done.state, VideoState::Done, "{:?}", done.problem);
    let slug = done.slug.clone();
    let medium = done.media_id.clone().expect("the first set has no medium");
    let before = the_set(&server, &slug);

    let stopped = build_one(&state, &server_id, &second).await;
    assert_eq!(stopped.state, VideoState::Problem);

    // «Replace» confirmed, and the run killed half way through the removal: the note is
    // there, the set's directory is gone, its prepared rungs are not. (A set checked since
    // T693 keeps none; one built before it does — they are laid here by hand.)
    let mut row = rows::get(&state.db, &stopped.id).unwrap().unwrap();
    row.replacing_json = Some(
        serde_json::json!({
            "phase": "deleting",
            "into_medium": false,
            "confirmed": false,
            "media_id": medium,
        })
        .to_string(),
    );
    rows::save(&state.db, &row).unwrap();
    server
        .exec_inside(&format!(
            "find '{VIDEO_DIR}/{slug}' -depth -delete && test ! -e '{VIDEO_DIR}/{slug}' && \
             head -c 4000 /dev/urandom > '{VIDEO_DIR}/{slug}_2.mp4' && \
             head -c 4000 /dev/urandom > '{VIDEO_DIR}/{slug}_1.mp4' && echo gone"
        ))
        .expect("the directory would not go");
    drop(state);

    // The next start.
    let next = state_on(&db, secrets.clone(), &origin_of(&server));
    // Building from the moment it was confirmed — never «not on the server».
    let view = vrcast_studio_lib::commands::library::api::library_list_known(&next, &server_id)
        .await
        .expect("the library would not read");
    let m = view
        .media
        .iter()
        .find(|m| m.id == medium)
        .expect("the medium is not in the library");
    assert_eq!(
        m.set_work.as_ref().map(|w| w.state),
        Some(SetWorkState::Building),
        "the medium's set reads as missing while it is being replaced"
    );

    video::restore_videos(&next).unwrap();
    // It starts on the problem it is replacing; the replace is over when its note is.
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    while rows::get(&next.db, &stopped.id)
        .unwrap()
        .unwrap()
        .replacing_json
        .is_some()
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the replace was never carried through"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let replaced = until(
        &next,
        &stopped.id,
        "the replace carried on",
        Duration::from_secs(400),
        |v| matches!(v.state, VideoState::Done | VideoState::Problem),
    )
    .await;
    assert_eq!(replaced.state, VideoState::Done, "{:?}", replaced.problem);
    assert_eq!(replaced.media_id.as_deref(), Some(medium.as_str()));
    assert!(rows::get(&next.db, &stopped.id)
        .unwrap()
        .unwrap()
        .replacing_json
        .is_none());
    the_set_is_served(&server, &slug);
    // The old prepared rungs went with the rest, and the new set keeps none (T693).
    no_prepared_files(&server, &slug);

    // Every rung is the second film's: nothing of the first was taken for done.
    let after = the_set(&server, &slug);
    for (name, old) in &before {
        if let Some((_, new)) = after.iter().find(|(n, _)| n == name) {
            if name.ends_with(".ts") || name.ends_with(".mp4") {
                assert_ne!(new, old, "{name} is still the first film's");
            }
        }
    }
    drop(scratch);
}
