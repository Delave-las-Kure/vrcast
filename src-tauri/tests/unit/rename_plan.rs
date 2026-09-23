//! T606 — `media::rename_plan` works out BOTH sides of every `mv` a short-name change will
//! run, before any of them runs.
//!
//! `media_rename`'s busy-guard (T599) used to see only the old names: the new ones were
//! computed inside the `mv` loop, after the guard. These tests pin down what the plan says
//! the destinations are, since that is exactly the set the guard now checks against running
//! uploads and builds.

use vrcast_studio_lib::domain::media::{rename_plan, Media};

fn medium(slug: &str, files: &[&str], ladders: &[&str]) -> Media {
    let mut m = Media::new("m_1", slug, slug, "2026-09-23T10:00:00Z");
    m.files = files.iter().map(|s| (*s).to_owned()).collect();
    m.ladders = ladders.iter().map(|s| (*s).to_owned()).collect();
    m
}

fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
    v.iter()
        .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
        .collect()
}

#[test]
fn the_audit_scenario_names_the_destination_an_upload_would_write() {
    // `film` holds `film_9.mp4`; renaming it `fresh` moves it onto `fresh_9.mp4` — the very
    // name a running upload of a new file would enter serving under.
    let m = medium("film", &["film_9.mp4"], &[]);
    let plan = rename_plan(&m, "film", "fresh");

    assert_eq!(plan.renames, pairs(&[("film_9.mp4", "fresh_9.mp4")]));
    assert_eq!(
        plan.targets().cloned().collect::<Vec<_>>(),
        vec![String::from("fresh_9.mp4")]
    );
    assert_eq!(plan.files, vec![String::from("fresh_9.mp4")]);
    assert!(plan.ladders.is_empty());
}

#[test]
fn a_ladder_directory_is_moved_once_and_its_inner_paths_follow() {
    let m = medium(
        "film",
        &["film_9.mp4", "film_22.mp4"],
        &["film/master.m3u8", "film/v0/index.m3u8"],
    );
    let plan = rename_plan(&m, "film", "fresh");

    assert_eq!(
        plan.renames,
        pairs(&[
            ("film_9.mp4", "fresh_9.mp4"),
            ("film_22.mp4", "fresh_22.mp4"),
            ("film", "fresh"),
        ])
    );
    assert_eq!(
        plan.targets().cloned().collect::<Vec<_>>(),
        vec![
            String::from("fresh_9.mp4"),
            String::from("fresh_22.mp4"),
            String::from("fresh"),
        ]
    );
    assert_eq!(
        plan.ladders,
        vec![
            String::from("fresh/master.m3u8"),
            String::from("fresh/v0/index.m3u8"),
        ]
    );
}

#[test]
fn a_path_outside_the_naming_convention_is_neither_moved_nor_a_destination() {
    let m = medium("film", &["film_9.mp4", "holiday.mp4"], &[]);
    let plan = rename_plan(&m, "film", "fresh");

    assert_eq!(plan.renames, pairs(&[("film_9.mp4", "fresh_9.mp4")]));
    assert!(
        !plan.targets().any(|t| t == "holiday.mp4"),
        "a file nobody asked to move was counted as a destination"
    );
    assert_eq!(
        plan.files,
        vec![String::from("fresh_9.mp4"), String::from("holiday.mp4")]
    );
}

#[test]
fn a_medium_with_no_files_has_no_destinations() {
    let m = medium("film", &[], &[]);
    let plan = rename_plan(&m, "film", "fresh");
    assert!(plan.renames.is_empty());
    assert_eq!(plan.targets().count(), 0);
}

#[test]
fn the_same_short_name_moves_nothing() {
    let m = medium("film", &["film_9.mp4"], &["film/master.m3u8"]);
    let plan = rename_plan(&m, "film", "film");
    assert!(plan.renames.is_empty());
    assert_eq!(plan.files, m.files);
    assert_eq!(plan.ladders, m.ladders);
}
