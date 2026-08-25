//! Process-state checks for the tests.
//!
//! Shared by the tests for starting programs and by those for the sweep: both check the
//! same thing — whether a process really died — and copies of this check that drifted apart
//! have already once given different answers to the same question.

/// Whether a process is really running.
///
/// On Unix it is not enough that `/proc/<pid>` exists: a **zombie** has one too. A zombie is
/// an already dead process whose place in the table is held until its parent collects its
/// exit code. In the tests the parent is either the test itself or the container's first
/// process, and neither is in a hurry to do so — a killed program would look alive.
///
/// Caught by a run on Linux on 2026-08-25, twice: first in the sweep, then on the
/// grandchildren of a cancellation. In real life this does not happen — the system adopts
/// the orphans.
pub fn alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
            .unwrap_or(false)
    }
    #[cfg(unix)]
    {
        // The third field in /proc/<pid>/stat is the state; "Z" means a zombie. Counting
        // starts from the last closing bracket: the second field is the program's name in
        // brackets, and it can hold both spaces and brackets of its own.
        match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => match stat.rfind(')') {
                Some(i) => !matches!(stat[i + 1..].split_whitespace().next(), Some("Z")),
                None => false,
            },
            Err(_) => false,
        }
    }
}

/// The ids of a process's direct children.
///
/// Counting processes by name will not do: the tests run in parallel, and other people's
/// children get counted in. What has to be checked is the parentage, not a matching name.
pub fn children_of(pid: u32) -> Vec<u32> {
    let out = if cfg!(windows) {
        std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "(Get-CimInstance Win32_Process -Filter 'ParentProcessId={pid}').ProcessId"
                ),
            ])
            .output()
    } else {
        std::process::Command::new("pgrep")
            .args(["-P", &pid.to_string()])
            .output()
    };

    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter_map(|l| l.trim().parse::<u32>().ok())
            .collect(),
        Err(_) => Vec::new(),
    }
}

// The module is shared by two sets of checks, and each takes what it needs. From here only
// the unit set uses this: the integration tests need nothing but the liveness check. The
// attribute sits here rather than on the whole file so that a function genuinely forgotten
// is still noticed.
#[allow(dead_code)]
/// A long-running program to check termination against.
pub fn long_running() -> (&'static str, Vec<String>) {
    if cfg!(windows) {
        // ping with a large count is the most portable "sleeping" process on Windows.
        (
            "cmd",
            vec!["/c".into(), "ping -n 300 127.0.0.1 >nul".into()],
        )
    } else {
        // The shell is deliberate: it spawns a child of its own, and that is what proves
        // termination takes the whole tree rather than only the direct child.
        ("sh", vec!["-c".into(), "sleep 300 & wait".into()])
    }
}
