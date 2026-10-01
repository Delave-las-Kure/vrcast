//! T678 — a quality set's prepared rung files belong to its medium: the catalogue records
//! them, deleting the medium takes them, the confirmation names them. What can be settled
//! without a server: the catalogue's shape on both sides of the change (what an older copy
//! wrote is read, what this one writes changes nothing for a medium without a set), which
//! files a set is credited with, and what deleting removes. The same against a real server:
//! `tests/integration/video_pipeline.rs` (built, deleted) and `ladder_attach.rs` (older sets).

use vrcast_studio_lib::domain::manifest::Manifest;
use vrcast_studio_lib::domain::media::{rename_plan, Media};
use vrcast_studio_lib::domain::set_files::{
    adopt, parse_records, record_built, records_script, rung_files_of_set, rung_mbits_in_master,
    sets_to_read, SetRecord,
};
use vrcast_studio_lib::server::listing::Entry;
use vrcast_studio_lib::server::reconcile::reconcile;

const MASTER_9_4: &str = "#EXTM3U\n#EXT-X-VERSION:3\n\
    #EXT-X-STREAM-INF:BANDWIDTH=9900000,RESOLUTION=1920x1080\nv9/stream.m3u8\n\
    #EXT-X-STREAM-INF:BANDWIDTH=4400000,RESOLUTION=1280x720\nv4/stream.m3u8\n";

fn medium(id: &str, slug: &str, files: &[&str], ladders: &[&str]) -> Media {
    let mut m = Media::new(id, slug, slug, "2026-10-02T00:00:00Z");
    m.files = files.iter().map(|s| (*s).to_owned()).collect();
    m.ladders = ladders.iter().map(|s| (*s).to_owned()).collect();
    m
}

fn catalogue(media: Vec<Media>) -> Manifest {
    Manifest {
        generation: 3,
        media,
        ..Manifest::empty()
    }
}

fn entry(name: &str, is_dir: bool) -> Entry {
    Entry {
        name: name.to_owned(),
        size_bytes: 10,
        is_dir,
    }
}

fn set_of(index: usize, slug: &str, master: &str, prepared: &str) -> SetRecord {
    SetRecord {
        media_index: index,
        slug: slug.to_owned(),
        master: master.to_owned(),
        prepared: prepared.to_owned(),
    }
}

// ---------- the catalogue on both sides of the change ----------

#[test]
fn a_catalogue_written_before_t678_is_read_with_no_set_files() {
    let text = r#"{ "generation": 7, "media": [
        { "id": "m1", "title": "Film", "slug": "film",
          "files": ["film.mp4"], "ladders": ["film/master.m3u8"],
          "created_at": "2026-08-01T10:00:00Z" } ] }"#;
    let m = Manifest::parse(text).expect("an older catalogue does not read");
    assert!(m.media[0].set_files.is_empty());
    assert!(m.media[0].extra.is_empty(), "{:?}", m.media[0].extra);
}

#[test]
fn a_medium_without_set_files_is_written_exactly_as_before() {
    // What an older copy of the application reads must not grow for media that have none.
    let m = catalogue(vec![medium("m1", "film", &["film.mp4"], &[])]);
    let json = m.to_json();
    assert!(!json.contains("set_files"), "{json}");
}

#[test]
fn set_files_are_written_as_a_list_of_names_and_read_back() {
    let mut m = catalogue(vec![medium("m1", "film", &[], &["film/master.m3u8"])]);
    m.media[0].set_files = vec!["film_4.mp4".into(), "film_9.mp4".into()];
    let json = m.to_json();
    assert!(json.contains("\"set_files\""), "{json}");
    let back = Manifest::parse(&json).unwrap();
    assert_eq!(back, m);
}

#[test]
fn an_older_copy_keeps_set_files_it_does_not_know_when_it_rewrites_the_catalogue() {
    // What a copy from before T678 does with the field: an unknown field of a medium lands in
    // its `extra` and is written back as it was (FR-131). Modelled here by reading the field
    // into a structure that has no such field — `extra` is exactly that mechanism.
    #[derive(serde::Deserialize, serde::Serialize)]
    struct OldMedia {
        id: String,
        #[serde(flatten)]
        extra: std::collections::HashMap<String, serde_json::Value>,
    }
    let text = r#"{ "id": "m1", "set_files": ["film_9.mp4"] }"#;
    let old: OldMedia = serde_json::from_str(text).unwrap();
    let again = serde_json::to_string(&old).unwrap();
    assert!(again.contains("\"set_files\":[\"film_9.mp4\"]"), "{again}");
}

// ---------- which files a set is credited with ----------

#[test]
fn the_master_names_the_rungs_it_serves() {
    assert_eq!(rung_mbits_in_master(MASTER_9_4), vec![4, 9]);
    assert!(rung_mbits_in_master("").is_empty());
}

#[test]
fn a_rung_file_is_the_set_s_only_by_the_set_s_own_word() {
    let candidates = [
        "film_9.mp4",
        "film_4v.mp4",
        "film_7.mp4",  // named like a rung — the set serves no 7
        "film_9x.mp4", // not a rung's name at all
        "other_9.mp4", // another medium's name
        "film.mp4",
    ];
    assert_eq!(
        rung_files_of_set("film", MASTER_9_4, "", &candidates),
        vec!["film_4v.mp4", "film_9.mp4"]
    );
    // With no master and no record, nothing — a name alone is never enough.
    assert!(rung_files_of_set("film", "", "", &candidates).is_empty());
    // The record names a file even when the master is unreadable.
    assert_eq!(
        rung_files_of_set(
            "film",
            "",
            "v9=film_9v.mp4\n",
            &["film_9v.mp4", "film_9.mp4"]
        ),
        vec!["film_9v.mp4"]
    );
}

#[test]
fn an_older_set_s_rung_files_are_attributed_on_reading_and_leave_the_unrecognised() {
    let m = catalogue(vec![
        medium("m1", "film", &[], &["film/master.m3u8"]),
        medium("m2", "other", &["other_9.mp4"], &[]),
    ]);
    let entries = vec![
        entry("film", true),
        entry("film_9.mp4", false),
        entry("film_4.mp4", false),
        entry("film_7.mp4", false),
        entry("other_9.mp4", false),
    ];
    let files: Vec<&str> = ["film_9.mp4", "film_4.mp4", "film_7.mp4", "other_9.mp4"].to_vec();
    let seen = adopt(&m, &[set_of(0, "film", MASTER_9_4, "")], &files);
    assert_eq!(seen.media[0].set_files, vec!["film_4.mp4", "film_9.mp4"]);
    assert!(seen.media[1].set_files.is_empty());
    assert_eq!(
        seen.generation, m.generation,
        "reading must not claim a write"
    );

    let r = reconcile(&seen, &entries);
    let loose: Vec<&str> = r.unrecognized.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(loose, vec!["film_7.mp4"]);
    assert_eq!(r.media_files[0].set_files.len(), 2);
}

#[test]
fn a_medium_s_single_file_named_like_a_rung_stays_its_single_file() {
    // T675: `film_9.mp4` is the medium's own file; the set's 9 Mbit/s rung is `film_9v.mp4`.
    let mut m = catalogue(vec![medium(
        "m1",
        "film",
        &["film_9.mp4"],
        &["film/master.m3u8"],
    )]);
    // Even a stale record of it as a set file does not make it one.
    m.media[0].set_files = vec!["film_9.mp4".into()];
    let files = ["film_9.mp4", "film_9v.mp4", "film_4.mp4"];
    let seen = adopt(
        &m,
        &[set_of(
            0,
            "film",
            MASTER_9_4,
            "v9=film_9v.mp4\nv4=film_4.mp4\n",
        )],
        &files,
    );
    assert_eq!(seen.media[0].files, vec!["film_9.mp4"]);
    assert_eq!(seen.media[0].set_files, vec!["film_4.mp4", "film_9v.mp4"]);
}

#[test]
fn a_recorded_set_file_that_is_gone_or_named_otherwise_is_dropped_from_the_view() {
    let mut m = catalogue(vec![medium("m1", "film", &[], &["film/master.m3u8"])]);
    m.media[0].set_files = vec![
        "film_9.mp4".into(), // gone from the server
        "old_4.mp4".into(),  // a record across a rename an older copy made
        "film_4.mp4".into(),
    ];
    let seen = adopt(&m, &[], &["film_4.mp4", "old_4.mp4"]);
    assert_eq!(seen.media[0].set_files, vec!["film_4.mp4"]);
}

#[test]
fn only_sets_under_their_medium_s_own_name_and_on_the_server_are_read() {
    let m = catalogue(vec![
        medium("m1", "film", &[], &["film/master.m3u8"]),
        medium("m2", "other", &[], &["elsewhere/master.m3u8"]),
        medium("m3", "gone", &[], &["gone/master.m3u8"]),
        medium("m4", "plain", &["plain.mp4"], &[]),
    ]);
    let dirs = ["film", "elsewhere"];
    assert_eq!(sets_to_read(&m, &dirs, None), vec![(0, "film".to_owned())]);
    assert!(sets_to_read(&m, &dirs, Some(1)).is_empty());
}

#[test]
fn the_records_script_is_read_back_set_by_set() {
    let slugs = vec!["a".to_owned(), "b".to_owned()];
    let script = records_script("/v", &slugs, |s| format!("'{s}'"));
    assert!(script.contains("'/v/a/master.m3u8'") && script.contains("'/v/b/.prepared'"));
    let mark = "--vrcast-set-record--";
    let out = format!("{mark}\nMA\n\n{mark}\nv9=a_9v.mp4\n\n{mark}\n\n{mark}\n\n");
    let parsed = parse_records(&out, 2);
    assert_eq!(parsed[0].0.trim(), "MA");
    assert_eq!(parsed[0].1.trim(), "v9=a_9v.mp4");
    assert_eq!(parsed[1], ("\n".to_owned(), "\n".to_owned()));
    // An answer cut short attributes nothing rather than misreading.
    assert_eq!(parse_records("", 1), vec![(String::new(), String::new())]);
}

// ---------- after a build ----------

#[test]
fn a_build_records_its_rung_files_under_the_medium() {
    let mut next = catalogue(vec![
        medium("m1", "film", &["film_9.mp4"], &["film/master.m3u8"]),
        medium("m2", "other", &[], &[]),
    ]);
    let built = vec!["film_9v.mp4".to_owned(), "film_4.mp4".to_owned()];
    assert!(record_built(&mut next, "film", &built));
    assert_eq!(next.media[0].set_files, vec!["film_4.mp4", "film_9v.mp4"]);
    // A file the build took whole from the medium's own (T675) stays a single file.
    let mut again = next.clone();
    assert!(!record_built(
        &mut again,
        "film",
        &["film_9.mp4".to_owned()]
    ));
    assert_eq!(again.media[0].files, vec!["film_9.mp4"]);
    // Nothing new — nothing to write.
    assert!(!record_built(&mut again, "film", &built));
    // No medium of that name — nothing.
    assert!(!record_built(&mut again, "nobody", &built));
}

// ---------- deleting and renaming ----------

#[test]
fn deleting_a_medium_removes_its_set_files_and_nothing_of_another_medium() {
    let mut m = medium("m1", "film", &["film.mp4"], &["film/master.m3u8"]);
    m.set_files = vec!["film_9.mp4".into(), "film_4v.mp4".into()];
    let paths: Vec<&str> = m.all_paths().map(String::as_str).collect();
    assert_eq!(
        paths,
        vec!["film.mp4", "film/master.m3u8", "film_9.mp4", "film_4v.mp4"]
    );
}

#[test]
fn a_set_file_filed_by_hand_becomes_what_it_is_filed_as() {
    let mut m = medium("m1", "film", &[], &["film/master.m3u8"]);
    m.set_files = vec!["film_9.mp4".into()];
    let cat = catalogue(vec![m, medium("m2", "other", &[], &[])]);
    let next = cat.with_file_under("m2", "film_9.mp4", false).unwrap();
    assert!(next.media[0].set_files.is_empty());
    assert_eq!(next.media[1].files, vec!["film_9.mp4"]);
}

#[test]
fn renaming_a_medium_moves_its_set_files_with_the_short_name() {
    let mut m = medium("m1", "film", &[], &["film/master.m3u8"]);
    m.set_files = vec!["film_9v.mp4".into()];
    let plan = rename_plan(&m, "film", "fresh");
    assert_eq!(plan.set_files, vec!["fresh_9v.mp4"]);
    assert!(plan
        .renames
        .contains(&("film_9v.mp4".to_owned(), "fresh_9v.mp4".to_owned())));
}
