//! T450 — where a variant is written while it is being made.
//!
//! **The defect, not the preference.** The owner asked to be able to change where the
//! working files go. What was there was worse than inflexible: `std::env::temp_dir()`, which
//! on Windows is on `C:` and on Linux is often a tmpfs sized from RAM. A variant is one and a
//! half to two gigabytes; they are made one at a time and swept after sending, so only one
//! lives at once — and on a machine with a small system disk that one is still enough to end
//! a build hours in, for space nobody agreed to spend. On a tmpfs it is worse: the "disk" is
//! memory, and filling it takes the whole machine down with it.
//!
//! **The default is beside the source, and this overrules a comment that said otherwise.**
//! The code used to argue: "beside the other working files rather than beside the source: a
//! person's film directory is theirs, and a half-made variant appearing in it is alarming."
//! That reads well and does not survive the arithmetic. The disk a film is on certainly fits
//! a film — no other default can promise that, on any platform — and the preparation step
//! already writes `.ready.mp4` beside the source, so the film's directory is not untouched
//! ground. Against a build that fails after four hours, a folder appearing and going again
//! is the smaller surprise. It is named plainly and swept, and anybody who disagrees moves
//! it: that is what the setting is for.

use std::path::{Path, PathBuf};

/// The folder made beside a source when nowhere else is chosen.
///
/// Named for the application and not hidden. A dotted name would be invisible on Linux and
/// merely odd on Windows, and something taking up two gigabytes should be findable by
/// somebody wondering where their space went.
pub const BESIDE_THE_SOURCE: &str = "vrcast-work";

/// Where to write while making a variant of `source`.
///
/// `chosen` is the person's setting: `None` means they have not chosen, and the folder goes
/// beside the source. A chosen path is used as given — it is theirs, and second-guessing it
/// would make the setting a suggestion.
///
/// A source with no parent directory — a bare file name, or a root — falls back to the
/// working directory's own folder rather than to the system's temporary one. Not the system's
/// temporary: that is the fault this exists to fix, and reintroducing it in the corner case
/// would make the fix hold everywhere except where paths are strange.
pub fn for_source(chosen: Option<&str>, source: &Path) -> PathBuf {
    if let Some(chosen) = chosen.map(str::trim).filter(|c| !c.is_empty()) {
        return PathBuf::from(chosen);
    }
    match source.parent().filter(|p| !p.as_os_str().is_empty()) {
        Some(beside) => beside.join(BESIDE_THE_SOURCE),
        None => PathBuf::from(BESIDE_THE_SOURCE),
    }
}

/// Whether this path is the system's temporary directory, or inside it.
///
/// Used by the check that keeps the default honest, and by the settings screen, which has to
/// say what choosing it costs rather than silently accepting the very thing T450 removed.
pub fn is_system_temp(path: &Path) -> bool {
    let temp = std::env::temp_dir();
    path == temp || path.starts_with(&temp)
}

/// What is still lying in a working folder.
///
/// **Why this is asked at all** (T453). Working files are swept after a variant is sent, so
/// the folder is normally empty — but a build that was killed leaves one behind, and it is
/// one and a half to two gigabytes. Changing the setting then points the application
/// somewhere new and leaves those where they were, under a path nobody is looking at any
/// more. That is the same fault T450 removes, except caused by us rather than inherited.
///
/// **Said, not moved.** Moving gigabytes between disks takes minutes and would happen inside
/// a click on a settings screen, with nothing to watch and no way to stop it. What a person
/// needs is to be told where their space went; deleting it is their call and their file
/// manager's job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Leftovers {
    pub files: u64,
    pub bytes: u64,
}

/// Add up what a working folder still holds, one level down and no further.
///
/// One level because that is all this folder ever has: a variant is written into it directly.
/// Walking deeper would mean walking a directory somebody may have pointed at their whole
/// film library, which is a lot of disk to read for an aside on a settings screen.
///
/// A folder that is not there, or will not open, is nothing rather than an error: this is an
/// aside, and it must not be able to stop a person changing a setting.
pub fn leftovers_in(path: &Path) -> Leftovers {
    let Ok(entries) = std::fs::read_dir(path) else {
        return Leftovers { files: 0, bytes: 0 };
    };
    let mut found = Leftovers { files: 0, bytes: 0 };
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_file() {
            found.files += 1;
            found.bytes += meta.len();
        }
    }
    found
}
