//! T077 — where to resume an interrupted transfer (R-05, FR-031).
//!
//! The position comes not from a local record but from the size of the staged file
//! **on the server**: a local record may be stale — the application was killed between
//! sending a window and writing to the database, and then it points earlier than the
//! truth. Worse if it points later: then a hole is left in the file, and only the
//! checksum comparison notices — after the whole transfer has finished.
//!
//! Stepping back one window before resuming is not optional: the last write may have
//! broken off midway, and its tail is in the file already without being whole.
//! Rewriting that window is cheaper than guessing.

use serde::{Deserialize, Serialize};

/// The size of a transfer window.
///
/// Four megabytes is a compromise: small windows spend a network round trip on every
/// acknowledgement, large ones increase what has to be rewritten after a break.
pub const WINDOW_BYTES: u64 = 4 * 1024 * 1024;

/// What to do with an interrupted transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeDecision {
    /// Nothing was sent — start from the beginning.
    FromStart,
    /// Resume from this offset.
    Continue { offset: u64 },
    /// Everything was sent; the checksum comparison and entering service remain.
    AlreadyComplete,
    /// The staged file is **larger** than the source. That is not "nearly done" but a
    /// sign that the source was swapped or the file on the server is not the right
    /// one. Resuming is out: it would splice two different files together, and only
    /// the comparison would find out, by which time the time has been spent.
    Mismatch { temp: u64, total: u64 },
}

/// Decide where to resume from, by the size of the staged file on the server.
pub fn decide_resume(temp_size: u64, total: u64, window: u64) -> ResumeDecision {
    if total == 0 {
        // An empty source is a legitimate case: there is nothing to send.
        return ResumeDecision::AlreadyComplete;
    }
    if temp_size > total {
        return ResumeDecision::Mismatch {
            temp: temp_size,
            total,
        };
    }
    if temp_size == total {
        return ResumeDecision::AlreadyComplete;
    }
    if temp_size == 0 {
        return ResumeDecision::FromStart;
    }

    // A step back of one window, in case the last write broke off. Never below zero.
    let offset = temp_size.saturating_sub(window.max(1));
    if offset == 0 {
        ResumeDecision::FromStart
    } else {
        ResumeDecision::Continue { offset }
    }
}

/// Everything needed to resume a transfer after the application restarts.
///
/// Kept in the task's `resume_token` field. The task record knows the id, the state
/// and the server; the rest is here: where to take the bytes from, where to put them,
/// and how to be sure the source is the same one.
///
/// The source's size and modification time are not decoration: if a person swapped the
/// local file between runs, the old transfer must not continue — it would come out a
/// mixture of two files. Noticing that at once is cheaper than noticing it at the
/// checksum comparison after an hour of transfer.
///
/// The fields marked `serde(default)` were added later: records written by earlier
/// versions have to keep reading, or an update quietly loses unfinished transfers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeToken {
    /// The full path of the staged file on the server.
    pub remote_temp: String,
    /// The final name in the serving directory.
    pub remote_name: String,
    /// The path to the source on this computer.
    #[serde(default)]
    pub local_path: Option<String>,
    /// Which medium to assign the file to once it is in service.
    ///
    /// Without this, an upload resumed after a restart would drop the file into "not
    /// recognised" even though the person had already said where it belongs.
    #[serde(default)]
    pub media_id: Option<String>,
    /// The speed cap the person set.
    ///
    /// It survives a restart too: a cap is set so that an upload does not eat the
    /// connection, and quietly lifting it on resume would take the whole channel at a
    /// moment nobody expects it.
    #[serde(default)]
    pub limit_bps: Option<u64>,
    /// The size of the source when the transfer began.
    pub source_size: u64,
    /// The source's modification time, if it could be found out.
    pub source_modified: Option<String>,
}

impl ResumeToken {
    pub fn parse(json: &str) -> Option<Self> {
        serde_json::from_str(json).ok()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Whether this is the source we started with.
    ///
    /// A matching size is not enough: a file can be rebuilt to the same size. So the
    /// modification time is compared too — when it is known.
    pub fn matches_source(&self, size: u64, modified: Option<&str>) -> bool {
        if self.source_size != size {
            return false;
        }
        match (&self.source_modified, modified) {
            (Some(was), Some(now)) => was == now,
            // The time is unknown on one side — the size will have to do.
            _ => true,
        }
    }
}
