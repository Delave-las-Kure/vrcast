//! T021 — a check that a process tree really does get terminated.
//!
//! Constitution, principle III (NOT NEGOTIABLE) and SC-010. What is checked is not that a
//! call to terminate returns success but that **no processes are left**: an orphaned
//! `ffmpeg` going on writing into the result file was the original incident.
//!
//! The grandchild here is not for completeness. `ffmpeg` and `ssh` spawn children of their
//! own, and ending only the direct child leaves those running — that is, exactly the fault
//! being guarded against.

use std::time::Duration;
use vrcast_studio_lib::tasks::process::ManagedProcess;

/// A long-running command available on both target operating systems.
use super::proc_check::{alive, children_of, long_running};

#[tokio::test]
async fn cancelling_ends_the_started_process() {
    let (prog, args) = long_running();
    let mut p = ManagedProcess::spawn(prog, &args).expect("the process did not start");
    let pid = p.id().expect("there is no process id");

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        alive(pid),
        "the process did not start, or died at once — there is nothing to check"
    );

    p.kill_tree().await.expect("the termination failed");
    tokio::time::sleep(Duration::from_millis(500)).await;

    assert!(
        !alive(pid),
        "the process {pid} lived through the cancellation"
    );
}

#[tokio::test]
async fn cancelling_takes_the_grandchildren_too() {
    // The main check. This is exactly where an ordinary terminate-by-id breaks: the direct
    // child dies while its own children go on running — just like an orphaned ffmpeg going
    // on spoiling the result file.
    let (prog, args) = long_running();
    let mut p = ManagedProcess::spawn(prog, &args).expect("the process did not start");
    let parent = p.id().expect("there is no id");

    tokio::time::sleep(Duration::from_millis(900)).await;

    let grandchildren = children_of(parent);
    assert!(
        !grandchildren.is_empty(),
        "no grandchildren appeared — the test checks nothing (parent {parent})"
    );

    p.kill_tree().await.expect("the termination failed");
    tokio::time::sleep(Duration::from_millis(900)).await;

    assert!(
        !alive(parent),
        "the parent {parent} lived through the cancellation"
    );
    let survivors: Vec<u32> = grandchildren
        .iter()
        .copied()
        .filter(|g| alive(*g))
        .collect();
    assert!(
        survivors.is_empty(),
        "ORPHANED PROCESSES lived through the cancellation: {survivors:?} (grandchildren {grandchildren:?})"
    );
}

#[tokio::test]
async fn terminating_twice_is_not_an_error() {
    // Constitution, principle V: repeating must be safe. Cancel may be pressed twice, and
    // the second time must not turn into an error.
    let (prog, args) = long_running();
    let mut p = ManagedProcess::spawn(prog, &args).unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    p.kill_tree().await.expect("the first termination");
    p.kill_tree()
        .await
        .expect("terminating a second time must not be an error");
}

#[cfg(windows)]
#[tokio::test]
async fn the_application_dying_takes_its_children_with_it() {
    // The property an ordinary termination does not have: the kernel closes the job
    // object's handle when the owning process dies — including when the application is
    // killed from Task Manager and not one line of its code runs any more (SC-010).
    //
    // Here that is reproduced by closing the handle: the structure is dropped without
    // terminating explicitly.
    let (prog, args) = long_running();
    let pid = {
        let p = ManagedProcess::spawn(prog, &args).unwrap();
        let pid = p.id().unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(alive(pid), "the process did not start");
        pid
        // p is dropped here: the job handle closes
    };

    tokio::time::sleep(Duration::from_millis(700)).await;
    assert!(
        !alive(pid),
        "the process {pid} lived through the job handle closing — the guarantee does not hold"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn pausing_and_carrying_on_work() {
    // FR-083a. On Unix it is checked by the process state; on Windows the threads' state
    // cannot be read so simply, so there it is covered by a check made by hand.
    let (prog, args) = long_running();
    // Mutable: the freeze is remembered in the process itself so a second one does not
    // throw the pause counter off (T070).
    let mut p = ManagedProcess::spawn(prog, &args).unwrap();
    let pid = p.id().unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    p.suspend().expect("pausing failed");
    // Repeating must be harmless: otherwise one "carry on" would not be enough, and the
    // task would hang for good.
    p.suspend()
        .expect("pausing a second time counted as an error");
    assert!(p.is_suspended());
    tokio::time::sleep(Duration::from_millis(300)).await;
    let state = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
    assert!(
        state.contains(") T "),
        "the process is not paused, state: {state}"
    );

    p.resume().expect("carrying on failed");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let state = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
    assert!(
        !state.contains(") T "),
        "the process did not carry on, state: {state}"
    );

    p.kill_tree().await.unwrap();
}
