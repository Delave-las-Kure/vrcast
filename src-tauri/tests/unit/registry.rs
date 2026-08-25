//! Tests for the start-up sweep of surviving programs.
//!
//! Two opposite properties are checked, and the second matters more than the first:
//!
//! 1. A program that survived the previous run **will** be terminated.
//! 2. An unrelated program that took over a reused number **will not** be touched.
//!
//! The second matters more because a mistake there costs more: failing to end our own is an
//! orphaned process until the next start, while ending someone else's is a person's killed
//! browser or, worse, someone else's long-running work.

use std::time::Duration;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::tasks::process::ManagedProcess;
use vrcast_studio_lib::tasks::registry;

use super::proc_check::{alive, long_running};

#[tokio::test]
async fn a_surviving_program_is_finished_off_at_start_up() {
    let db = Db::open_in_memory().unwrap();

    // The previous run is portrayed: the program is running, there is a record of it, and
    // the application "died" without managing to end it.
    let (prog, args) = long_running();
    let mut p = ManagedProcess::spawn(prog, &args).unwrap();
    let pid = p.id().unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        alive(pid),
        "the process did not start — there is nothing to check"
    );

    registry::record(&db, pid, prog, None).unwrap();

    let report = registry::sweep_on_startup(&db).unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;

    assert!(
        report.killed.contains(&pid),
        "the surviving program was not finished off: {report:?}"
    );
    assert!(!alive(pid), "the process {pid} lived through the sweep");
    assert!(
        !report.is_clean(),
        "the sweep reported there had been nothing to clean up"
    );

    // The table is cleared: everything in it belonged to the previous run.
    let second = registry::sweep_on_startup(&db).unwrap();
    assert!(
        second.is_clean(),
        "records were left after the sweep: {second:?}"
    );

    let _ = p.kill_tree().await;
}

#[tokio::test]
async fn an_unrelated_program_under_a_reused_number_is_left_alone() {
    // The main check. Process numbers are reused: by the next start-up a person's browser
    // may well stand behind an old number. Killing it is not allowed.
    //
    // The reuse is imitated the way it really happens: the record holds the identifying
    // mark of ANOTHER, already dead process, while the number is held by a live one. The
    // old imitation — swapping the name while the process lived — stopped being true once
    // the comparison moved to the start time: the time honestly says "this is the same
    // process", and the sweep would have been right to kill it.
    let db = Db::open_in_memory().unwrap();

    let (prog, args) = long_running();
    let mut p = ManagedProcess::spawn(prog, &args).unwrap();
    let pid = p.id().unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(alive(pid));

    registry::record(&db, pid, prog, None).unwrap();
    // The mark is swapped: the record now describes a process that no longer exists.
    db.with_conn(|c| {
        c.execute(
            "UPDATE running_processes SET identity = ?1 WHERE pid = ?2",
            rusqlite::params!["definitely-another-process", pid],
        )?;
        Ok(())
    })
    .unwrap();

    let report = registry::sweep_on_startup(&db).unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;

    assert!(
        report.killed.is_empty(),
        "AN UNRELATED PROGRAM WAS KILLED under a reused number: {report:?}"
    );
    assert!(
        report.reused.contains(&pid),
        "the number's reuse went unnoticed: {report:?}"
    );
    assert!(
        alive(pid),
        "the process {pid} was killed although the number is another's"
    );

    p.kill_tree().await.unwrap();
}

#[tokio::test]
async fn an_old_record_with_no_mark_is_compared_by_name() {
    // Records made before the identifying mark appeared will turn up at the first start
    // after the application is upgraded. For them the name comparison remains — worse, but
    // better than killing blind.
    let db = Db::open_in_memory().unwrap();

    let (prog, args) = long_running();
    let mut p = ManagedProcess::spawn(prog, &args).unwrap();
    let pid = p.id().unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;

    registry::record(&db, pid, "ffmpeg", None).unwrap();
    // There is no mark — as in a record made by an earlier version of the application.
    db.with_conn(|c| {
        c.execute(
            "UPDATE running_processes SET identity = NULL WHERE pid = ?1",
            [pid],
        )?;
        Ok(())
    })
    .unwrap();

    let report = registry::sweep_on_startup(&db).unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;

    assert!(
        report.killed.is_empty(),
        "a program whose name did not match the record was killed: {report:?}"
    );
    assert!(
        alive(pid),
        "the process {pid} was killed on a name mismatch"
    );

    p.kill_tree().await.unwrap();
}

#[tokio::test]
async fn a_record_of_an_already_finished_program_is_harmless() {
    let db = Db::open_in_memory().unwrap();

    let (prog, args) = long_running();
    let mut p = ManagedProcess::spawn(prog, &args).unwrap();
    let pid = p.id().unwrap();
    registry::record(&db, pid, prog, None).unwrap();

    // The program finished as it should, before the sweep.
    p.kill_tree().await.unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;

    let report = registry::sweep_on_startup(&db).unwrap();
    // The record must be classified rather than quietly thrown away: usually "already
    // gone", and on an instant reuse of the number by the system, "the number is another's".
    // (The right-hand side of the "or" used to be killed.is_empty() — the same check as the
    // assert below, so the first assertion could never fail at all.)
    assert!(
        report.already_gone.contains(&pid) || report.reused.contains(&pid),
        "a finished program was handled wrongly: {report:?}"
    );
    assert!(
        report.killed.is_empty(),
        "something that was already gone got killed: {report:?}"
    );
}

#[tokio::test]
async fn a_normal_ending_removes_the_record() {
    let db = Db::open_in_memory().unwrap();

    let (prog, args) = long_running();
    let mut p = ManagedProcess::spawn(prog, &args).unwrap();
    let pid = p.id().unwrap();
    registry::record(&db, pid, prog, None).unwrap();

    p.kill_tree().await.unwrap();
    registry::forget(&db, pid).unwrap();

    let report = registry::sweep_on_startup(&db).unwrap();
    assert!(
        report.is_clean() && report.already_gone.is_empty(),
        "the record was not removed on a normal ending: {report:?}"
    );
}
