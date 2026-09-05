// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Monotonic time: a point that only differences of it give meaning to,
//! and the span between two such points. Both count microseconds, which
//! resolves a retransmission timer and still spans half a million years in
//! a `u64`.
//!
//! Nothing here reads a clock. An [`Instant`] enters an interface as a
//! parameter, which is what lets a sixty-second backoff be exercised in
//! microseconds of wall clock (D-46).

use core::ops::{Add, AddAssign, Sub};

/// Microseconds in one millisecond.
const MICROS_PER_MILLI: u64 = 1_000;

/// Microseconds in one second.
const MICROS_PER_SECOND: u64 = 1_000_000;

/// A span of time, in microseconds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Duration(u64);

impl Duration {
    /// No time at all.
    pub const ZERO: Duration = Duration(0);

    /// The longest span this type holds.
    pub const MAX: Duration = Duration(u64::MAX);

    /// A span of `micros` microseconds.
    #[must_use]
    pub const fn from_micros(micros: u64) -> Duration {
        Duration(micros)
    }

    /// A span of `millis` milliseconds, saturating at [`Duration::MAX`].
    #[must_use]
    pub const fn from_millis(millis: u64) -> Duration {
        Duration(millis.saturating_mul(MICROS_PER_MILLI))
    }

    /// A span of `seconds` seconds, saturating at [`Duration::MAX`].
    #[must_use]
    pub const fn from_secs(seconds: u64) -> Duration {
        Duration(seconds.saturating_mul(MICROS_PER_SECOND))
    }

    /// The span in microseconds.
    #[must_use]
    pub const fn as_micros(self) -> u64 {
        self.0
    }

    /// The span in whole milliseconds; a remainder below one millisecond
    /// is dropped.
    #[must_use]
    pub const fn as_millis(self) -> u64 {
        self.0.wrapping_div(MICROS_PER_MILLI)
    }

    /// The span in whole seconds; a remainder below one second is dropped.
    #[must_use]
    pub const fn as_secs(self) -> u64 {
        self.0.wrapping_div(MICROS_PER_SECOND)
    }

    /// The span in whole seconds as a signed value.
    ///
    /// The longest span is `u64::MAX` microseconds, whose whole seconds are
    /// 18446744073709; every value of the type therefore fits an `i64` and
    /// the narrowing is exact.
    #[must_use]
    pub const fn as_secs_i64(self) -> i64 {
        self.as_secs().cast_signed()
    }

    /// Whether the span is zero.
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// The sum of two spans, or `None` when it would overflow.
    #[must_use]
    pub const fn checked_add(self, other: Duration) -> Option<Duration> {
        match self.0.checked_add(other.0) {
            Some(micros) => Some(Duration(micros)),
            None => None,
        }
    }

    /// The difference of two spans, or `None` when `other` is the longer.
    #[must_use]
    pub const fn checked_sub(self, other: Duration) -> Option<Duration> {
        match self.0.checked_sub(other.0) {
            Some(micros) => Some(Duration(micros)),
            None => None,
        }
    }

    /// The span multiplied by `factor`, or `None` when it would overflow.
    /// A retransmission backoff is this and nothing else.
    #[must_use]
    #[expect(
        clippy::as_conversions,
        reason = "`u64::from` is not a const fn; the widening is exact"
    )]
    pub const fn checked_mul(self, factor: u32) -> Option<Duration> {
        match self.0.checked_mul(factor as u64) {
            Some(micros) => Some(Duration(micros)),
            None => None,
        }
    }

    /// The sum of two spans, capped at [`Duration::MAX`].
    #[must_use]
    pub const fn saturating_add(self, other: Duration) -> Duration {
        Duration(self.0.saturating_add(other.0))
    }

    /// The difference of two spans, zero when `other` is the longer.
    #[must_use]
    pub const fn saturating_sub(self, other: Duration) -> Duration {
        Duration(self.0.saturating_sub(other.0))
    }
}

/// A point on a monotonic scale, in microseconds from an origin the caller
/// chooses. Two instants from different origins must not be compared; the
/// crate cannot detect that, so the origin is part of what a caller
/// documents.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Instant(u64);

impl Instant {
    /// The origin of the scale.
    pub const ZERO: Instant = Instant(0);

    /// The furthest point the scale reaches.
    pub const MAX: Instant = Instant(u64::MAX);

    /// The point `micros` microseconds after the origin.
    #[must_use]
    pub const fn from_micros(micros: u64) -> Instant {
        Instant(micros)
    }

    /// The point as microseconds from the origin.
    #[must_use]
    pub const fn as_micros(self) -> u64 {
        self.0
    }

    /// The point `duration` later, capped at [`Instant::MAX`]. A timer that
    /// saturates fires late; one that wraps fires immediately and forever,
    /// which is why this is the addition the `Add` implementation uses.
    #[must_use]
    pub const fn saturating_add(self, duration: Duration) -> Instant {
        Instant(self.0.saturating_add(duration.0))
    }

    /// The point `duration` later, or `None` when it would overflow.
    #[must_use]
    pub const fn checked_add(self, duration: Duration) -> Option<Instant> {
        match self.0.checked_add(duration.0) {
            Some(micros) => Some(Instant(micros)),
            None => None,
        }
    }

    /// The point `duration` earlier, or `None` when it would fall before
    /// the origin.
    #[must_use]
    pub const fn checked_sub(self, duration: Duration) -> Option<Instant> {
        match self.0.checked_sub(duration.0) {
            Some(micros) => Some(Instant(micros)),
            None => None,
        }
    }

    /// The span from `earlier` to this point, zero when this point is not
    /// the later of the two.
    #[must_use]
    pub const fn saturating_duration_since(self, earlier: Instant) -> Duration {
        Duration(self.0.saturating_sub(earlier.0))
    }

    /// The span from `earlier` to this point, or `None` when this point is
    /// the earlier of the two.
    #[must_use]
    pub const fn checked_duration_since(self, earlier: Instant) -> Option<Duration> {
        match self.0.checked_sub(earlier.0) {
            Some(micros) => Some(Duration(micros)),
            None => None,
        }
    }
}

impl Add<Duration> for Instant {
    type Output = Instant;

    /// Saturating, as [`Instant::saturating_add`] explains.
    fn add(self, duration: Duration) -> Instant {
        self.saturating_add(duration)
    }
}

impl AddAssign<Duration> for Instant {
    fn add_assign(&mut self, duration: Duration) {
        *self = self.saturating_add(duration);
    }
}

impl Sub<Instant> for Instant {
    type Output = Duration;

    /// Saturating, as [`Instant::saturating_duration_since`] explains.
    fn sub(self, earlier: Instant) -> Duration {
        self.saturating_duration_since(earlier)
    }
}

impl Add<Duration> for Duration {
    type Output = Duration;

    /// Saturating at [`Duration::MAX`].
    fn add(self, other: Duration) -> Duration {
        self.saturating_add(other)
    }
}

impl Sub<Duration> for Duration {
    type Output = Duration;

    /// Saturating at [`Duration::ZERO`].
    fn sub(self, other: Duration) -> Duration {
        self.saturating_sub(other)
    }
}
