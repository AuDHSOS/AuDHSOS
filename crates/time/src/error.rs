// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why a time value was rejected.

use core::fmt;

/// What this crate found wrong with a time or with the result of an
/// operation on one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TimeError {
    /// The year is negative or above [`MAX_YEAR`](crate::civil::MAX_YEAR).
    Year(i32),
    /// The month is not one to twelve.
    Month(u8),
    /// The day is zero, or above the length of its month in its year.
    Day(u8),
    /// The hour is not zero to twenty-three.
    Hour(u8),
    /// The minute is not zero to fifty-nine.
    Minute(u8),
    /// The second is not zero to fifty-nine. The POSIX scale has no leap
    /// second, so sixty is out of range here as well.
    Second(u8),
    /// The result of a conversion or of an arithmetic operation does not
    /// fit the type that would have to hold it.
    OutOfRange,
}

impl fmt::Display for TimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TimeError::Year(year) => write!(f, "the year {year} is outside 0 to 9999"),
            TimeError::Month(month) => write!(f, "the month {month} is not one to twelve"),
            TimeError::Day(day) => write!(f, "the day {day} is not one to the length of its month"),
            TimeError::Hour(hour) => write!(f, "the hour {hour} is not zero to twenty-three"),
            TimeError::Minute(minute) => {
                write!(f, "the minute {minute} is not zero to fifty-nine")
            }
            TimeError::Second(second) => {
                write!(f, "the second {second} is not zero to fifty-nine")
            }
            TimeError::OutOfRange => f.write_str("the result is outside the range of its type"),
        }
    }
}
