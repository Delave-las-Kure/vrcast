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

/// Portray a run that is over: whatever wrote this record is no longer there.
///
/// **Every test below needs it stated.** These processes are recorded by the test itself,
/// which is alive — so without this they would describe a live instance sweeping its own
/// records, and that is the one case the sweep must not act on
/// (`a_program_started_by_an_instance_that_is_still_running_is_left_alone`). An empty owner
/// is also what rows written before migration 0010 carry, and they are swept the same way.
fn the_run_that_made_it_is_over(db: &Db, pid: u32) {
    db.with_conn(|c| {
        c.execute(
            "UPDATE running_processes SET owner_pid = NULL, owner_identity = NULL WHERE pid = ?1",
            [pid],
        )?;
        Ok(())
    })
    .unwrap();
}

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
    the_run_that_made_it_is_over(&db, pid);

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
    the_run_that_made_it_is_over(&db, pid);
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
    the_run_that_made_it_is_over(&db, pid);
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
    the_run_that_made_it_is_over(&db, pid);

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
    the_run_that_made_it_is_over(&db, pid);

    p.kill_tree().await.unwrap();
    registry::forget(&db, pid).unwrap();

    let report = registry::sweep_on_startup(&db).unwrap();
    assert!(
        report.is_clean() && report.already_gone.is_empty(),
        "the record was not removed on a normal ending: {report:?}"
    );
}

#[tokio::test]
async fn a_program_started_by_an_instance_that_is_still_running_is_left_alone() {
    // **The hazard the tray creates, and the reason it cannot be introduced without this**
    // (R-34). Today closing the window ends the process, so there is never a live first
    // instance for a second one to trip over. Minimising to the tray creates exactly that:
    // the application goes on running with encodes in flight, somebody starts it again, and
    // the second instance sweeps — finding records of programs that are alive, belonging to
    // the first instance, and finishing them off. Hours of encoding, killed by opening the
    // application.
    //
    // The sweep cannot tell them apart by what it records now. It knows a program is alive
    // and that there is a record of it; it does not know **whose** record. So it must know:
    // a record whose owner is still running is not a survivor of a previous run, and is not
    // this sweep's to touch.
    //
    // This test process plays the first instance — it started the program, and it is alive.
    let db = Db::open_in_memory().unwrap();

    let (prog, args) = long_running();
    let mut p = ManagedProcess::spawn(prog, &args).unwrap();
    let pid = p.id().unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(alive(pid), "the process did not start — nothing to check");

    registry::record(&db, pid, prog, None).unwrap();

    let report = registry::sweep_on_startup(&db).unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;

    assert!(
        !report.killed.contains(&pid),
        "the sweep finished off a program belonging to an instance that is still running:          {report:?}"
    );
    assert!(
        alive(pid),
        "the program {pid} was killed by the start-up sweep of another instance"
    );

    let _ = p.kill_tree().await;
}

#[tokio::test]
async fn a_record_whose_owner_has_since_died_is_still_swept() {
    // The other half of the ownership rule, and the one that keeps it from becoming an
    // excuse: an empty owner is swept because it is old, but a **recorded** owner that is
    // no longer running is exactly the previous run this whole module exists for. Without
    // this, "the owner is written down" would quietly turn into "never sweep anything".
    let db = Db::open_in_memory().unwrap();
    let (prog, args) = long_running();

    // A process to stand in for the instance that has since gone.
    let mut gone = ManagedProcess::spawn(prog, &args).unwrap();
    let dead_owner = gone.id().unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let _ = gone.kill_tree().await;
    for _ in 0..40 {
        if !alive(dead_owner) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!alive(dead_owner), "the stand-in owner would not die");

    let mut p = ManagedProcess::spawn(prog, &args).unwrap();
    let pid = p.id().unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(alive(pid), "the process did not start — nothing to check");

    registry::record(&db, pid, prog, None).unwrap();
    db.with_conn(|c| {
        c.execute(
            "UPDATE running_processes SET owner_pid = ?2 WHERE pid = ?1",
            rusqlite::params![pid, dead_owner],
        )?;
        Ok(())
    })
    .unwrap();

    let report = registry::sweep_on_startup(&db).unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;

    assert!(
        report.killed.contains(&pid),
        "a program left behind by a run that has ended was not finished off: {report:?}"
    );
    assert!(!alive(pid), "the process {pid} lived through the sweep");

    let _ = p.kill_tree().await;
}
