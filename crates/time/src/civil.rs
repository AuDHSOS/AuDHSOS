// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The calendar: a date and a time of day in UTC, and the two functions
//! that turn a date into a day number and back.
//!
//! The rule is the proleptic Gregorian one, applied to every year in the
//! range without exception, so the crate disagrees with the historical
//! calendars that preceded it. That is a boundary, not an oversight: a
//! certificate, a file, and a network timer never carry a date from before
//! the reform, and a single rule is the one that can be tested exhaustively.
//!
//! The arithmetic is Howard Hinnant's `days_from_civil`, which shifts the
//! year to begin in March so that the leap day falls at its end and no
//! table of month lengths enters the conversion. Every intermediate value
//! is bounded by the year range this module validates before it computes,
//! which is why the wrapping operators below never wrap.

use crate::error::TimeError;

/// The lowest year the crate represents.
pub const MIN_YEAR: i32 = 0;

/// The highest year the crate represents. It is the highest a
/// `GeneralizedTime` can write down, and therefore the highest a
/// certificate can name.
pub const MAX_YEAR: i32 = 9999;

/// The day number of 0000-03-01 counted from 1970-01-01, negated: the
/// shift that moves Hinnant's March-based era onto the Unix epoch.
const EPOCH_SHIFT: i64 = 719_468;

/// The number of days in one era of four hundred years.
const DAYS_PER_ERA: i64 = 146_097;

/// The last day number of an era, which the year-of-era estimate divides by.
const LAST_DAY_OF_ERA: i64 = 146_096;

/// A date and a time of day in UTC.
///
/// The fields are public because the type is a record of what an encoding
/// carried, and every function of this crate that consumes one validates
/// it. A value built field by field is therefore not trusted; a value
/// built by [`CivilTime::new`] is.
///
/// The field order is the order of significance, so the derived ordering
/// is the chronological one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CivilTime {
    /// The year, in full, from [`MIN_YEAR`] to [`MAX_YEAR`].
    pub year: i32,
    /// The month, one to twelve.
    pub month: u8,
    /// The day, one to the length of its month in its year.
    pub day: u8,
    /// The hour, zero to twenty-three.
    pub hour: u8,
    /// The minute, zero to fifty-nine.
    pub minute: u8,
    /// The second, zero to fifty-nine. The POSIX scale has no leap second.
    pub second: u8,
}

impl CivilTime {
    /// The Unix epoch, 1970-01-01T00:00:00Z.
    pub const EPOCH: CivilTime = CivilTime {
        year: 1970,
        month: 1,
        day: 1,
        hour: 0,
        minute: 0,
        second: 0,
    };

    /// A validated time.
    ///
    /// # Errors
    ///
    /// The first field that is out of range, the day being checked against
    /// the true length of its month.
    pub const fn new(
        year: i32,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
    ) -> Result<CivilTime, TimeError> {
        let value = CivilTime {
            year,
            month,
            day,
            hour,
            minute,
            second,
        };
        match value.validate() {
            Ok(()) => Ok(value),
            Err(error) => Err(error),
        }
    }

    /// Checks every field against its range.
    ///
    /// # Errors
    ///
    /// The first field that is out of range, in the order year, month, day,
    /// hour, minute, second.
    pub const fn validate(&self) -> Result<(), TimeError> {
        if self.year < MIN_YEAR || self.year > MAX_YEAR {
            return Err(TimeError::Year(self.year));
        }
        let length = match days_in_month(self.year, self.month) {
            Ok(length) => length,
            Err(error) => return Err(error),
        };
        if self.day < 1 || self.day > length {
            return Err(TimeError::Day(self.day));
        }
        if self.hour > 23 {
            return Err(TimeError::Hour(self.hour));
        }
        if self.minute > 59 {
            return Err(TimeError::Minute(self.minute));
        }
        if self.second > 59 {
            return Err(TimeError::Second(self.second));
        }
        Ok(())
    }

    /// The seconds elapsed since midnight of the same day.
    ///
    /// # Errors
    ///
    /// [`TimeError`] for the first field that is out of range.
    pub fn seconds_of_day(&self) -> Result<u32, TimeError> {
        if self.hour > 23 {
            return Err(TimeError::Hour(self.hour));
        }
        if self.minute > 59 {
            return Err(TimeError::Minute(self.minute));
        }
        if self.second > 59 {
            return Err(TimeError::Second(self.second));
        }
        let hours = u32::from(self.hour).wrapping_mul(3600);
        let minutes = u32::from(self.minute).wrapping_mul(60);
        Ok(hours
            .wrapping_add(minutes)
            .wrapping_add(u32::from(self.second)))
    }
}

/// Whether `year` carries a twenty-ninth of February under the Gregorian
/// rule: every fourth year, except every hundredth, except every four
/// hundredth.
#[must_use]
pub const fn is_leap_year(year: i32) -> bool {
    year.wrapping_rem(4) == 0 && (year.wrapping_rem(100) != 0 || year.wrapping_rem(400) == 0)
}

/// The number of days in `month` of `year`.
///
/// # Errors
///
/// [`TimeError::Year`] when the year is outside [`MIN_YEAR`] to
/// [`MAX_YEAR`], [`TimeError::Month`] when the month is not one to twelve.
pub const fn days_in_month(year: i32, month: u8) -> Result<u8, TimeError> {
    if year < MIN_YEAR || year > MAX_YEAR {
        return Err(TimeError::Year(year));
    }
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => Ok(31),
        4 | 6 | 9 | 11 => Ok(30),
        2 if is_leap_year(year) => Ok(29),
        2 => Ok(28),
        other => Err(TimeError::Month(other)),
    }
}

/// The number of days from 1970-01-01 to the given date, negative before
/// the epoch.
///
/// # Errors
///
/// [`TimeError`] for the first field that is out of range, the day being
/// checked against the true length of its month.
pub fn days_from_civil(year: i32, month: u8, day: u8) -> Result<i64, TimeError> {
    let length = days_in_month(year, month)?;
    if day < 1 || day > length {
        return Err(TimeError::Day(day));
    }
    // March-based: a year that begins in March ends with its leap day, so
    // the day of the year is one expression and needs no month table.
    let shifted = if month <= 2 {
        i64::from(year).wrapping_sub(1)
    } else {
        i64::from(year)
    };
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted.wrapping_sub(399)
    }
    .wrapping_div(400);
    let year_of_era = shifted.wrapping_sub(era.wrapping_mul(400));
    let month_of_era = if month > 2 {
        i64::from(month).wrapping_sub(3)
    } else {
        i64::from(month).wrapping_add(9)
    };
    let day_of_year = month_of_era
        .wrapping_mul(153)
        .wrapping_add(2)
        .wrapping_div(5)
        .wrapping_add(i64::from(day))
        .wrapping_sub(1);
    let day_of_era = year_of_era
        .wrapping_mul(365)
        .wrapping_add(year_of_era.wrapping_div(4))
        .wrapping_sub(year_of_era.wrapping_div(100))
        .wrapping_add(day_of_year);
    Ok(era
        .wrapping_mul(DAYS_PER_ERA)
        .wrapping_add(day_of_era)
        .wrapping_sub(EPOCH_SHIFT))
}

/// The date `days` days after 1970-01-01, as year, month, and day.
///
/// # Errors
///
/// [`TimeError::Year`] when the date falls outside [`MIN_YEAR`] to
/// [`MAX_YEAR`], [`TimeError::OutOfRange`] when `days` is so far out that
/// the shift onto the March-based era does not fit an `i64`.
pub fn civil_from_days(days: i64) -> Result<(i32, u8, u8), TimeError> {
    let shifted = days.checked_add(EPOCH_SHIFT).ok_or(TimeError::OutOfRange)?;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted.wrapping_sub(LAST_DAY_OF_ERA)
    }
    .wrapping_div(DAYS_PER_ERA);
    let day_of_era = shifted.wrapping_sub(era.wrapping_mul(DAYS_PER_ERA));
    let year_of_era = day_of_era
        .wrapping_sub(day_of_era.wrapping_div(1460))
        .wrapping_add(day_of_era.wrapping_div(36524))
        .wrapping_sub(day_of_era.wrapping_div(LAST_DAY_OF_ERA))
        .wrapping_div(365);
    let day_of_year = day_of_era.wrapping_sub(
        year_of_era
            .wrapping_mul(365)
            .wrapping_add(year_of_era.wrapping_div(4))
            .wrapping_sub(year_of_era.wrapping_div(100)),
    );
    let month_of_era = day_of_year
        .wrapping_mul(5)
        .wrapping_add(2)
        .wrapping_div(153);
    let day = day_of_year
        .wrapping_sub(
            month_of_era
                .wrapping_mul(153)
                .wrapping_add(2)
                .wrapping_div(5),
        )
        .wrapping_add(1);
    let month = if month_of_era < 10 {
        month_of_era.wrapping_add(3)
    } else {
        month_of_era.wrapping_sub(9)
    };
    let year = era
        .wrapping_mul(400)
        .wrapping_add(year_of_era)
        .wrapping_add(i64::from(month <= 2));
    let year = i32::try_from(year).map_err(|_| TimeError::OutOfRange)?;
    if !(MIN_YEAR..=MAX_YEAR).contains(&year) {
        return Err(TimeError::Year(year));
    }
    // Both are bounded by the algorithm: the month of an era is zero to
    // eleven and the day of a month is one to thirty-one. The narrowing is
    // fallible only because no infallible one exists.
    let month = u8::try_from(month).map_err(|_| TimeError::OutOfRange)?;
    let day = u8::try_from(day).map_err(|_| TimeError::OutOfRange)?;
    Ok((year, month, day))
}
