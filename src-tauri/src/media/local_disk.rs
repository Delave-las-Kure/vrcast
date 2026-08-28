//! T452 — how much room is left on the disk we are about to write two gigabytes to.
//!
//! **The gap this fills.** The server's disk has been checked before an upload since T085 and
//! before a set since T409. The local one has never been asked at all — and it is the one that
//! fills first: a variant is one and a half to two gigabytes, and the build writes it in full
//! before sending a byte. Running out there ends a build hours in, with a message from the
//! operating system about a file it could not write.
//!
//! **The same arithmetic as the server's**, deliberately. `free_space::check` decides what
//! "enough" means — the margin, the floor under it, the naming of what is short. Two answers
//! to the same question would drift, and the day they disagreed nobody would know which to
//! believe.

use crate::commands::library::DiskUsage;

/// What the disk holding `path` has.
///
/// Returns `None` when it cannot be read. Not zero: zero is a disk that is full, which is a
/// refusal, and a refusal handed out because a question could not be asked is the worst of the
/// three outcomes — it stops work that would have succeeded, for a reason that is not true.
///
/// `used_by_videos_bytes` is nought here. It exists so the library screen can say how much of
/// a server's disk its own files take; on a local scratch folder nothing else is ours to
/// count, and the space check does not read it.
pub fn usage(path: &std::path::Path) -> Option<DiskUsage> {
    let (total, free) = raw(path)?;
    Some(DiskUsage {
        total_bytes: total,
        free_bytes: free,
        used_by_videos_bytes: 0,
    })
}

/// The nearest ancestor that exists.
///
/// The working folder is made when the build starts, so at the moment the question is asked it
/// is usually **not there yet** — and both platforms answer about a path, not about a path
/// that might be. Walking up finds the disk it is going to be on, which is what was being
/// asked all along.
fn existing_ancestor(path: &std::path::Path) -> Option<&std::path::Path> {
    let mut at = path;
    loop {
        if at.exists() {
            return Some(at);
        }
        at = at.parent()?;
    }
}

#[cfg(windows)]
fn raw(path: &std::path::Path) -> Option<(u64, u64)> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let at = existing_ancestor(path)?;
    let wide: Vec<u16> = at
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut free_to_us: u64 = 0;
    let mut total: u64 = 0;
    let mut free_in_all: u64 = 0;
    // SAFETY: `wide` is a null-terminated path that outlives the call, and the three outputs
    // are owned `u64`s written by the callee. A false return means nothing was written, and
    // that is the branch that returns `None`.
    let ok = unsafe {
        GetDiskFreeSpaceExW(wide.as_ptr(), &mut free_to_us, &mut total, &mut free_in_all)
    };
    if ok == 0 {
        return None;
    }
    // What is free **to us**, not what is free on the volume: with a disk quota in force the
    // second is a promise the filesystem will not keep, and this exists to stop a build that
    // cannot finish.
    Some((total, free_to_us))
}

#[cfg(target_os = "linux")]
fn raw(path: &std::path::Path) -> Option<(u64, u64)> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let at = existing_ancestor(path)?;
    let c_path = CString::new(at.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c_path` is a null-terminated path that outlives the call, and `stat` is an
    // owned, zeroed struct of the size the callee expects. A non-zero return means it wrote
    // nothing, and that is the branch that returns `None`.
    if unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) } != 0 {
        return None;
    }
    // `f_frsize` and not `f_bsize`: the block counts below are in fragments. They are equal on
    // every ordinary filesystem, which is exactly why using the wrong one is a fault that
    // never shows up until it does.
    let unit = if stat.f_frsize > 0 {
        stat.f_frsize as u64
    } else {
        stat.f_bsize as u64
    };
    // `f_bavail` and not `f_bfree`: the difference is the reserve only root may use, and we
    // are not root. Counting it would promise space this process cannot have.
    Some((stat.f_blocks as u64 * unit, stat.f_bavail as u64 * unit))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn raw(_path: &std::path::Path) -> Option<(u64, u64)> {
    // macOS is deferred (plan, milestone F). Answering `None` says "not known", which the
    // caller already handles by saying the check did not happen — never by refusing.
    None
}
