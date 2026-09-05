// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Unix time: seconds from 1970-01-01T00:00:00Z, negative before it.
//!
//! The scale is the POSIX one, on which a day is 86400 seconds without
//! exception. A leap second therefore has no number of its own; the second
//! before it is named twice. That is a limit of the scale and not of this
//! crate, and it is stated here because a certificate window that is off
//! by the twenty-seven leap seconds inserted so far is off by less than
//! any window a certificate carries.

use crate::civil::{CivilTime, civil_from_days, days_from_civil};
use crate::error::TimeError;
use crate::instant::Duration;

/// Seconds in one day on the POSIX scale.
const SECONDS_PER_DAY: i64 = 86_400;

/// A point in time as seconds from the Unix epoch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnixTime(i64);

impl UnixTime {
    /// 1970-01-01T00:00:00Z.
    pub const EPOCH: UnixTime = UnixTime(0);

    /// The point `seconds` seconds from the epoch.
    #[must_use]
    pub const fn from_seconds(seconds: i64) -> UnixTime {
        UnixTime(seconds)
    }

    /// The point as seconds from the epoch.
    #[must_use]
    pub const fn seconds(self) -> i64 {
        self.0
    }

    /// The point a civil time names.
    ///
    /// # Errors
    ///
    /// [`TimeError`] for the first field of `time` that is out of range.
    pub fn from_civil(time: CivilTime) -> Result<UnixTime, TimeError> {
        let days = days_from_civil(time.year, time.month, time.day)?;
        let seconds_of_day = time.seconds_of_day()?;
        let seconds = days
            .checked_mul(SECONDS_PER_DAY)
            .and_then(|seconds| seconds.checked_add(i64::from(seconds_of_day)))
            .ok_or(TimeError::OutOfRange)?;
        Ok(UnixTime(seconds))
    }

    /// The civil time this point names, in UTC.
    ///
    /// # Errors
    ///
    /// [`TimeError::Year`] when the date falls outside the year range of
    /// [`CivilTime`], [`TimeError::OutOfRange`] when the division into days
    /// does not fit.
    pub fn to_civil(self) -> Result<CivilTime, TimeError> {
        // Euclidean division, so that a point before the epoch belongs to
        // the day it falls in rather than to the one after it.
        let days = self
            .0
            .checked_div_euclid(SECONDS_PER_DAY)
            .ok_or(TimeError::OutOfRange)?;
        let rest = self
            .0
            .checked_rem_euclid(SECONDS_PER_DAY)
            .ok_or(TimeError::OutOfRange)?;
        let (year, month, day) = civil_from_days(days)?;
        let hour = rest.wrapping_div(3600);
        let minute = rest.wrapping_rem(3600).wrapping_div(60);
        let second = rest.wrapping_rem(60);
        // The remainder of a Euclidean division by 86400 is zero to 86399,
        // so each of the three is bounded; the narrowing is fallible only
        // because no infallible one exists.
        let hour = u8::try_from(hour).map_err(|_| TimeError::OutOfRange)?;
        let minute = u8::try_from(minute).map_err(|_| TimeError::OutOfRange)?;
        let second = u8::try_from(second).map_err(|_| TimeError::OutOfRange)?;
        Ok(CivilTime {
            year,
            month,
            day,
            hour,
            minute,
            second,
        })
    }

    /// The point `duration` later, or `None` when the result leaves the
    /// range of an `i64`.
    ///
    /// The resolution of a `UnixTime` is one second, so the part of
    /// `duration` below a second does not contribute.
    #[must_use]
    pub const fn checked_add(self, duration: Duration) -> Option<UnixTime> {
        match self.0.checked_add(duration.as_secs_i64()) {
            Some(seconds) => Some(UnixTime(seconds)),
            None => None,
        }
    }

    /// The point `duration` earlier, or `None` when the result leaves the
    /// range of an `i64`.
    ///
    /// The resolution of a `UnixTime` is one second, so the part of
    /// `duration` below a second does not contribute.
    #[must_use]
    pub const fn checked_sub(self, duration: Duration) -> Option<UnixTime> {
        match self.0.checked_sub(duration.as_secs_i64()) {
            Some(seconds) => Some(UnixTime(seconds)),
            None => None,
        }
    }

    /// The span from `earlier` to this point, or `None` when this point is
    /// the earlier of the two or the difference does not fit a `Duration`.
    #[must_use]
    pub fn checked_duration_since(self, earlier: UnixTime) -> Option<Duration> {
        let seconds = self.0.checked_sub(earlier.0)?;
        let seconds = u64::try_from(seconds).ok()?;
        seconds.checked_mul(1_000_000).map(Duration::from_micros)
    }
}

impl TryFrom<CivilTime> for UnixTime {
    type Error = TimeError;

    fn try_from(time: CivilTime) -> Result<UnixTime, TimeError> {
        UnixTime::from_civil(time)
    }
}

impl TryFrom<UnixTime> for CivilTime {
    type Error = TimeError;

    fn try_from(time: UnixTime) -> Result<CivilTime, TimeError> {
        time.to_civil()
    }
}
