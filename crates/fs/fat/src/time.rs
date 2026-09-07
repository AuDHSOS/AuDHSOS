// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The date and the time a directory entry carries, against the calendar
//! of `audhsos-time`.
//!
//! FAT packs a date into sixteen bits — seven of year from 1980, four of
//! month, five of day — and a time into sixteen more — five of hour, six
//! of minute, five of a second that counts in twos. What a caller hands
//! over is a [`UnixTime`]; what does not fit those fields is refused
//! rather than rounded, so that what is written reads back as what it
//! was.

use audhsos_time::{CivilTime, UnixTime};

use crate::error::Error;

/// The first year a directory entry can name.
pub const MIN_YEAR: i32 = 1980;

/// The last year a directory entry can name: 1980 plus the 127 the seven
/// bits of the field hold.
pub const MAX_YEAR: i32 = 2107;

/// The date and the time of `at`, as a directory entry carries them.
///
/// # Errors
///
/// [`Error::Time`] for a time before [`MIN_YEAR`], after [`MAX_YEAR`], or
/// with an odd second, which the field cannot say.
pub fn to_entry(at: UnixTime) -> Result<(u16, u16), Error> {
    let civil = at.to_civil().map_err(|_| Error::Time)?;
    if civil.year < MIN_YEAR || civil.year > MAX_YEAR || civil.second % 2 != 0 {
        return Err(Error::Time);
    }
    let years = u16::try_from(civil.year.saturating_sub(MIN_YEAR)).map_err(|_| Error::Time)?;
    let date = (years << 9) | (u16::from(civil.month) << 5) | u16::from(civil.day);
    let time = (u16::from(civil.hour) << 11)
        | (u16::from(civil.minute) << 5)
        | u16::from(civil.second / 2);
    Ok((date, time))
}

/// The time a directory entry's `date` and `time` fields stand for.
///
/// # Errors
///
/// [`Error::Time`] for fields that name no day of the calendar: a month
/// or a day of zero, a day past the end of its month, or an hour, minute,
/// or second out of range.
pub fn from_entry(date: u16, time: u16) -> Result<UnixTime, Error> {
    let year = i32::from(date >> 9).saturating_add(MIN_YEAR);
    let month = u8::try_from((date >> 5) & 0x0F).map_err(|_| Error::Time)?;
    let day = u8::try_from(date & 0x1F).map_err(|_| Error::Time)?;
    let hour = u8::try_from(time >> 11).map_err(|_| Error::Time)?;
    let minute = u8::try_from((time >> 5) & 0x3F).map_err(|_| Error::Time)?;
    let second = u8::try_from((time & 0x1F).saturating_mul(2)).map_err(|_| Error::Time)?;
    let civil = CivilTime::new(year, month, day, hour, minute, second).map_err(|_| Error::Time)?;
    UnixTime::from_civil(civil).map_err(|_| Error::Time)
}
