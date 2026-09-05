// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Generators of the time types for property tests. Available behind the
//! feature `test-strategies` and in this crate's own tests.
//!
//! The day numbers are the ones the calendar covers: [`FIRST_DAY`] is
//! 0000-01-01 and [`LAST_DAY`] is 9999-12-31. A generator that produced a
//! day outside them would test nothing but the error path.

use test_support::generators::{BoxGen, Generator, pair, range};

use crate::civil::{CivilTime, civil_from_days};
use crate::instant::{Duration, Instant};
use crate::unix::UnixTime;

/// The day number of 0000-01-01, the first date the calendar represents.
pub const FIRST_DAY: i64 = -719_528;

/// The day number of 9999-12-31, the last date the calendar represents.
pub const LAST_DAY: i64 = 2_932_896;

/// A day number the calendar covers.
#[must_use]
pub fn any_day() -> BoxGen<i64> {
    range(FIRST_DAY..=LAST_DAY).boxed()
}

/// A valid civil time: a date the calendar covers and a time of day.
#[must_use]
pub fn any_civil_time() -> BoxGen<CivilTime> {
    pair(
        any_day(),
        pair(range(0u8..=23), pair(range(0u8..=59), range(0u8..=59))),
    )
    .map(|(days, (hour, (minute, second)))| {
        let (year, month, day) = civil_from_days(days).unwrap_or((1970, 1, 1));
        CivilTime {
            year,
            month,
            day,
            hour,
            minute,
            second,
        }
    })
    .boxed()
}

/// A point in time the calendar covers, to the second.
#[must_use]
pub fn any_unix_time() -> BoxGen<UnixTime> {
    pair(any_day(), range(0i64..=86_399))
        .map(|(days, rest)| UnixTime::from_seconds(days.wrapping_mul(86_400).wrapping_add(rest)))
        .boxed()
}

/// A span of up to one day.
#[must_use]
pub fn any_duration() -> BoxGen<Duration> {
    range(0u64..=86_400_000_000)
        .map(Duration::from_micros)
        .boxed()
}

/// A point on the monotonic scale within the first thousand years of its
/// origin, so that adding [`any_duration`] to it cannot saturate.
#[must_use]
pub fn any_instant() -> BoxGen<Instant> {
    range(0u64..=31_536_000_000_000_000)
        .map(Instant::from_micros)
        .boxed()
}
