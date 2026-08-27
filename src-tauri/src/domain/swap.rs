//! T259 — does this server need a swap file, and how big (FR-134)?
//!
//! The stand the owner set aside has 961 MB of memory and no swap, and that is not an
//! unusual VPS — it is the cheapest tier, which is what somebody buying their first server
//! buys. Installing packages is the peak of memory use in the whole deployment, and on such
//! a machine it is where the install is killed. FR-134 forbids leaving that to the person to
//! sort out by hand.
//!
//! **The numbers here are a choice, not a measurement**, and that is said plainly: nothing in
//! this project measured them, and the deployment skill never created swap at all. They may
//! be changed by an ordinary decision — unlike the ladder's constants, which may be changed
//! only by a new measurement (constitution, principle VI).

use serde::{Deserialize, Serialize};

/// What memory and swap together should come to, in megabytes.
///
/// Two gigabytes is enough for apt to unpack and configure the heaviest of what is installed
/// (ffmpeg pulls in a great many libraries) without being so much that it eats a small disk.
pub const TARGET_TOTAL_MB: u32 = 2048;

/// Swap files are made a whole number of these. A file of 1087 MB is a number nobody can read
/// and nobody chose.
pub const GRANULARITY_MB: u32 = 256;

/// The smallest swap file worth making. Below this the file costs more in disk than it gives
/// back in headroom.
pub const SMALLEST_MB: u32 = 512;

/// How much disk to leave alone besides the swap file itself.
///
/// The serving directory lives on the same disk, and a deployment that filled it to make room
/// for swap would trade one failure for a worse one — this one shows up later, when a video
/// is being sent.
pub const KEEP_FREE_MB: u32 = 1024;

/// What to do about swap on this server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Swap {
    /// There is enough already.
    NotNeeded,
    /// Make a file of this many megabytes.
    Make { megabytes: u32 },
    /// Needed, and there is nowhere to put it. Refused rather than attempted: a swap file
    /// that fills the disk leaves a server that installs nothing *and* serves nothing.
    NoRoom { wanted_mb: u32, free_mb: u32 },
}

/// Decide.
///
/// `memory_mb` and `swap_mb` are what the server reports about itself; `free_disk_mb` is what
/// is free where the file would go.
///
/// **A container answers this wrongly and it is not this function's fault** (T246): `free`
/// inside a container reports the *host's* swap, so a caller that reads it there will be told
/// there is plenty. That is why the step reports "cannot be established here" rather than
/// "already done" when it runs somewhere it cannot see the truth.
pub fn decide(memory_mb: u32, swap_mb: u32, free_disk_mb: u32) -> Swap {
    let have = memory_mb.saturating_add(swap_mb);
    if have >= TARGET_TOTAL_MB {
        return Swap::NotNeeded;
    }

    let short = TARGET_TOTAL_MB - have;
    // Rounded up: a file that leaves the total just short of the target would be all of the
    // cost and none of the point.
    let rounded = short.div_ceil(GRANULARITY_MB) * GRANULARITY_MB;
    let wanted = rounded.max(SMALLEST_MB);

    if free_disk_mb < wanted.saturating_add(KEEP_FREE_MB) {
        return Swap::NoRoom {
            wanted_mb: wanted,
            free_mb: free_disk_mb,
        };
    }
    Swap::Make { megabytes: wanted }
}
