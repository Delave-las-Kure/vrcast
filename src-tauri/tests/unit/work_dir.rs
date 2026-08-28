//! T450 — where a variant is written while it is being made.
//!
//! **What the old code did and why it was wrong.** `std::env::temp_dir().join("vrcast-ladder")`
//! — the system's temporary directory, which on Windows is on `C:` and on Linux is often a
//! tmpfs sized from RAM. One variant is one and a half to two gigabytes. On a machine with a
//! small system disk the build ends hours in, out of space nobody agreed to spend; on a
//! tmpfs it fills memory instead, which takes the whole machine with it.

use std::path::{Path, PathBuf};
use vrcast_studio_lib::domain::work_dir::{self, BESIDE_THE_SOURCE};

#[test]
fn the_default_is_on_the_same_disk_as_the_film() {
    // The whole argument for this default in one line: the disk a film is on certainly fits
    // a film. Nothing else can promise that on any platform.
    let source = Path::new("F:/films/season 1/episode 4.mkv");
    let work = work_dir::for_source(None, source);
    assert_eq!(
        work,
        PathBuf::from("F:/films/season 1").join(BESIDE_THE_SOURCE)
    );
    assert!(
        work.starts_with("F:/films"),
        "the working folder left the film's disk: {}",
        work.display()
    );
}

#[test]
fn the_default_is_never_the_system_temporary_directory() {
    // The fault itself, stated as a check. Any path the default produces must be somewhere
    // the person chose to keep a film — never the place this task exists to leave.
    for source in [
        "F:/films/a.mkv",
        "D:/video/b.mp4",
        "/home/someone/films/c.mkv",
    ] {
        let work = work_dir::for_source(None, Path::new(source));
        assert!(
            !work_dir::is_system_temp(&work),
            "the default for {source} landed back in the system's temporary directory: {}",
            work.display()
        );
    }
}

#[test]
fn what_a_person_chose_is_used_as_they_wrote_it() {
    // A setting that is quietly adjusted is a suggestion. If somebody points it at a
    // scratch disk, that is where it goes.
    let chosen = "E:/scratch";
    let work = work_dir::for_source(Some(chosen), Path::new("F:/films/a.mkv"));
    assert_eq!(work, PathBuf::from(chosen));
}

#[test]
fn nothing_and_blank_mean_the_same_thing() {
    // A setting stored as an empty string comes back as "" rather than absent — the store
    // keeps text. Both have to mean "not chosen", or clearing the box in the interface would
    // put the working files in a folder named nothing at all.
    let source = Path::new("F:/films/a.mkv");
    let by_default = work_dir::for_source(None, source);
    assert_eq!(work_dir::for_source(Some(""), source), by_default);
    assert_eq!(work_dir::for_source(Some("   "), source), by_default);
}

#[test]
fn a_source_with_nowhere_beside_it_still_avoids_the_temporary_directory() {
    // A bare name has no parent. The tempting fallback is the system's temporary directory,
    // which would make this fix hold everywhere except where paths are strange — and that is
    // where nobody would look for it.
    let work = work_dir::for_source(None, Path::new("film.mkv"));
    assert!(!work_dir::is_system_temp(&work), "{}", work.display());
    assert_eq!(work, PathBuf::from(BESIDE_THE_SOURCE));
}

#[test]
fn the_system_temporary_directory_is_recognised_from_inside_it() {
    // The settings screen has to be able to say what choosing it costs, so recognising it
    // has to work for a folder within it and not only for the folder itself.
    assert!(work_dir::is_system_temp(&std::env::temp_dir()));
    assert!(work_dir::is_system_temp(
        &std::env::temp_dir().join("vrcast-ladder")
    ));
    assert!(!work_dir::is_system_temp(Path::new("F:/films/vrcast-work")));
}

// ---------- what a change of path leaves behind (T453) ----------

#[test]
fn what_is_left_in_a_working_folder_is_counted() {
    // The case this exists for: a build was killed, a variant is still there, and the person
    // then points the setting somewhere else. Two gigabytes stay where nobody is looking.
    let dir = std::env::temp_dir().join(format!(
        "vrcast-leftovers-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&dir).expect("no working directory");
    std::fs::write(dir.join("film_22.mp4"), vec![0u8; 2048]).unwrap();
    std::fs::write(dir.join("film_12.mp4"), vec![0u8; 1024]).unwrap();
    // A directory among them is not a file and must not be counted as one.
    std::fs::create_dir_all(dir.join("segments")).unwrap();

    let found = work_dir::leftovers_in(&dir);
    assert_eq!(found.files, 2);
    assert_eq!(found.bytes, 3072);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_folder_that_is_not_there_is_nothing_rather_than_a_failure() {
    // This is an aside on a settings screen. It must never be able to stop somebody changing
    // a setting, and a path that has never been used is the ordinary case, not a fault.
    let found = work_dir::leftovers_in(Path::new("F:/nothing/of/the/sort"));
    assert_eq!(found.files, 0);
    assert_eq!(found.bytes, 0);
}
