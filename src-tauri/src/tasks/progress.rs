//! T020 — capping how often progress events are sent.
//!
//! Transferring a file reports progress hundreds of times a second. Letting all of it
//! through would make the stream of events the cause of the interface stuttering — the
//! very means of showing responsiveness would destroy it (SC-009, R-15).
//!
//! The subtlety is not in the cap but in the exceptions to it. The last message before
//! completion must always get through, or the bar freezes at 87% on a task that has
//! already finished.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// No more than four times a second per task.
pub const MIN_INTERVAL: Duration = Duration::from_millis(250);

/// The valve that progress events pass through.
#[derive(Debug)]
pub struct ProgressThrottle {
    min_interval: Duration,
    last: Mutex<Option<Instant>>,
}

impl Default for ProgressThrottle {
    fn default() -> Self {
        Self::new(MIN_INTERVAL)
    }
}

impl ProgressThrottle {
    pub fn new(min_interval: Duration) -> Self {
        Self {
            min_interval,
            last: Mutex::new(None),
        }
    }

    /// Whether to let this event through.
    ///
    /// `important` marks a message that must pass regardless of the rate: a change of
    /// state, a completion, an error. Without that exception the figure sticks at the
    /// last value that happened to get through.
    pub fn allow(&self, important: bool) -> bool {
        self.allow_at(Instant::now(), important)
    }

    /// The same, with an explicit instant — so tests do not depend on a real clock.
    pub fn allow_at(&self, now: Instant, important: bool) -> bool {
        let mut last = match self.last.lock() {
            Ok(l) => l,
            // A poisoned lock is no reason to lose an event.
            Err(e) => e.into_inner(),
        };

        if important {
            *last = Some(now);
            return true;
        }

        match *last {
            Some(prev) if now.duration_since(prev) < self.min_interval => false,
            _ => {
                *last = Some(now);
                true
            }
        }
    }

    /// Forget the mark — when a task resumes after being paused, for instance.
    pub fn reset(&self) {
        if let Ok(mut last) = self.last.lock() {
            *last = None;
        }
    }
}
