//! T018 — starting external programs in a way that lets them be killed for certain.
//!
//! Constitution, principle III (NOT NEGOTIABLE). This is not a precaution taken just in
//! case but a lesson the project wrote down: an orphaned `ffmpeg` goes on writing into the
//! same file and spoils it quietly — a person learns of it from their viewers, when it is
//! too late to fix.
//!
//! An ordinary `kill` by process id is not enough: `ffmpeg` and `ssh` spawn children of
//! their own, and killing the parent leaves those running. So every start gets **a group of
//! its own**, and the group is terminated whole:
//!
//! | OS | Mechanism | What it gives beyond an ordinary termination |
//! |---|---|---|
//! | Windows | a job object that kills on close | the children die **even when the application is killed from Task Manager**: the kernel closes the handle |
//! | Unix | a process group of its own, signalled as a whole | the signal reaches every child, not only the direct one |
//!
//! The Windows row is the crucial one. It covers the case where no application code is
//! running any more while the guarantee must hold all the same (SC-010).

use std::path::Path;
use std::process::Stdio;
use tokio::process::{Child, Command};

/// A short external program, started without a window of its own.
///
/// For the reads that finish in a moment and have nothing to kill: examining a source,
/// listing keyframes, asking FFmpeg what it can do. `ManagedProcess` below is for the work
/// that runs for hours and must die on command; putting these through it would mean a job
/// object and a process group for something that lives for eighty milliseconds.
///
/// **What it is for is the flag.** A released build on Windows has no console of its own,
/// so a child started without `CREATE_NO_WINDOW` is handed a brand new console window by
/// the system — which appears and vanishes. Seven places used to start programs directly
/// and every one of them flickered; choosing a single file made five of them appear in a
/// row. Nothing made anybody think about it, so `tests/unit/spawn_hygiene.rs` now does:
/// `Command::new` outside this file fails the build.
///
/// Standard input is closed as well. It was closed in two of those seven places and open in
/// five, for no reason anybody could name, and an open one is how a background program ends
/// up waiting forever for a keypress nobody will make.
pub fn quiet(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut cmd = Command::new(program);
    cmd.stdin(Stdio::null());

    #[cfg(windows)]
    {
        // No `CommandExt` import: tokio's own `Command` carries `creation_flags`, and the
        // std trait would only shadow it — which is why `spawn_in` below does not import it
        // either.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    cmd
}

#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("could not start {program}: {reason}")]
    Spawn { program: String, reason: String },

    #[error("could not kill the process tree: {0}")]
    Kill(String),

    #[error("suspending the process failed: {0}")]
    Suspend(String),
}

pub type Result<T> = std::result::Result<T, ProcessError>;

/// An external program started in a group of its own.
///
/// While this structure lives, the process tree is held; `kill_tree` terminates it whole.
pub struct ManagedProcess {
    /// Whether the program is frozen. Kept so that freezing it twice does not throw the
    /// counter off (see `suspend`).
    suspended: bool,
    child: Child,
    program: String,
    #[cfg(windows)]
    job: windows_job::Job,
}

impl ManagedProcess {
    /// Start a program in a process group of its own.
    ///
    /// **Do not call this from short-lived threads.** On Linux the protection rests on a
    /// signal the kernel sends when the parent *thread* dies, not the parent process. Spawn
    /// ffmpeg from a thread that ends soon after and ffmpeg dies along with it mid-work.
    /// The task runner's worker threads live for as long as the application does — calling
    /// from there is fine; from separate threads for blocking operations it is not.
    pub fn spawn(program: &str, args: &[String]) -> Result<Self> {
        Self::spawn_in(None, program, args)
    }

    /// The same, but started in a directory of our choosing.
    ///
    /// **Needed for one real reason**: a filter string is parsed by FFmpeg itself, and a
    /// Windows path inside one is not a path — the colon after the drive letter separates
    /// options and the backslashes escape whatever follows them. The measurement writes its
    /// results through such a string. Working in a directory and naming the file relatively
    /// sidesteps the whole question; the alternative is escaping by hand, which this
    /// project has already once got wrong quietly, writing the results nowhere.
    pub fn spawn_in(dir: Option<&Path>, program: &str, args: &[String]) -> Result<Self> {
        let mut cmd = Command::new(program);
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(dir) = dir {
            cmd.current_dir(dir);
        }

        #[cfg(unix)]
        {
            // The `std::os::unix::process::CommandExt` trait is NOT wanted here: tokio's
            // `Command` has a `pre_exec` of its own. The needless import compiles only
            // under Unix — under Windows this block is not built at all — and only a
            // continuous integration run on Linux caught it.
            //
            // The parent's id is remembered BEFORE spawning: the child will need it to
            // close the gap described below.
            let parent_pid = std::process::id();
            unsafe {
                cmd.pre_exec(move || {
                    // A process group of its own: a signal will reach every child, not
                    // only the direct one.
                    if libc_setsid() == -1 {
                        return Err(std::io::Error::last_os_error());
                    }

                    #[cfg(target_os = "linux")]
                    {
                        // The kernel will kill the child itself when the parent dies — and
                        // will do so even when no code of ours is running any more: the
                        // application killed by a signal, eaten by the out-of-memory killer
                        // or brought down by a panic. This is the only Linux equivalent of
                        // what a job object gives on Windows.
                        //
                        // Two limits worth remembering:
                        //   1. It applies only to the DIRECT child. Grandchildren are
                        //      covered by the start-up sweep (see tasks::registry).
                        //   2. It fires when the parent THREAD dies, not the process. So
                        //      processes must not be spawned from short-lived threads —
                        //      see the warning on ManagedProcess::spawn.
                        const PR_SET_PDEATHSIG: i32 = 1;
                        const SIGKILL: i64 = 9;
                        libc_prctl(PR_SET_PDEATHSIG, SIGKILL, 0, 0, 0);

                        // The gap: the parent may have died between the spawn and the line
                        // above — in which case the signal will never come. We check and
                        // leave of our own accord.
                        //
                        // Leaving must go through _exit: between fork and exec only
                        // async-signal-safe calls are allowed, while an ordinary exit runs
                        // atexit handlers and flushes the output streams — which can hang
                        // forever on a lock left behind by a thread of the parent, and the
                        // child would become an everlasting orphan, exactly what this
                        // guards against.
                        if libc_getppid() != parent_pid as i32 {
                            libc_exit_now(0);
                        }
                    }

                    Ok(())
                });
            }
        }

        #[cfg(windows)]
        {
            // CREATE_SUSPENDED is not used: the process must be in the job before it
            // starts working, but suspending it would complicate reading its streams. It is
            // enough that assignment to the job happens right after the start, before the
            // first read.
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
            cmd.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
        }

        let child = cmd.spawn().map_err(|e| ProcessError::Spawn {
            program: program.to_owned(),
            reason: e.to_string(),
        })?;

        #[cfg(windows)]
        let job = {
            let job = windows_job::Job::create().map_err(|e| ProcessError::Spawn {
                program: program.to_owned(),
                reason: format!("could not create the job object: {e}"),
            })?;
            if let Some(handle) = child.raw_handle() {
                job.assign(handle).map_err(|e| ProcessError::Spawn {
                    program: program.to_owned(),
                    reason: format!("could not assign the process to the job: {e}"),
                })?;
            }
            job
        };

        tracing::debug!(program, pid = ?child.id(), "external program started in its own group");

        Ok(Self {
            child,
            program: program.to_owned(),
            #[cfg(windows)]
            job,
            suspended: false,
        })
    }

    pub fn id(&self) -> Option<u32> {
        self.child.id()
    }

    pub fn program(&self) -> &str {
        &self.program
    }

    /// Take the output streams, to read the progress from them.
    pub fn take_output(
        &mut self,
    ) -> (
        Option<tokio::process::ChildStdout>,
        Option<tokio::process::ChildStderr>,
    ) {
        (self.child.stdout.take(), self.child.stderr.take())
    }

    /// Wait until it finishes.
    pub async fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        self.child.wait().await
    }

    /// Terminate the whole process tree.
    ///
    /// Hard and at once: the result of an interrupted task is thrown away regardless, and
    /// waiting for a polite exit is the very gap an orphaned process slips through.
    pub async fn kill_tree(&mut self) -> Result<()> {
        #[cfg(windows)]
        {
            // Terminates EVERY process in the job, grandchildren included.
            self.job
                .terminate()
                .map_err(|e| ProcessError::Kill(e.to_string()))?;
        }

        #[cfg(unix)]
        {
            if let Some(pid) = self.child.id() {
                // A negative id means the whole process group.
                unsafe {
                    libc_killpg(pid as i32, 9);
                }
            }
        }

        // The direct child is finished off in any case and waited for, so as to leave no
        // zombie behind.
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;

        tracing::debug!(program = %self.program, "the process tree was terminated");
        Ok(())
    }

    /// Pause the work without losing what has been done (FR-083a).
    ///
    /// Calling it again does nothing, and that is a necessity rather than a convenience: on
    /// Windows freezing a thread is counted, and two in a row need two resumes. A task
    /// paused twice would not come back to life after one "carry on" — and would look like
    /// one hung for good (T070).
    pub fn suspend(&mut self) -> Result<()> {
        if self.suspended {
            return Ok(());
        }
        self.signal_stop(true)?;
        self.suspended = true;
        Ok(())
    }

    /// Carry on paused work. Repeating is just as harmless.
    pub fn resume(&mut self) -> Result<()> {
        if !self.suspended {
            return Ok(());
        }
        self.signal_stop(false)?;
        self.suspended = false;
        Ok(())
    }

    /// Whether the program is frozen right now.
    pub fn is_suspended(&self) -> bool {
        self.suspended
    }

    #[cfg(unix)]
    fn signal_stop(&self, stop: bool) -> Result<()> {
        let Some(pid) = self.child.id() else {
            return Err(ProcessError::Suspend(String::from(
                "the process has already finished",
            )));
        };
        // SIGSTOP / SIGCONT to the whole group: the children have to be paused too.
        let sig = if stop { 19 } else { 18 };
        unsafe {
            libc_killpg(pid as i32, sig);
        }
        Ok(())
    }

    #[cfg(windows)]
    fn signal_stop(&self, stop: bool) -> Result<()> {
        let Some(pid) = self.child.id() else {
            return Err(ProcessError::Suspend(String::from(
                "the process has already finished",
            )));
        };
        windows_job::suspend_process(pid, stop).map_err(ProcessError::Suspend)
    }
}

#[cfg(unix)]
extern "C" {
    #[link_name = "setsid"]
    fn libc_setsid() -> i32;
    #[link_name = "killpg"]
    fn libc_killpg(pgrp: i32, sig: i32) -> i32;
    #[link_name = "kill"]
    fn libc_kill(pid: i32, sig: i32) -> i32;
    #[cfg(target_os = "linux")]
    #[link_name = "getppid"]
    fn libc_getppid() -> i32;
    #[cfg(target_os = "linux")]
    #[link_name = "prctl"]
    fn libc_prctl(option: i32, a2: i64, a3: i64, a4: i64, a5: i64) -> i32;
    /// An immediate exit with no atexit handlers — the only safe way to end between fork
    /// and exec.
    #[cfg(target_os = "linux")]
    #[link_name = "_exit"]
    fn libc_exit_now(status: i32) -> !;
}

/// Terminate a process by its id — for the start-up sweep of survivors.
///
/// Kept apart from [`ManagedProcess::kill_tree`]: there the process is ours and alive, here
/// it is a stranger to the current run of the application, left over from the previous
/// one.
pub(crate) fn kill_pid(pid: u32) -> bool {
    #[cfg(unix)]
    {
        unsafe { libc_kill(pid as i32, 9) == 0 }
    }
    #[cfg(windows)]
    {
        windows_job::terminate_pid(pid)
    }
}

/// The name of a process's executable, if it is still alive.
///
/// Needed not to make a report look good but for the safety of the sweep: process ids are
/// reused, and by the next start-up a completely unrelated program may stand behind an old
/// number. Killing it is not allowed.
pub(crate) fn process_name(pid: u32) -> Option<String> {
    #[cfg(unix)]
    {
        std::fs::read_to_string(format!("/proc/{pid}/comm"))
            .ok()
            .map(|s| s.trim().to_owned())
    }
    #[cfg(windows)]
    {
        windows_job::process_name(pid)
    }
}

/// Check a process by its start time and terminate it through ONE handle.
///
/// Windows only: there a handle opened on a live process holds its number, so the check and
/// the termination certainly refer to one and the same process. On Unix the number is held
/// by the process staying a zombie until the parent reaps it, and separate steps are not
/// dangerous there.
///
/// `Some(true)` — terminated; `Some(false)` — this is another process already, left
/// untouched; `None` — there is no such process, or it cannot be reached.
#[cfg(windows)]
pub(crate) fn verify_and_terminate(pid: u32, expected_created_at: &str) -> Option<bool> {
    windows_job::verify_and_terminate(pid, expected_created_at)
}

/// A process's identifying mark — the time it started.
///
/// Needed by the start-up sweep: before finishing off a surviving program we must be sure
/// no stranger stands behind its number. Numbers are reused, and killing someone else's
/// costs more than failing to finish off our own.
///
/// **Why not the name.** The name lies, as a live example showed: `sh -c "sleep"` replaces
/// itself with `sleep`, and the recorded name stops matching. On top of that
/// `/proc/<pid>/comm` is truncated at fifteen characters, so a program with a long name
/// will not match itself. A start time does not change when the image is replaced, is not
/// truncated, and tells two processes with one number apart exactly.
///
/// `None` means "it could not be learned" — the name comparison is then all that is left,
/// which is worse, but better than nothing.
pub(crate) fn process_identity(pid: u32) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        // In /proc/<pid>/stat the twenty-second field is the start time in ticks since the
        // system booted. Parsing is complicated by the second field: the program's name in
        // brackets, which can hold both spaces and brackets of its own. So counting starts
        // from the LAST closing bracket rather than splitting the line on spaces from the
        // beginning.
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let after_name = &stat[stat.rfind(')')? + 1..];
        let field = after_name.split_whitespace().nth(19)?;
        Some(field.to_owned())
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        // On other Unixes there is no reliable way without extra dependencies.
        let _ = pid;
        None
    }
    #[cfg(windows)]
    {
        windows_job::process_created_at(pid)
    }
}

#[cfg(windows)]
mod windows_job {
    //! A minimal wrapper over the Windows job object.
    //!
    //! No separate dependency is taken: four calls are needed, and every extra package in
    //! the tree is one more licence to check (the application is under the GPL).

    use std::ffi::c_void;

    type Handle = *mut c_void;

    #[repr(C)]
    #[derive(Default)]
    struct IoCounters {
        read_ops: u64,
        write_ops: u64,
        other_ops: u64,
        read_bytes: u64,
        write_bytes: u64,
        other_bytes: u64,
    }

    #[repr(C)]
    #[derive(Default)]
    struct BasicLimitInformation {
        per_process_user_time: i64,
        per_job_user_time: i64,
        limit_flags: u32,
        min_working_set: usize,
        max_working_set: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct ExtendedLimitInformation {
        basic: BasicLimitInformation,
        io: IoCounters,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }

    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x2000;
    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: u32 = 9;
    const THREAD_SUSPEND_RESUME: u32 = 0x0002;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateJobObjectW(attrs: *mut c_void, name: *const u16) -> Handle;
        fn SetInformationJobObject(job: Handle, class: u32, info: *mut c_void, len: u32) -> i32;
        fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
        fn TerminateJobObject(job: Handle, exit_code: u32) -> i32;
        fn CloseHandle(h: Handle) -> i32;
        fn OpenThread(access: u32, inherit: i32, thread_id: u32) -> Handle;
        fn SuspendThread(thread: Handle) -> u32;
        fn ResumeThread(thread: Handle) -> u32;
        fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> Handle;
        fn Thread32First(snapshot: Handle, entry: *mut ThreadEntry32) -> i32;
        fn Thread32Next(snapshot: Handle, entry: *mut ThreadEntry32) -> i32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        fn TerminateProcess(process: Handle, exit_code: u32) -> i32;
        fn QueryFullProcessImageNameW(
            process: Handle,
            flags: u32,
            buf: *mut u16,
            size: *mut u32,
        ) -> i32;
        fn GetProcessTimes(
            process: Handle,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
    }

    /// A time in the system's own form: two thirty-two-bit words.
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct ThreadEntry32 {
        size: u32,
        usage: u32,
        thread_id: u32,
        owner_process_id: u32,
        base_priority: i32,
        delta_priority: i32,
        flags: u32,
    }

    pub struct Job(Handle);

    // The job handle belongs to a single owner and moves between threads along with it.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    impl Job {
        pub fn create() -> Result<Self, String> {
            unsafe {
                let job = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
                if job.is_null() {
                    return Err(std::io::Error::last_os_error().to_string());
                }

                // This is the line the whole thing was for: when the job handle is closed
                // — and the kernel closes it when the owning process dies, a kill from Task
                // Manager included — every process in the job is terminated with it.
                let mut info = ExtendedLimitInformation {
                    basic: BasicLimitInformation {
                        limit_flags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                        ..Default::default()
                    },
                    ..Default::default()
                };

                let ok = SetInformationJobObject(
                    job,
                    JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
                    &mut info as *mut _ as *mut c_void,
                    std::mem::size_of::<ExtendedLimitInformation>() as u32,
                );
                if ok == 0 {
                    let e = std::io::Error::last_os_error().to_string();
                    CloseHandle(job);
                    return Err(e);
                }
                Ok(Self(job))
            }
        }

        pub fn assign(&self, process: Handle) -> Result<(), String> {
            unsafe {
                if AssignProcessToJobObject(self.0, process) == 0 {
                    return Err(std::io::Error::last_os_error().to_string());
                }
            }
            Ok(())
        }

        pub fn terminate(&self) -> Result<(), String> {
            unsafe {
                if TerminateJobObject(self.0, 1) == 0 {
                    return Err(std::io::Error::last_os_error().to_string());
                }
            }
            Ok(())
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    /// Terminate a process by its id. Used by the start-up sweep of survivors.
    pub fn terminate_pid(pid: u32) -> bool {
        const PROCESS_TERMINATE: u32 = 0x0001;
        unsafe {
            let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if h.is_null() {
                return false;
            }
            let ok = TerminateProcess(h, 1) != 0;
            CloseHandle(h);
            ok
        }
    }

    /// Make sure this is the very process meant, and terminate it — through ONE handle.
    ///
    /// Checking and killing by number separately will not do, and that is not theory:
    /// between the check and the kill the system is free to hand the freed number to
    /// another program, and that is the one that would be killed. Numbers are reused
    /// quickly, and the sweep runs at the application's start-up — exactly when the system
    /// hands numbers out in batches (debt T074).
    ///
    /// A handle opened on a live process holds its number: while it is open, that number
    /// goes to nobody else. So the start-time check and the termination certainly refer to
    /// one and the same process.
    ///
    /// `Some(true)` — terminated; `Some(false)` — this is another process, left untouched;
    /// `None` — there is no such process any more, or it cannot be reached.
    pub fn verify_and_terminate(pid: u32, expected_created_at: &str) -> Option<bool> {
        const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
        const PROCESS_TERMINATE: u32 = 0x0001;
        unsafe {
            let h = OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE,
                0,
                pid,
            );
            if h.is_null() {
                return None;
            }

            let mut creation = FileTime::default();
            let mut ignored = FileTime::default();
            let ok = GetProcessTimes(h, &mut creation, &mut ignored, &mut ignored, &mut ignored);
            if ok == 0 {
                CloseHandle(h);
                return None;
            }
            let actual = format!(
                "{}",
                (u64::from(creation.high) << 32) | u64::from(creation.low)
            );

            if actual != expected_created_at {
                CloseHandle(h);
                return Some(false);
            }

            let killed = TerminateProcess(h, 1) != 0;
            CloseHandle(h);
            Some(killed)
        }
    }

    /// The time a process started — an exact identifying mark.
    ///
    /// Process numbers are reused, while a new process with the same number will have a
    /// different start time. That is enough to avoid killing someone else's.
    pub fn process_created_at(pid: u32) -> Option<String> {
        const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return None;
            }
            let mut creation = FileTime::default();
            let mut ignored = FileTime::default();
            let ok = GetProcessTimes(h, &mut creation, &mut ignored, &mut ignored, &mut ignored);
            CloseHandle(h);
            if ok == 0 {
                return None;
            }
            Some(format!(
                "{}",
                (u64::from(creation.high) << 32) | u64::from(creation.low)
            ))
        }
    }

    pub fn process_name(pid: u32) -> Option<String> {
        const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return None;
            }
            let mut buf = [0u16; 260];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len);
            CloseHandle(h);
            if ok == 0 {
                return None;
            }
            let full = String::from_utf16_lossy(&buf[..len as usize]);
            // The path is parsed by the standard means: they know both OSes' separators.
            std::path::Path::new(&full)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .or(Some(full))
        }
    }

    /// Freeze or release every thread of a process.
    ///
    /// Windows has no means of suspending a whole process in one call — only thread by
    /// thread.
    ///
    /// The snapshot of threads is taken SEVERAL TIMES, until no new ones turn up. One
    /// snapshot left a gap: a thread created between taking the list and freezing it went
    /// on working, and the "paused" program went on writing into the file (debt T070).
    /// Resuming needs no repetition — there is nothing to release but what is already
    /// frozen.
    pub fn suspend_process(pid: u32, stop: bool) -> Result<(), String> {
        let mut known: Vec<u32> = Vec::new();
        let mut rounds = 0;

        loop {
            let touched = touch_threads(pid, stop, &mut known)?;
            rounds += 1;
            // Not one new thread — the gap is closed. Three rounds as a limit in case of a
            // program that spawns threads without stopping: better to leave than to stand
            // here forever.
            if !stop || touched == 0 || rounds >= 3 {
                break;
            }
        }

        if known.is_empty() {
            return Err(String::from("not one thread of the process was found"));
        }
        Ok(())
    }

    /// One pass over a process's threads. Returns how many NEW ones were found.
    fn touch_threads(pid: u32, stop: bool, known: &mut Vec<u32>) -> Result<usize, String> {
        const TH32CS_SNAPTHREAD: u32 = 0x0000_0004;
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
            if snapshot as isize == -1 {
                return Err(std::io::Error::last_os_error().to_string());
            }

            let mut entry = ThreadEntry32 {
                size: std::mem::size_of::<ThreadEntry32>() as u32,
                ..Default::default()
            };

            let mut fresh = 0;
            let mut ok = Thread32First(snapshot, &mut entry);
            while ok != 0 {
                if entry.owner_process_id == pid && !known.contains(&entry.thread_id) {
                    let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.thread_id);
                    if !thread.is_null() {
                        if stop {
                            SuspendThread(thread);
                        } else {
                            ResumeThread(thread);
                        }
                        CloseHandle(thread);
                        known.push(entry.thread_id);
                        fresh += 1;
                    }
                }
                entry.size = std::mem::size_of::<ThreadEntry32>() as u32;
                ok = Thread32Next(snapshot, &mut entry);
            }
            CloseHandle(snapshot);
            Ok(fresh)
        }
    }
}
