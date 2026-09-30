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

// ---------- T661: the heavy short runs go through the managed path too ----------

mod run_to_end {
    use std::time::{Duration, Instant};
    use tokio_util::sync::CancellationToken;
    use vrcast_studio_lib::tasks::process::{run_to_end, RunError};

    use super::super::proc_check::{alive, children_of, long_running};

    const HELPER: &str = "process::run_to_end::the_parent_that_gets_killed";
    const HELPER_ENV: &str = "VRCAST_T661_HELPER";

    #[tokio::test]
    async fn a_program_that_says_nothing_is_stopped_for_it() {
        let (prog, args) = long_running();
        let started = Instant::now();
        let outcome = run_to_end(prog, &args, None, Duration::from_millis(1_200)).await;
        assert!(
            matches!(outcome, Err(RunError::Stalled { .. })),
            "a mute program was not stopped: {outcome:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_cancel_stops_it_and_waits_for_it() {
        let (prog, args) = long_running();
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            trigger.cancel();
        });
        let started = Instant::now();
        let outcome = run_to_end(prog, &args, Some(&cancel), Duration::from_secs(600)).await;
        assert!(matches!(outcome, Err(RunError::Cancelled)), "{outcome:?}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the cancel took {:?} to land",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn a_program_that_finishes_gives_back_what_it_said() {
        let (prog, args): (&str, Vec<String>) = if cfg!(windows) {
            ("cmd", vec!["/c".into(), "echo hello".into()])
        } else {
            ("sh", vec!["-c".into(), "echo hello".into()])
        };
        let done = run_to_end(prog, &args, None, Duration::from_secs(30))
            .await
            .expect("a program that finishes did not finish");
        assert!(done.status.success());
        assert!(String::from_utf8_lossy(&done.stdout).contains("hello"));
    }

    /// Half of the check below: the application that is killed. Started as a separate
    /// process; without the environment it does nothing.
    #[test]
    #[ignore = "half of the parent-death check: started as a separate process"]
    fn the_parent_that_gets_killed() {
        if std::env::var(HELPER_ENV).is_err() {
            return;
        }
        let rt = tokio::runtime::Runtime::new().expect("no runtime");
        rt.block_on(async {
            let (prog, args) = long_running();
            let _ = run_to_end(prog, &args, None, Duration::from_secs(3600)).await;
        });
    }

    #[test]
    fn a_heavy_run_dies_with_the_application() {
        // QA-24B-02, the real Windows probe: a `quiet(..).output()` child lived on 2.5 s past
        // its parent's death, a `ManagedProcess` one did not. `run_to_end` is what the heavy
        // short runs go through now; this kills its parent from outside, with no chance to
        // tidy up, and looks for survivors.
        let mut parent = std::process::Command::new(
            std::env::current_exe().expect("could not find our own program"),
        )
        .args([HELPER, "--exact", "--ignored", "--test-threads=1"])
        .env(HELPER_ENV, "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("the parent did not start");

        let pid = parent.id();
        let deadline = Instant::now() + Duration::from_secs(30);
        let children = loop {
            let found = children_of(pid);
            if !found.is_empty() {
                break found;
            }
            assert!(
                Instant::now() < deadline,
                "the parent never started its child — nothing to check"
            );
            std::thread::sleep(Duration::from_millis(200));
        };
        let grandchildren: Vec<u32> = children.iter().flat_map(|c| children_of(*c)).collect();
        assert!(children.iter().all(|c| alive(*c)));

        parent.kill().expect("could not kill the parent");
        let _ = parent.wait();
        std::thread::sleep(Duration::from_millis(2_500));

        let survivors: Vec<u32> = children
            .iter()
            .chain(grandchildren.iter())
            .copied()
            .filter(|p| alive(*p))
            .collect();
        assert!(
            survivors.is_empty(),
            "ORPHANED: {survivors:?} outlived the application that started them \
             (children {children:?}, grandchildren {grandchildren:?})"
        );
    }
}
