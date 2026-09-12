// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The firmware clock, read as a count of seconds from the Unix epoch.
//!
//! UEFI 2.11, section 8.3.1 defines [`Time`] as a local date and time with
//! the offset that makes it universal beside it. This module is the
//! arithmetic that turns one into the other, and it is here rather than in
//! the loader so that a calendar conversion is host-tested under a coverage
//! gate instead of living next to an `unsafe` call.
//!
//! The resolution is the second. `EFI_TIME` carries a nanosecond field and
//! the loader drops it: the device behind it is a real time clock whose
//! reporting resolution is one second (section 8.3.1,
//! `EFI_TIME_CAPABILITIES`), so the fraction says nothing the second does
//! not.

use audhsos_abi::WallClockSource;
use audhsos_time::{CivilTime, TimeError, UnixTime};

use crate::protocols::Time;

/// `EFI_UNSPECIFIED_TIMEZONE`. UEFI 2.11, section 8.3.1: the time is then
/// a local time whose offset the firmware does not name.
pub const UNSPECIFIED_TIMEZONE: i16 = 0x07FF;

/// The furthest a named offset may lie from universal time, in minutes.
/// UEFI 2.11, section 8.3.1 gives the range as -1440 to 1440.
const MAX_TIMEZONE: i16 = 1440;

/// `EFI_TIME_ADJUST_DAYLIGHT`: the time is one that daylight saving
/// affects.
pub const TIME_ADJUST_DAYLIGHT: u8 = 0x01;

/// `EFI_TIME_IN_DAYLIGHT`: the time has already been adjusted for daylight
/// saving.
pub const TIME_IN_DAYLIGHT: u8 = 0x02;

/// Seconds in one minute, for the offset arithmetic.
const SECONDS_PER_MINUTE: i64 = 60;

/// Why a firmware clock reading was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClockError {
    /// A date or time field is outside the range the calendar allows.
    Field(TimeError),
    /// The offset from universal time is neither
    /// [`UNSPECIFIED_TIMEZONE`] nor inside -1440 to 1440.
    TimeZone(i16),
    /// The daylight field has a bit the specification does not define.
    Daylight(u8),
    /// The moment is not after the epoch, which is a clock that was never
    /// set rather than a machine older than 1970.
    BeforeEpoch(i64),
}

/// The moment `time` names, in universal time, and how far it can be
/// trusted.
///
/// The offset is applied in the direction UEFI 2.11, section 8.3.1 gives:
/// `Localtime = UTC - TimeZone`, so universal time is the local time plus
/// the offset. The daylight bits are read only to be checked. They carry no
/// correction of their own: the same section has the firmware move both the
/// time and the offset when daylight saving begins or ends, so the offset
/// alone already describes the reading.
///
/// A value with no named offset is read as universal time and reported as
/// [`WallClockSource::FirmwareUnspecifiedZone`], because that is what the
/// section leaves a reader: a local time and no way to know which locality.
///
/// # Errors
///
/// [`ClockError`] for a reading this system will not build a clock out of.
/// A firmware that has no clock answers with an all-zero structure, whose
/// month of zero is a [`ClockError::Field`].
pub fn to_unix(time: Time) -> Result<(UnixTime, WallClockSource), ClockError> {
    if time.daylight & !(TIME_ADJUST_DAYLIGHT | TIME_IN_DAYLIGHT) != 0 {
        return Err(ClockError::Daylight(time.daylight));
    }
    let (offset, source) = if time.time_zone == UNSPECIFIED_TIMEZONE {
        (0, WallClockSource::FirmwareUnspecifiedZone)
    } else if time.time_zone < -MAX_TIMEZONE || time.time_zone > MAX_TIMEZONE {
        return Err(ClockError::TimeZone(time.time_zone));
    } else {
        (time.time_zone, WallClockSource::FirmwareUtc)
    };
    let civil = CivilTime::new(
        i32::from(time.year),
        time.month,
        time.day,
        time.hour,
        time.minute,
        time.second,
    )
    .map_err(ClockError::Field)?;
    let local = UnixTime::from_civil(civil).map_err(ClockError::Field)?;
    // Saturating rather than checked: the year is at most 9999, so the sum
    // is nowhere near the end of the type, and a checked call here would
    // add an arm no input can reach.
    let seconds = local
        .seconds()
        .saturating_add(i64::from(offset).saturating_mul(SECONDS_PER_MINUTE));
    if seconds <= 0 {
        return Err(ClockError::BeforeEpoch(seconds));
    }
    Ok((UnixTime::from_seconds(seconds), source))
}
