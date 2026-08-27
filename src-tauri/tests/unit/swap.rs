//! T259 — swap on a small server (FR-134).
//!
//! The numbers being checked are a choice rather than a measurement, and the checks are
//! written to say so: they are about the shape of the answer — never short of the target,
//! never filling the disk, never a number nobody chose — rather than about the constants
//! themselves, which may be changed by an ordinary decision.

use vrcast_studio_lib::domain::swap::{
    decide, Swap, GRANULARITY_MB, KEEP_FREE_MB, SMALLEST_MB, TARGET_TOTAL_MB,
};

/// The stand the owner set aside: the cheapest tier, and what a first server usually is.
const STAND_MEMORY_MB: u32 = 961;
const STAND_DISK_MB: u32 = 9 * 1024;

#[test]
fn the_stand_gets_a_swap_file() {
    // The case FR-134 was written for. Without it apt is killed part-way through installing,
    // and the person is left with a server that is neither bare nor deployed.
    let Swap::Make { megabytes } = decide(STAND_MEMORY_MB, 0, STAND_DISK_MB) else {
        panic!("the cheapest tier of VPS was told it needs no swap");
    };
    assert!(
        STAND_MEMORY_MB + megabytes >= TARGET_TOTAL_MB,
        "the file leaves the total short of the target: {megabytes} MB on top of {STAND_MEMORY_MB}"
    );
    assert_eq!(
        megabytes % GRANULARITY_MB,
        0,
        "a size nobody chose and nobody can read: {megabytes} MB"
    );
}

#[test]
fn a_server_with_enough_memory_is_left_alone() {
    assert_eq!(decide(TARGET_TOTAL_MB, 0, STAND_DISK_MB), Swap::NotNeeded);
    assert_eq!(decide(8192, 0, STAND_DISK_MB), Swap::NotNeeded);
}

#[test]
fn swap_that_is_already_there_counts() {
    // Otherwise every repeat of the deployment adds another file, and the check that makes a
    // repeat safe (FR-124) would be the one thing failing to do its job.
    assert_eq!(
        decide(1024, 1024, STAND_DISK_MB),
        Swap::NotNeeded,
        "existing swap was not counted, so a repeat would add a second file"
    );
}

#[test]
fn a_tiny_shortfall_still_gets_a_file_worth_making() {
    // Rounding alone would give a file of a few dozen megabytes: all of the cost and none of
    // the point.
    let Swap::Make { megabytes } = decide(TARGET_TOTAL_MB - 64, 0, STAND_DISK_MB) else {
        panic!("a server just short of the target was told it needs nothing");
    };
    assert!(
        megabytes >= SMALLEST_MB,
        "a file too small to be worth its disk: {megabytes} MB"
    );
}

#[test]
fn a_full_disk_is_refused_rather_than_filled() {
    // The trade this refusal avoids: a swap file that fills the disk leaves a server that
    // installs nothing *and* serves nothing — and the second failure shows up later, when a
    // video is being sent, where nobody would connect it to this.
    let verdict = decide(STAND_MEMORY_MB, 0, KEEP_FREE_MB);
    let Swap::NoRoom { wanted_mb, free_mb } = verdict else {
        panic!("a swap file was made on a disk with no room: {verdict:?}");
    };
    assert!(wanted_mb > 0);
    assert_eq!(free_mb, KEEP_FREE_MB);
}

#[test]
fn the_disk_keeps_its_headroom() {
    // Exactly enough for the file and nothing else is still not enough: the serving directory
    // is on the same disk.
    let Swap::Make { megabytes } = decide(STAND_MEMORY_MB, 0, STAND_DISK_MB) else {
        panic!("no file was planned");
    };
    assert_eq!(
        decide(STAND_MEMORY_MB, 0, megabytes),
        Swap::NoRoom {
            wanted_mb: megabytes,
            free_mb: megabytes
        },
        "the file was made with nothing left over for the videos"
    );
    assert!(matches!(
        decide(STAND_MEMORY_MB, 0, megabytes + KEEP_FREE_MB),
        Swap::Make { .. }
    ));
}
