//! T078 — capping the sending speed (FR-034).
//!
//! What it is for: an upload of tens of gigabytes fills the outgoing channel entirely,
//! and everything else on the computer stops working — to the point where a person
//! cannot watch how their own upload is going.
//!
//! The cap sits **on the sending side** rather than being a request to the server:
//! there is nobody to ask, the channel is filled at our end. The mechanism is a
//! counter of allowed bytes that refills over time; when there are not enough, sending
//! waits.
//!
//! Time is passed in rather than read inside. Not for purity: otherwise the limiter
//! could not be tested without spending as many real seconds as the stretch under
//! test lasts.

use std::time::{Duration, Instant};

/// How much the limiter allows to burst after a lull.
///
/// Without an allowance every window would wait its turn from zero and the transfer
/// would proceed in jerks, strictly by timetable. With a second's allowance a short
/// pause does not turn into lost speed, and the average still holds to the cap.
const BURST_SECONDS: u64 = 1;

/// The speed limiter.
///
/// A cap of `None` means no cap at all — and then the wait is always zero.
#[derive(Debug)]
pub struct RateLimiter {
    limit_bps: Option<u64>,
    /// How many bytes may be sent right now.
    allowance: f64,
    last: Option<Instant>,
}

impl RateLimiter {
    pub fn new(limit_bps: Option<u64>) -> Self {
        Self {
            // Zero as a cap is meaningless: it would mean never transferring at all.
            // It is taken as no cap, the same as no value.
            limit_bps: limit_bps.filter(|v| *v > 0),
            allowance: 0.0,
            last: None,
        }
    }

    pub fn limit_bps(&self) -> Option<u64> {
        self.limit_bps
    }

    /// How long to wait before sending `bytes` bytes.
    ///
    /// Called **before** sending, and it returns the delay; the limiter itself does
    /// not sleep — the caller decides to, and the caller can break off on a cancel.
    pub fn delay_for(&mut self, bytes: u64, now: Instant) -> Duration {
        let Some(limit) = self.limit_bps else {
            return Duration::ZERO;
        };
        let rate = limit as f64;

        // The first call gets the full allowance, or the transfer would start by waiting.
        let elapsed = match self.last {
            Some(last) => now.saturating_duration_since(last).as_secs_f64(),
            None => BURST_SECONDS as f64,
        };
        self.last = Some(now);

        self.allowance = (self.allowance + elapsed * rate).min(rate * BURST_SECONDS as f64);

        let need = bytes as f64;
        if self.allowance >= need {
            self.allowance -= need;
            return Duration::ZERO;
        }

        let missing = need - self.allowance;
        self.allowance = 0.0;
        let wait = missing / rate;
        // The wait moves the clock forward: the next call must not count those
        // seconds as freshly earned allowance.
        self.last = Some(now + Duration::from_secs_f64(wait));
        Duration::from_secs_f64(wait)
    }

    /// Forget what has accumulated. Needed after a long pause: the allowance earned
    /// while idle would otherwise burst out all at once.
    pub fn reset(&mut self) {
        self.allowance = 0.0;
        self.last = None;
    }
}
