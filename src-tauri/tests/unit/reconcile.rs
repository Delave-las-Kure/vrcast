//! T051 — reconciling the catalogue with the directory's contents (FR-015, FR-018).
//!
//! Checked without a server: reconciling is a pure function, and it is where a file is
//! easiest to lose. Such a loss must be caught by a test rather than by a person who one
//! day finds disk space missing.

use vrcast_studio_lib::domain::manifest::Manifest;
use vrcast_studio_lib::domain::media::Media;
use vrcast_studio_lib::server::listing::Entry;
use vrcast_studio_lib::server::reconcile::reconcile;

fn file(name: &str, size: u64) -> Entry {
    Entry {
        name: name.to_owned(),
        size_bytes: size,
        is_dir: false,
    }
}

fn dir(name: &str, size: u64) -> Entry {
    Entry {
        name: name.to_owned(),
        size_bytes: size,
        is_dir: true,
    }
}

fn manifest_with(media: Vec<Media>) -> Manifest {
    Manifest {
        generation: 1,
        media,
        ..Manifest::empty()
    }
}

fn media(id: &str, slug: &str, files: &[&str], ladders: &[&str]) -> Media {
    let mut m = Media::new(id, slug, slug, "2026-08-01T10:00:00Z");
    m.files = files.iter().map(|s| (*s).to_owned()).collect();
    m.ladders = ladders.iter().map(|s| (*s).to_owned()).collect();
    m
}

#[test]
fn files_outside_the_catalogue_land_among_the_unrecognised() {
    // FR-015: they must not be hidden. A file invisible in the application still takes up
    // room and is still served by its direct link.
    let m = manifest_with(vec![media("m1", "film", &["film_22.mp4"], &[])]);
    let entries = vec![
        file("film_22.mp4", 100),
        file("посторонний.mp4", 200),
        file("ещё один.mkv", 300),
    ];

    let r = reconcile(&m, &entries);
    let names: Vec<&str> = r.unrecognized.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["посторонний.mp4", "ещё один.mkv"]);
}

#[test]
fn a_catalogued_file_that_is_gone_is_marked_rather_than_vanishing() {
    // FR-018. Removing it quietly would hide the loss from a person.
    let m = manifest_with(vec![media(
        "m1",
        "film",
        &["film_22.mp4", "film_10.mp4"],
        &[],
    )]);
    let entries = vec![file("film_22.mp4", 100)];

    let r = reconcile(&m, &entries);
    let files = &r.media_files[0].files;

    assert_eq!(files.len(), 2, "the missing file vanished from the medium");
    assert!(
        files
            .iter()
            .find(|f| f.path == "film_22.mp4")
            .unwrap()
            .exists
    );
    assert!(
        !files
            .iter()
            .find(|f| f.path == "film_10.mp4")
            .unwrap()
            .exists
    );
}

#[test]
fn a_quality_ladder_is_one_entry_rather_than_a_hundred_segments() {
    // The catalogue's path is nested, while what it takes up is a top-level entry — the
    // directory.
    let m = manifest_with(vec![media("m1", "film", &[], &["film/master.m3u8"])]);
    let entries = vec![dir("film", 5_000_000)];

    let r = reconcile(&m, &entries);
    assert!(
        r.unrecognized.is_empty(),
        "the quality ladder's directory was declared unrecognised: {:?}",
        r.unrecognized
    );
    assert!(r.media_files[0].ladders[0].exists);
}

#[test]
fn housekeeping_entries_are_not_shown_as_video() {
    // Otherwise a person sees in their library the catalogue of their own library and the
    // directory of trimmed descriptions — and decides those are their files.
    let m = manifest_with(vec![]);
    let entries = vec![
        file("library.json", 42),
        dir("_slow", 10),
        file("настоящее видео.mp4", 100),
    ];

    let r = reconcile(&m, &entries);
    let names: Vec<&str> = r.unrecognized.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["настоящее видео.mp4"]);
}

#[test]
fn no_catalogue_entry_is_lost_or_counted_twice() {
    // The property reconciling exists for in the first place.
    let m = manifest_with(vec![
        media("m1", "a", &["a_10.mp4", "a_22.mp4"], &["a/master.m3u8"]),
        media("m2", "b", &["b_10.mp4"], &[]),
    ]);
    let entries = vec![
        file("a_10.mp4", 1),
        file("a_22.mp4", 2),
        dir("a", 3),
        file("b_10.mp4", 4),
        file("чужое.mp4", 5),
        dir("чужой каталог", 6),
        file("library.json", 7),
    ];

    let r = reconcile(&m, &entries);

    let accounted: usize = r
        .media_files
        .iter()
        .map(|mf| mf.files.len() + mf.ladders.len())
        .sum::<usize>()
        + r.unrecognized.len();
    // Seven entries, minus the housekeeping catalogue.
    assert_eq!(accounted, 6, "entries were lost or doubled: {r:?}");

    let mut visible: Vec<&str> = r
        .media_files
        .iter()
        .flat_map(|mf| mf.files.iter().chain(mf.ladders.iter()))
        .map(|f| f.path.as_str())
        .chain(r.unrecognized.iter().map(|e| e.name.as_str()))
        .collect();
    visible.sort_unstable();
    let before = visible.len();
    visible.dedup();
    assert_eq!(
        before,
        visible.len(),
        "an entry landed in two places at once"
    );
}

#[test]
fn a_nested_path_is_not_credited_with_the_directory_size() {
    // We do not know the size of `film/master.m3u8` itself: what is known is the size of
    // the whole directory. Crediting it to the description would show a person a text file
    // weighing five megabytes.
    let m = manifest_with(vec![media("m1", "film", &[], &["film/master.m3u8"])]);
    let entries = vec![dir("film", 5_000_000)];

    let r = reconcile(&m, &entries);
    assert_eq!(r.media_files[0].ladders[0].size_bytes, 0);
}

#[test]
fn an_empty_catalogue_hands_back_the_whole_directory_as_unrecognised() {
    // The ordinary state of a server that was uploaded to by scripts: no catalogue, but
    // files aplenty. The library must show all of them.
    let m = Manifest::empty();
    let entries = vec![file("one.mp4", 1), file("two.mp4", 2), dir("three", 3)];

    let r = reconcile(&m, &entries);
    assert!(r.media_files.is_empty());
    assert_eq!(r.unrecognized.len(), 3);
}
