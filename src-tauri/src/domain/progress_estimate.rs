//! T079 — speed and time remaining (FR-035).
//!
//! Instantaneous speed cannot be shown: it jumps from window to window, and the number
//! flickers so badly it cannot be read. Nor can the average over all time: after a
//! break and half an hour idle it shows half of what is really happening.
//!
//! So speed is worked out over a sliding window of the last few seconds — and never over
//! fewer than two samples, so there is always something to measure against.
//!
//! **A pause is said, not guessed** (T659, QA-24A №10). What was gathered before a pause, a
//! break or a restart no longer describes what is happening and is thrown away — but the
//! transfer says when that happened, by calling [`ProgressEstimate::reset`] (and then
//! [`ProgressEstimate::record`] the point it starts from again). The estimate used to guess
//! it instead: any two samples further apart than the averaging window counted as a pause.
//! A sample is taken once per window of the file, and on a slow or capped link one window
//! takes longer than that — four megabytes at two megabits is sixteen seconds — so every
//! sample looked like the first after a pause, and the speed and time left never appeared at
//! all. A gap is still taken for a stall when it is longer than any one window can take
//! ([`LONGEST_WINDOW`]): by then the transfer has given the connection up anyway.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// The stretch the average is taken over.
const WINDOW: Duration = Duration::from_secs(10);

/// The longest one window of a transfer may take before the connection is presumed gone
/// (`server::upload::write_window` gives up on it then). Two samples further apart than this
/// cannot be one slow window, and are not measured across.
pub const LONGEST_WINDOW: Duration = Duration::from_secs(600);

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
        // Longer than any window can take: not a slow link but a stall the transfer did not
        // report. What was accumulated before it says nothing about the speed now.
        if let Some((last, _)) = self.samples.back() {
            if now.saturating_duration_since(*last) > LONGEST_WINDOW.max(self.window) {
                self.samples.clear();
            }
        }

        self.samples.push_back((now, transferred));

        // Everything older than the window goes — but the last two samples always stay:
        // on a slow link one window of the file takes longer than the averaging window,
        // and the speed is then the speed of that last window rather than nothing.
        while self.samples.len() > 2 {
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
    ///
    /// The one way a pause reaches the estimate (T659). Follow it with a `record` of where
    /// the transfer starts again, so the first window after it already gives a speed.
    pub fn reset(&mut self) {
        self.samples.clear();
    }
}
