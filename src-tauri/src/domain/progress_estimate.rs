//! T079 — speed and time remaining (FR-035).
//!
//! Instantaneous speed cannot be shown: it jumps from window to window, and the number
//! flickers so badly it cannot be read. Nor can the average over all time: after a
//! break and half an hour idle it shows half of what is really happening.
//!
//! So speed is worked out over a sliding window of the last few seconds. And there is
//! a separate rule about pauses: if more than a window has passed between two samples,
//! what was accumulated no longer describes what is happening and is thrown away.
//! Without that rule a person sees "four hundred hours left" after a pause and decides
//! everything is broken.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// The stretch the average is taken over.
const WINDOW: Duration = Duration::from_secs(10);

/// How many samples are kept. No more is needed: at four events a second (R-15) that
/// many will not accumulate within the averaging window anyway.
const MAX_SAMPLES: usize = 64;

#[derive(Debug)]
pub struct ProgressEstimate {
    /// Pairs of "when" and "how much has been sent in all".
    samples: VecDeque<(Instant, u64)>,
    window: Duration,
}

impl Default for ProgressEstimate {
    fn default() -> Self {
        Self::new(WINDOW)
    }
}

impl ProgressEstimate {
    pub fn new(window: Duration) -> Self {
        Self {
            samples: VecDeque::new(),
            window,
        }
    }

    /// Record how much has been sent in all by this moment.
    pub fn record(&mut self, now: Instant, transferred: u64) {
        // A gap longer than the averaging window means a pause, a break or a
        // restart. What was accumulated before it says nothing about the speed now.
        if let Some((last, _)) = self.samples.back() {
            if now.saturating_duration_since(*last) > self.window {
                self.samples.clear();
            }
        }

        self.samples.push_back((now, transferred));

        // Everything older than the window goes, but the last sample is kept: without
        // it there is nothing to compare the next one against.
        while self.samples.len() > 1 {
            let Some((oldest, _)) = self.samples.front() else {
                break;
            };
            if now.saturating_duration_since(*oldest) > self.window
                || self.samples.len() > MAX_SAMPLES
            {
                self.samples.pop_front();
            } else {
                break;
            }
        }
    }

    /// Speed in bytes per second. `None` while there are too few samples to say.
    pub fn speed_bps(&self) -> Option<u64> {
        let (first_at, first_bytes) = *self.samples.front()?;
        let (last_at, last_bytes) = *self.samples.back()?;

        let seconds = last_at.saturating_duration_since(first_at).as_secs_f64();
        // Too short a stretch gives a number not worth believing: dividing by
        // thousandths of a second turns any jitter into gigabits.
        if seconds < 0.5 {
            return None;
        }
        let bytes = last_bytes.saturating_sub(first_bytes);
        Some((bytes as f64 / seconds).round() as u64)
    }

    /// How long is left at the present speed. `None` if the speed is unknown or zero
    /// — there is no point showing a person infinity.
    pub fn eta(&self, remaining: u64) -> Option<Duration> {
        let speed = self.speed_bps()?;
        if speed == 0 {
            return None;
        }
        Some(Duration::from_secs_f64(remaining as f64 / speed as f64))
    }

    /// Forget what was accumulated — on a pause, a break, or resuming after a restart.
    pub fn reset(&mut self) {
        self.samples.clear();
    }
}
