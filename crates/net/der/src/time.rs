// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The two time types, as far as their syntax goes.
//!
//! What this module does not do is deliberate and temporary. It checks the
//! form RFC 5280 requires, the digits, the `Z` suffix, and the range of
//! every field, and it applies the two-digit year window of `UTCTime`. It
//! does not turn the fields into a point in time, and it checks the day
//! against thirty-one rather than against the true length of its month,
//! because both need a calendar and decision D-46 puts the calendar in
//! `audhsos-time`.
//!
//! `audhsos-time` does not exist yet. When it does, [`Timestamp`] becomes
//! its `CivilTime`, this crate gains that dependency, and the conversion
//! and the remaining check arrive with it. Section 11.14 of document 11
//! carries this as an open seam so that it is not forgotten.

use crate::error::DerError;

/// A time as the fields the encoding carries, in UTC.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp {
    /// The year, in full, after the two-digit window of `UTCTime` has been
    /// applied.
    pub year: u16,
    /// The month, one to twelve.
    pub month: u8,
    /// The day, one to thirty-one. The true length of the month is checked
    /// where the calendar lives.
    pub day: u8,
    /// The hour, zero to twenty-three.
    pub hour: u8,
    /// The minute, zero to fifty-nine.
    pub minute: u8,
    /// The second, zero to fifty-nine. RFC 5280 requires seconds to be
    /// present, so there is no absent case.
    pub second: u8,
}

/// The year at which the two-digit window of `UTCTime` turns: a year of
/// fifty or above is nineteen hundred, below it two thousand, as RFC 5280
/// prescribes.
const WINDOW: u8 = 50;

impl Timestamp {
    /// The time a `UTCTime` holds: `YYMMDDHHMMSSZ` and nothing else.
    ///
    /// # Errors
    ///
    /// [`DerError::BadTime`] for any other form, a non-digit, or a field
    /// out of range.
    pub fn from_utc_time(bytes: &[u8]) -> Result<Timestamp, DerError> {
        let digits: &[u8; 13] = bytes.try_into().map_err(|_| DerError::BadTime)?;
        let (year, rest) = two_digits(digits)?;
        let full = if year >= WINDOW {
            u16::from(year).wrapping_add(1900)
        } else {
            u16::from(year).wrapping_add(2000)
        };
        Timestamp::from_fields(full, rest)
    }

    /// The time a `GeneralizedTime` holds: `YYYYMMDDHHMMSSZ` and nothing
    /// else. RFC 5280 forbids the fractional seconds and the local time
    /// forms that the encoding rules otherwise allow.
    ///
    /// # Errors
    ///
    /// [`DerError::BadTime`] for any other form, a non-digit, or a field
    /// out of range.
    pub fn from_generalized_time(bytes: &[u8]) -> Result<Timestamp, DerError> {
        let digits: &[u8; 15] = bytes.try_into().map_err(|_| DerError::BadTime)?;
        let (century, rest) = two_digits(digits)?;
        let (year, rest) = two_digits(rest)?;
        let full = u16::from(century)
            .wrapping_mul(100)
            .wrapping_add(u16::from(year));
        Timestamp::from_fields(full, rest)
    }

    /// The fields after the year: month, day, hour, minute, second, `Z`.
    fn from_fields(year: u16, rest: &[u8]) -> Result<Timestamp, DerError> {
        let (month, rest) = two_digits(rest)?;
        let (day, rest) = two_digits(rest)?;
        let (hour, rest) = two_digits(rest)?;
        let (minute, rest) = two_digits(rest)?;
        let (second, rest) = two_digits(rest)?;
        if rest != b"Z" {
            return Err(DerError::BadTime);
        }
        if !(1..=12).contains(&month)
            || !(1..=31).contains(&day)
            || hour > 23
            || minute > 59
            || second > 59
        {
            return Err(DerError::BadTime);
        }
        Ok(Timestamp {
            year,
            month,
            day,
            hour,
            minute,
            second,
        })
    }
}

/// The value of the first two digits, and what is left after them.
fn two_digits(bytes: &[u8]) -> Result<(u8, &[u8]), DerError> {
    let (pair, rest) = bytes.split_first_chunk::<2>().ok_or(DerError::BadTime)?;
    let [high, low] = *pair;
    let high = digit(high)?;
    let low = digit(low)?;
    Ok((high.wrapping_mul(10).wrapping_add(low), rest))
}

/// The value of one decimal digit.
const fn digit(byte: u8) -> Result<u8, DerError> {
    if byte.is_ascii_digit() {
        Ok(byte.wrapping_sub(b'0'))
    } else {
        Err(DerError::BadTime)
    }
}
