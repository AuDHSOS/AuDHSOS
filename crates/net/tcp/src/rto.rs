// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The retransmission timer of RFC 6298: how long to wait for an
//! acknowledgment before sending again.
//!
//! Two numbers are kept, a smoothed round trip time and its variation,
//! and the timeout is the first plus four times the second. What that
//! buys over a plain average is the reason the memo gives: a path whose
//! delay is steady gets a tight timeout and one whose delay jumps gets a
//! loose one, so a retransmission is neither early on a jittery path nor
//! late on a steady one.
//!
//! Three rules bound it. The timeout is never below one second, which is
//! section 2.4 of the memo, and never above sixty, which is the maximum
//! it permits. It doubles on every attempt that goes unanswered, which is
//! section 5.5, and the doubling is thrown away as soon as a measurement
//! arrives, because a measurement is evidence and a backoff is a guess.
//! And Karn's rule: a segment that was sent more than once yields no
//! measurement at all, because there is no telling which of the copies
//! the acknowledgment answers, and taking the wrong one would shorten the
//! timeout exactly when the path is losing segments.
//!
//! The arithmetic is integer arithmetic over microseconds. The two
//! weights of the memo are `1/8` for the smoothed time and `1/4` for the
//! variation, both powers of two, so nothing here needs a division that
//! is not a shift.

use audhsos_time::Duration;

/// What the timeout is before a single round trip has been measured
/// (RFC 6298, section 2.1).
pub const INITIAL: Duration = Duration::from_secs(1);

/// The floor of section 2.4. A shorter timeout would retransmit into a
/// path that is merely slow.
pub const MINIMUM: Duration = Duration::from_secs(1);

/// The ceiling. The memo permits any maximum of at least sixty seconds.
pub const MAXIMUM: Duration = Duration::from_secs(60);

/// How many times the variation is counted towards the timeout: `K` of
/// the memo.
const VARIATION_WEIGHT: u64 = 4;

/// The estimate a connection waits by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rto {
    /// `SRTT`, once a round trip has been measured.
    smoothed: Option<Duration>,
    /// `RTTVAR`.
    variation: Duration,
    /// The timeout the measurements yield, before any backoff.
    base: Duration,
    /// How often the timeout has doubled without an answer.
    attempts: u32,
}

impl Default for Rto {
    fn default() -> Rto {
        Rto::new()
    }
}

impl Rto {
    /// An estimate that has measured nothing yet.
    #[must_use]
    pub const fn new() -> Rto {
        Rto {
            smoothed: None,
            variation: Duration::from_micros(0),
            base: INITIAL,
            attempts: 0,
        }
    }

    /// The timeout in force: the measured one, doubled once per attempt
    /// that went unanswered, and never above the ceiling.
    #[must_use]
    pub fn get(&self) -> Duration {
        let micros = self
            .base
            .as_micros()
            .checked_shl(self.attempts)
            .unwrap_or(u64::MAX);
        Duration::from_micros(micros.min(MAXIMUM.as_micros()))
    }

    /// The smoothed round trip time, once one has been measured.
    #[must_use]
    pub const fn smoothed(&self) -> Option<Duration> {
        self.smoothed
    }

    /// Its variation.
    #[must_use]
    pub const fn variation(&self) -> Duration {
        self.variation
    }

    /// How many attempts have gone unanswered. A connection gives up when
    /// this reaches the count it was configured with.
    #[must_use]
    pub const fn attempts(&self) -> u32 {
        self.attempts
    }

    /// Takes a round trip measurement, which throws away any backoff.
    ///
    /// The caller applies Karn's rule before it calls this: a segment that
    /// was sent more than once yields no measurement.
    pub fn sample(&mut self, rtt: Duration) {
        let measured = rtt.as_micros();
        let (smoothed, variation) = match self.smoothed {
            // The first measurement is taken as the truth, with half of it
            // as the variation (RFC 6298, section 2.2).
            None => (measured, measured.saturating_div(2)),
            Some(previous) => {
                let previous = previous.as_micros();
                // RTTVAR <- 3/4 RTTVAR + 1/4 |SRTT - R|
                let variation = self
                    .variation
                    .as_micros()
                    .saturating_mul(3)
                    .saturating_add(previous.abs_diff(measured))
                    .saturating_div(4);
                // SRTT <- 7/8 SRTT + 1/8 R
                let smoothed = previous
                    .saturating_mul(7)
                    .saturating_add(measured)
                    .saturating_div(8);
                (smoothed, variation)
            }
        };
        self.smoothed = Some(Duration::from_micros(smoothed));
        self.variation = Duration::from_micros(variation);
        self.attempts = 0;
        let computed = smoothed.saturating_add(variation.saturating_mul(VARIATION_WEIGHT));
        self.base =
            Duration::from_micros(computed.max(MINIMUM.as_micros()).min(MAXIMUM.as_micros()));
    }

    /// Records that the timeout expired, which doubles it.
    pub const fn back_off(&mut self) {
        self.attempts = self.attempts.saturating_add(1);
    }

    /// Forgets every measurement, which is what a connection does when it
    /// is reused.
    pub const fn reset(&mut self) {
        *self = Rto::new();
    }
}
