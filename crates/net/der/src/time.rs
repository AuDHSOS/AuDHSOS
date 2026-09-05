// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The two time forms of RFC 5280, read into the [`CivilTime`] of
//! `audhsos-time`.
//!
//! This module owns the syntax and nothing else: the digits, the `Z`
//! suffix, the two-digit year window of `UTCTime`, and the forms the
//! profile forbids. What a field may hold, the true length of a month
//! included, belongs to the calendar, so the parser hands its fields to
//! [`CivilTime::validate`] and reports [`DerError::BadTime`] for whatever
//! that rejects (D-46).

use audhsos_time::CivilTime;

use crate::error::DerError;

/// The year at which the two-digit window of `UTCTime` turns: a year of
/// fifty or above is nineteen hundred, below it two thousand, as RFC 5280
/// prescribes.
const WINDOW: u8 = 50;

/// The time a `UTCTime` holds: `YYMMDDHHMMSSZ` and nothing else.
///
/// # Errors
///
/// [`DerError::BadTime`] for any other form, a non-digit, or a field the
/// calendar rejects.
pub fn from_utc_time(bytes: &[u8]) -> Result<CivilTime, DerError> {
    let digits: &[u8; 13] = bytes.try_into().map_err(|_| DerError::BadTime)?;
    let (year, rest) = two_digits(digits)?;
    let full = if year >= WINDOW {
        i32::from(year).wrapping_add(1900)
    } else {
        i32::from(year).wrapping_add(2000)
    };
    from_fields(full, rest)
}

/// The time a `GeneralizedTime` holds: `YYYYMMDDHHMMSSZ` and nothing else.
/// RFC 5280 forbids the fractional seconds and the local time forms that
/// the encoding rules otherwise allow.
///
/// # Errors
///
/// [`DerError::BadTime`] for any other form, a non-digit, or a field the
/// calendar rejects.
pub fn from_generalized_time(bytes: &[u8]) -> Result<CivilTime, DerError> {
    let digits: &[u8; 15] = bytes.try_into().map_err(|_| DerError::BadTime)?;
    let (century, rest) = two_digits(digits)?;
    let (year, rest) = two_digits(rest)?;
    let full = i32::from(century)
        .wrapping_mul(100)
        .wrapping_add(i32::from(year));
    from_fields(full, rest)
}

/// The fields after the year: month, day, hour, minute, second, `Z`.
fn from_fields(year: i32, rest: &[u8]) -> Result<CivilTime, DerError> {
    let (month, rest) = two_digits(rest)?;
    let (day, rest) = two_digits(rest)?;
    let (hour, rest) = two_digits(rest)?;
    let (minute, rest) = two_digits(rest)?;
    let (second, rest) = two_digits(rest)?;
    if rest != b"Z" {
        return Err(DerError::BadTime);
    }
    CivilTime::new(year, month, day, hour, minute, second).map_err(|_| DerError::BadTime)
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
