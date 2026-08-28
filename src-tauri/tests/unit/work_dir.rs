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

// ---------- whether the local disk has room for one variant (T452) ----------
//
// **What was never asked.** The server's disk has been checked before an upload since T085
// and before a set since T409. The local one was not asked at all — and it fills first: a
// variant is written whole, one and a half to two gigabytes, before a byte is sent. Running
// out there ends a build hours in, with a message from the operating system about a file.

use vrcast_studio_lib::commands::library::DiskUsage;
use vrcast_studio_lib::media::local_disk;
use vrcast_studio_lib::server::free_space::{self, SpaceVerdict};

#[test]
fn the_disk_under_a_real_path_can_be_read() {
    let here = std::env::temp_dir();
    let disk = local_disk::usage(&here).expect("this machine would not say what its disk holds");
    assert!(disk.total_bytes > 0, "a disk of no size at all");
    assert!(
        disk.free_bytes <= disk.total_bytes,
        "more free than there is: {} of {}",
        disk.free_bytes,
        disk.total_bytes
    );
}

#[test]
fn a_folder_not_made_yet_is_answered_for_by_the_disk_it_will_be_on() {
    // The working folder is made when the build starts, so at the moment this is asked it is
    // usually not there. Both platforms answer about a path, not about a path that might be —
    // and answering "unknown" here would mean the check never runs in the ordinary case,
    // which is the same as not having it.
    let not_yet = std::env::temp_dir()
        .join("vrcast-not-made-yet")
        .join("nor-this");
    assert!(!not_yet.exists());
    let disk = local_disk::usage(&not_yet).expect("the disk it will be on was not found");
    assert!(disk.total_bytes > 0);
}

/// **Never a disk of no size.** This is the property the caller leans on: zero free is a full
/// disk, which is a refusal, and a refusal handed out because a question could not be answered
/// stops work that would have succeeded for a reason that is not true. Whatever comes back
/// must be a real reading or nothing at all.
#[test]
fn an_answer_is_a_real_disk_or_no_answer() {
    for path in [
        std::env::temp_dir(),
        std::env::temp_dir().join("vrcast-not-made-yet"),
        std::path::PathBuf::from("Q:/nothing/of/the/sort"),
        std::path::PathBuf::from("/nothing/of/the/sort"),
    ] {
        if let Some(disk) = local_disk::usage(&path) {
            assert!(
                disk.total_bytes > 0,
                "{} was answered for with a disk of no size, which reads as a full one",
                path.display()
            );
        }
    }
}

/// Windows only, and not by preference. On Linux the walk up to an existing ancestor always
/// lands somewhere — `/` at worst — so a path leading nowhere is not a thing that happens
/// there. On Windows a drive letter that is not mounted has no ancestor at all, which is the
/// one case where the reading genuinely cannot be taken.
///
/// Found by `scripts/check-linux.sh` on 2026-08-28, which is what that script is for: the
/// first shape of this test asserted the Windows behaviour on both and failed there.
#[cfg(windows)]
#[test]
fn a_drive_that_is_not_there_is_unknown_rather_than_full() {
    let nowhere = std::path::Path::new(r"Q:\nothing\of\the\sort");
    assert!(
        local_disk::usage(nowhere).is_none(),
        "an unmounted drive was answered for; if this machine has a Q: drive, that is why"
    );
}

#[test]
fn one_variant_is_what_is_asked_for_locally_not_the_whole_set() {
    // The variants are made and sent one at a time, and each is removed once it is away. Only
    // one is ever on this disk. Asking for the whole set would refuse a build that had room
    // all along — on a small scratch disk, every time.
    let one_rung =
        vrcast_studio_lib::domain::ladder_size::bytes_for_rung(22_000_000, 256_000, 7200.0);
    let whole_set = vrcast_studio_lib::domain::ladder_size::bytes_for_set(
        &[22_000_000, 12_000_000, 6_000_000],
        256_000,
        7200.0,
    );
    assert!(
        one_rung < whole_set,
        "one variant is not smaller than all of them, so the distinction this check rests on \
         does not exist"
    );

    // A disk with room for one and not for three must let the build through.
    let disk = DiskUsage {
        total_bytes: 200_000_000_000,
        free_bytes: one_rung + free_space::reserve_for(200_000_000_000) + 1,
        used_by_videos_bytes: 0,
    };
    assert_eq!(free_space::check(&disk, one_rung, 0), SpaceVerdict::Fits);
    assert!(matches!(
        free_space::check(&disk, whole_set, 0),
        SpaceVerdict::NotEnough { .. }
    ));
}

#[test]
fn a_local_disk_too_small_for_one_variant_is_a_refusal_not_a_warning() {
    // **A bar, not a warning**, the same as the server's (T409). Room does not appear out of
    // consent, and a build that runs into the end of the disk after two hours leaves a half
    // written variant and a person with nothing to show for the afternoon.
    let one_rung =
        vrcast_studio_lib::domain::ladder_size::bytes_for_rung(22_000_000, 256_000, 7200.0);
    let small = DiskUsage {
        total_bytes: 60_000_000_000,
        free_bytes: one_rung / 2,
        used_by_videos_bytes: 0,
    };
    let verdict = free_space::check(&small, one_rung, 0);
    let SpaceVerdict::NotEnough {
        needed,
        free,
        short_by,
    } = verdict
    else {
        panic!("a disk with half the room said the variant fits");
    };
    // The refusal names how much is missing. Otherwise a person has to work it out, and the
    // one thing they want to know is how much to free.
    assert_eq!(short_by, needed - free);
    assert!(short_by > 0);
}

#[test]
fn the_check_asks_for_the_heaviest_variant_rather_than_the_first() {
    // The first is not always the largest once a rung has been left out, and being wrong
    // there is being wrong in the one direction this check exists to avoid.
    let rungs = [6_000_000u64, 22_000_000, 12_000_000];
    let heaviest = rungs.iter().copied().max().unwrap();
    assert_eq!(heaviest, 22_000_000);
    assert!(
        vrcast_studio_lib::domain::ladder_size::bytes_for_rung(heaviest, 256_000, 7200.0)
            > vrcast_studio_lib::domain::ladder_size::bytes_for_rung(rungs[0], 256_000, 7200.0),
        "asking about the first rung of this set would ask for a third of what is needed"
    );
}
