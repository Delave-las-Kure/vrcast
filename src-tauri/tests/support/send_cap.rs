//! T717 — the one «Send speed» held for the length of a check.
//!
//! An upload had a cap of its own (`UploadRequest.limit_bps`) and the checks that need a
//! transfer to take a tangible time slowed it with that. It is gone: every send shares
//! `Settings.send_limit_bps` ([`SENDING`]). That cap is one for the process — and every
//! integration check runs in one process, one after another — so a cap left behind would
//! slow every check after it. Held through this guard, it is lifted when the guard goes,
//! a failed assertion included.

use vrcast_studio_lib::domain::rate_limit::SENDING;

#[must_use = "the cap is lifted as soon as the guard is dropped"]
pub struct SendCap(());

impl SendCap {
    /// Hold every send to `bytes_per_s` until the guard is dropped.
    pub fn hold(bytes_per_s: u64) -> Self {
        SENDING.set(Some(bytes_per_s));
        Self(())
    }
}

impl Drop for SendCap {
    fn drop(&mut self) {
        SENDING.set(None);
    }
}
