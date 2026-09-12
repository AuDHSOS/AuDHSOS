// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::time`, covering catalog item 6.6.71.

use audhsos_abi::WallClockSource;
use audhsos_time::TimeError;

use crate::protocols::Time;
use crate::time::{
    ClockError, TIME_ADJUST_DAYLIGHT, TIME_IN_DAYLIGHT, UNSPECIFIED_TIMEZONE, to_unix,
};

/// A reading of 2026-09-12T13:00:00, with the offset left to the caller.
fn at(time_zone: i16) -> Time {
    Time {
        year: 2026,
        month: 9,
        day: 12,
        hour: 13,
        minute: 0,
        second: 0,
        pad1: 0,
        nanosecond: 0,
        time_zone,
        daylight: 0,
        pad2: 0,
    }
}

/// 2026-09-12T13:00:00Z.
const NOON_ISH: i64 = 1_789_218_000;

#[test]
fn a_reading_in_universal_time_converts_to_its_second() {
    let (unix, source) = to_unix(at(0)).unwrap();
    assert_eq!(unix.seconds(), NOON_ISH);
    assert_eq!(source, WallClockSource::FirmwareUtc);
}

#[test]
fn a_named_offset_is_added_to_reach_universal_time() {
    // UEFI 2.11, section 8.3.1: Localtime = UTC - TimeZone, so a Pacific
    // standard reading of 13:00 with an offset of 480 is 21:00 UTC.
    let (unix, source) = to_unix(at(480)).unwrap();
    assert_eq!(unix.seconds(), NOON_ISH + 480 * 60);
    assert_eq!(source, WallClockSource::FirmwareUtc);

    let (ahead, _) = to_unix(at(-120)).unwrap();
    assert_eq!(ahead.seconds(), NOON_ISH - 120 * 60);
}

#[test]
fn an_unspecified_offset_is_read_as_universal_time_and_says_so() {
    let (unix, source) = to_unix(at(UNSPECIFIED_TIMEZONE)).unwrap();
    assert_eq!(unix.seconds(), NOON_ISH);
    assert_eq!(
        source,
        WallClockSource::FirmwareUnspecifiedZone,
        "the value is usable, and the caller is told it may be off by a zone"
    );
}

#[test]
fn the_offset_bounds_of_the_specification_are_the_ones_accepted() {
    for zone in [-1440, 1440] {
        assert!(to_unix(at(zone)).is_ok());
    }
    for zone in [-1441, 1441, 2046, i16::MIN, i16::MAX] {
        assert_eq!(to_unix(at(zone)), Err(ClockError::TimeZone(zone)));
    }
}

#[test]
fn the_daylight_bits_are_checked_and_change_nothing() {
    // The firmware moves the offset with the time when daylight saving
    // begins, so the bits carry no correction of their own.
    for bits in [0, TIME_ADJUST_DAYLIGHT, TIME_IN_DAYLIGHT, 0x03] {
        let mut time = at(0);
        time.daylight = bits;
        let (unix, _) = to_unix(time).unwrap();
        assert_eq!(unix.seconds(), NOON_ISH);
    }
    for bits in [0x04, 0x80, 0xFF] {
        let mut time = at(0);
        time.daylight = bits;
        assert_eq!(to_unix(time), Err(ClockError::Daylight(bits)));
    }
}

#[test]
fn a_firmware_without_a_clock_answers_zeros_and_is_refused() {
    let time = Time::default();
    assert_eq!(to_unix(time), Err(ClockError::Field(TimeError::Month(0))));
}

#[test]
fn a_field_outside_the_calendar_is_refused() {
    let cases = [
        (13_u8, 12_u8, 0_u8, 0_u8, 0_u8, TimeError::Month(13)),
        (9, 31, 0, 0, 0, TimeError::Day(31)),
        (2, 30, 0, 0, 0, TimeError::Day(30)),
        (9, 12, 24, 0, 0, TimeError::Hour(24)),
        (9, 12, 13, 60, 0, TimeError::Minute(60)),
        (9, 12, 13, 0, 60, TimeError::Second(60)),
    ];
    for (month, day, hour, minute, second, expected) in cases {
        let mut time = at(0);
        time.month = month;
        time.day = day;
        time.hour = hour;
        time.minute = minute;
        time.second = second;
        assert_eq!(to_unix(time), Err(ClockError::Field(expected)));
    }
}

#[test]
fn a_year_the_calendar_will_not_take_is_refused() {
    let mut time = at(0);
    time.year = 10_000;
    assert_eq!(
        to_unix(time),
        Err(ClockError::Field(TimeError::Year(10_000)))
    );
}

#[test]
fn a_moment_that_is_not_after_the_epoch_is_refused() {
    let mut time = at(0);
    time.year = 1969;
    let seconds = match to_unix(time) {
        Err(ClockError::BeforeEpoch(seconds)) => seconds,
        other => panic!("expected a refusal before the epoch, got {other:?}"),
    };
    assert!(seconds < 0);

    let mut epoch = at(0);
    epoch.year = 1970;
    epoch.month = 1;
    epoch.day = 1;
    epoch.hour = 0;
    assert_eq!(to_unix(epoch), Err(ClockError::BeforeEpoch(0)));
}

#[test]
fn a_leap_day_converts() {
    let mut time = at(0);
    time.year = 2028;
    time.month = 2;
    time.day = 29;
    time.hour = 0;
    let (unix, _) = to_unix(time).unwrap();
    assert_eq!(unix.seconds(), 1_835_395_200);
}

#[test]
fn the_nanosecond_field_is_not_read() {
    let mut time = at(0);
    time.nanosecond = 999_999_999;
    let (unix, _) = to_unix(time).unwrap();
    assert_eq!(unix.seconds(), NOON_ISH);
}
