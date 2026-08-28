//! Keeping account of the external programs started, and sweeping up the survivors at
//! start-up.
//!
//! Why this is needed, briefly: **the guarantees on Windows and on Linux are not equal.**
//!
//! | How the application ended | Windows | Linux |
//! |---|---|---|
//! | our code ran to the end | we kill them ourselves | we kill them ourselves |
//! | a panic, an abnormal exit | the kernel closes the job object | a kernel signal to the direct child |
//! | a termination signal, out of memory | the kernel closes the job object | a kernel signal to the direct child |
//! | a grandchild spawned by the program | closed with the job object | **not closed** |
//!
//! On Windows a job object holds the whole tree, and the kernel holds the job object. On
//! Linux the signal sent when a parent dies reaches only the direct child; there is nothing
//! to close the grandchildren with.
//!
//! This account covers what is left: the identifiers of started programs are written to the
//! database, and at the next start-up the application checks whether any survived and
//! finishes them off.
//!
//! **Checking before killing is not optional.** Process numbers are reused, and by the next
//! start-up a completely unrelated program may stand behind an old number — a person's
//! browser, for instance. So before terminating anything we check that it is the same
//! process, and on a mismatch the record is simply forgotten.
//!
//! What is compared is the **start time**, not the name. The name lies, as continuous
//! integration on Linux found out on 2026-08-25: `sh -c "sleep 300"` replaces itself with
//! `sleep`, the recorded name stops matching, and a surviving program lived through the
//! sweep. On top of that the system shows no more than fifteen characters of a name, so a
//! program with a long name would not match itself. The name stayed as a fallback — for
//! when the time could not be learned.

use super::process::{kill_pid, process_name};
use crate::store::db::{now_rfc3339, Db, DbError};

/// Record a started program so that it can be finished off after a crash.
pub fn record(db: &Db, pid: u32, program: &str, task_id: Option<&str>) -> Result<(), DbError> {
    // The identifying mark is taken right now, while the process is certainly alive and
    // certainly the one we mean. Later, anyone at all may stand behind this number.
    let identity = crate::tasks::process::process_identity(pid);
    // And who started it. A record whose owner is still running belongs to a live instance
    // and is not a survivor of anything — see `sweep_on_startup` and migration 0010.
    let owner_pid = std::process::id();
    let owner_identity = crate::tasks::process::process_identity(owner_pid);
    db.with_conn(|c| {
        c.execute(
            "INSERT OR REPLACE INTO running_processes
                 (pid, program, task_id, started_at, identity, owner_pid, owner_identity)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                pid,
                program,
                task_id,
                now_rfc3339(),
                identity,
                owner_pid,
                owner_identity
            ],
        )?;
        Ok(())
    })
}

/// Forget a record: the program finished as it should.
pub fn forget(db: &Db, pid: u32) -> Result<(), DbError> {
    db.with_conn(|c| {
        c.execute("DELETE FROM running_processes WHERE pid = ?1", [pid])?;
        Ok(())
    })
}

/// What the sweep did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SweepReport {
    /// Survivors of the previous run that were terminated.
    pub killed: Vec<u32>,
    /// Records whose numbers turned out to hold an unrelated program. Left untouched.
    pub reused: Vec<u32>,
    /// Records whose programs had already finished on their own.
    pub already_gone: Vec<u32>,
    /// Records belonging to an instance that is still running. Not touched, not forgotten.
    ///
    /// **Not a kind of cleaning up**, which is why `is_clean` ignores it: nothing survived
    /// and nothing was left behind. Somebody else is simply still working.
    pub still_owned: Vec<u32>,
}

impl SweepReport {
    pub fn is_clean(&self) -> bool {
        self.killed.is_empty() && self.reused.is_empty()
    }
}

/// Finish off the programs that survived the application's previous run.
///
/// Called once at start-up, before any new tasks appear. After the sweep the table is
/// cleared entirely: everything in it belonged to the previous run.
pub fn sweep_on_startup(db: &Db) -> Result<SweepReport, DbError> {
    type Record = (u32, String, Option<String>, Option<u32>, Option<String>);
    let records: Vec<Record> = db.with_conn(|c| {
        let mut stmt = c.prepare(
            "SELECT pid, program, identity, owner_pid, owner_identity FROM running_processes",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, u32>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<u32>>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    })?;

    let mut report = SweepReport::default();

    for (pid, program, recorded_identity, owner_pid, owner_identity) in records {
        // **Whose record is this.** An instance that is still running has not left anything
        // behind, and its work is not this sweep's to end. Checked in the same two steps the
        // child process is checked in below, and for the same reason: the number alone lies
        // once it has been handed out again.
        if let Some(owner) = owner_pid {
            if owner != std::process::id() || owner_identity.is_some() {
                let owner_is_there = process_name(owner).is_some();
                let same_owner = match (
                    &owner_identity,
                    crate::tasks::process::process_identity(owner),
                ) {
                    (Some(was), Some(now)) => *was == now,
                    // The mark could not be read on one side or the other. Sparing something
                    // that should have been ended costs an orphan until the next start;
                    // ending something that should have been spared costs hours of work.
                    _ => true,
                };
                if owner_is_there && same_owner {
                    tracing::debug!(
                        pid,
                        owner,
                        program = %program,
                        "left alone: the instance that started it is still running"
                    );
                    report.still_owned.push(pid);
                    continue;
                }
            }
        }

        // On Windows the check and the termination are done through ONE handle: between a
        // separate check and kill the system is free to hand the freed number to another
        // program, and that is the one that would be killed. The sweep runs at the
        // application's start-up — exactly when numbers are handed out in batches
        // (debt T074).
        #[cfg(windows)]
        if let Some(identity) = recorded_identity.as_deref() {
            match crate::tasks::process::verify_and_terminate(pid, identity) {
                None => report.already_gone.push(pid),
                Some(true) => {
                    tracing::warn!(
                        pid,
                        program = %program,
                        "finished off a program that survived the previous run"
                    );
                    report.killed.push(pid);
                }
                Some(false) => {
                    tracing::debug!(
                        pid,
                        expected = %program,
                        "the process number was reused; record forgotten without terminating"
                    );
                    report.reused.push(pid);
                }
            }
            continue;
        }

        match process_name(pid) {
            None => report.already_gone.push(pid),
            Some(actual) => {
                if same_process(pid, &program, &actual, recorded_identity.as_deref()) {
                    if kill_pid(pid) {
                        tracing::warn!(
                            pid,
                            program = %program,
                            "finished off a program that survived the previous run"
                        );
                        report.killed.push(pid);
                    } else {
                        // We could not terminate it — but we do not keep quiet about that.
                        tracing::error!(pid, program = %program, "could not terminate a surviving program");
                        report.reused.push(pid);
                    }
                } else {
                    // The number was reused by an unrelated program. Left untouched.
                    tracing::debug!(
                        pid,
                        expected = %program,
                        found = %actual,
                        "the process number was reused; record forgotten without terminating"
                    );
                    report.reused.push(pid);
                }
            }
        }
    }

    db.with_conn(|c| {
        c.execute("DELETE FROM running_processes", [])?;
        Ok(())
    })?;

    Ok(report)
}

/// Whether this is the process we recorded — or someone else already stands behind its
/// number.
///
/// What is compared first is the **start time**: it does not change when the image is
/// replaced and it is not truncated. The name is a fallback for when the time could not be
/// learned (an old record without it, a non-Linux Unix, a refused permission).
///
/// Why the name is not enough was found out by continuous integration on Linux on
/// 2026-08-25: `sh -c "sleep 300"` replaces itself with `sleep`, the recorded name stops
/// matching, and a surviving program lived through the sweep. On top of that
/// `/proc/<pid>/comm` is truncated at fifteen characters — a program with a long name would
/// not match itself.
fn same_process(
    pid: u32,
    recorded_name: &str,
    actual_name: &str,
    recorded_identity: Option<&str>,
) -> bool {
    if let Some(recorded) = recorded_identity {
        if let Some(actual) = crate::tasks::process::process_identity(pid) {
            return actual == recorded;
        }
    }
    names_match(actual_name, recorded_name)
}

/// Whether two program names match.
///
/// They are compared without the extension and without regard to case: a record may hold
/// `ffmpeg` while the system shows `ffmpeg.exe`. Truncation is taken into account too: the
/// system shows no more than fifteen characters of a name, and a long name would otherwise
/// not match itself.
fn names_match(actual: &str, recorded: &str) -> bool {
    /// How many characters of a name the system shows. It will never show more.
    const SHOWN_CHARS: usize = 15;

    let norm = |s: &str| {
        let base = std::path::Path::new(s)
            .file_stem()
            .map(|x| x.to_string_lossy().into_owned())
            .unwrap_or_else(|| s.to_owned());
        base.to_lowercase()
    };

    let (a, r) = (norm(actual), norm(recorded));
    if a == r {
        return true;
    }
    // The shown name hit the limit — compare only as much as is visible.
    a.chars().count() == SHOWN_CHARS && r.chars().take(SHOWN_CHARS).collect::<String>() == a
}

#[cfg(test)]
mod tests {
    use super::names_match;

    #[test]
    fn names_match_ignoring_extension_and_case() {
        assert!(names_match("ffmpeg.exe", "ffmpeg"));
        assert!(names_match("FFmpeg.EXE", "ffmpeg"));
        assert!(names_match("ffmpeg", "C:/tools/ffmpeg.exe"));
        assert!(!names_match("chrome.exe", "ffmpeg"));
        assert!(!names_match("ffmpeg-probe", "ffmpeg"));
    }
}
