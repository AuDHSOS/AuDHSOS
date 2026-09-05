// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Unix time: the two conversions, the ordering, and the checked
//! arithmetic at the extremes.

use test_support::property::check;

use crate::civil::CivilTime;
use crate::error::TimeError;
use crate::instant::Duration;
use crate::strategies::{FIRST_DAY, LAST_DAY, any_unix_time};
use crate::unix::UnixTime;

/// The last second the calendar covers, 9999-12-31T23:59:59Z.
const LAST_SECOND: i64 = 253_402_300_799;

/// The first second the calendar covers, 0000-01-01T00:00:00Z.
const FIRST_SECOND: i64 = -62_167_219_200;

#[test]
fn the_epoch_is_zero() {
    assert_eq!(UnixTime::EPOCH.seconds(), 0);
    assert_eq!(UnixTime::from_civil(CivilTime::EPOCH), Ok(UnixTime::EPOCH));
    assert_eq!(UnixTime::EPOCH.to_civil(), Ok(CivilTime::EPOCH));
}

#[test]
fn a_known_point_converts_both_ways() {
    let time = CivilTime::new(2001, 9, 9, 1, 46, 40).expect("a valid time");
    let point = UnixTime::from_seconds(1_000_000_000);
    assert_eq!(UnixTime::from_civil(time), Ok(point));
    assert_eq!(point.to_civil(), Ok(time));
}

#[test]
fn a_time_before_the_epoch_is_negative_and_converts_back() {
    for (seconds, fields) in [
        (-1, (1969, 12, 31, 23, 59, 59)),
        (-86_400, (1969, 12, 31, 0, 0, 0)),
        (-86_401, (1969, 12, 30, 23, 59, 59)),
        (FIRST_SECOND, (0, 1, 1, 0, 0, 0)),
    ] {
        let (year, month, day, hour, minute, second) = fields;
        let time = CivilTime::new(year, month, day, hour, minute, second).expect("a valid time");
        let point = UnixTime::from_seconds(seconds);
        assert!(point.seconds() < 0 || seconds == 0, "{seconds}");
        assert_eq!(point.to_civil(), Ok(time), "{seconds}");
        assert_eq!(UnixTime::from_civil(time), Ok(point), "{seconds}");
    }
}

#[test]
fn the_ends_of_the_calendar_convert_both_ways() {
    let last = CivilTime::new(9999, 12, 31, 23, 59, 59).expect("a valid time");
    assert_eq!(
        UnixTime::from_civil(last),
        Ok(UnixTime::from_seconds(LAST_SECOND))
    );
    assert_eq!(UnixTime::from_seconds(LAST_SECOND).to_civil(), Ok(last));
    assert_eq!(
        UnixTime::from_seconds(FIRST_SECOND).seconds(),
        FIRST_DAY.saturating_mul(86_400)
    );
}

#[test]
fn a_point_outside_the_calendar_does_not_convert() {
    assert_eq!(
        UnixTime::from_seconds(LAST_SECOND.saturating_add(1)).to_civil(),
        Err(TimeError::Year(10_000))
    );
    assert_eq!(
        UnixTime::from_seconds(FIRST_SECOND.saturating_sub(1)).to_civil(),
        Err(TimeError::Year(-1))
    );
    assert_eq!(
        UnixTime::from_seconds(i64::MAX).to_civil(),
        Err(TimeError::OutOfRange)
    );
    assert_eq!(
        UnixTime::from_seconds(i64::MIN).to_civil(),
        Err(TimeError::OutOfRange)
    );
}

#[test]
fn a_time_that_does_not_validate_does_not_convert() {
    let bad = CivilTime {
        year: 2023,
        month: 2,
        day: 30,
        hour: 0,
        minute: 0,
        second: 0,
    };
    assert_eq!(UnixTime::from_civil(bad), Err(TimeError::Day(30)));
    let bad_hour = CivilTime {
        hour: 24,
        ..CivilTime::EPOCH
    };
    assert_eq!(UnixTime::from_civil(bad_hour), Err(TimeError::Hour(24)));
}

#[test]
fn checked_arithmetic_at_the_extremes_returns_none() {
    let far = UnixTime::from_seconds(i64::MAX);
    assert_eq!(far.checked_add(Duration::from_secs(1)), None);
    assert_eq!(
        far.checked_sub(Duration::from_secs(1)),
        Some(UnixTime::from_seconds(i64::MAX.saturating_sub(1)))
    );
    let near = UnixTime::from_seconds(i64::MIN);
    assert_eq!(near.checked_sub(Duration::from_secs(1)), None);
    assert_eq!(
        near.checked_add(Duration::from_secs(1)),
        Some(UnixTime::from_seconds(i64::MIN.saturating_add(1)))
    );
    // The whole seconds of the longest duration fit an `i64`, so the only
    // way to fail is the sum itself.
    assert_eq!(
        UnixTime::EPOCH.checked_add(Duration::MAX),
        Some(UnixTime::from_seconds(Duration::MAX.as_secs_i64()))
    );
}

#[test]
fn the_part_of_a_duration_below_a_second_does_not_move_a_point() {
    let point = UnixTime::from_seconds(10);
    assert_eq!(
        point.checked_add(Duration::from_micros(999_999)),
        Some(point)
    );
    assert_eq!(
        point.checked_add(Duration::from_micros(1_000_001)),
        Some(UnixTime::from_seconds(11))
    );
}

#[test]
fn the_span_between_two_points_is_a_duration() {
    let earlier = UnixTime::from_seconds(100);
    let later = UnixTime::from_seconds(160);
    assert_eq!(
        later.checked_duration_since(earlier),
        Some(Duration::from_secs(60))
    );
    assert_eq!(earlier.checked_duration_since(later), None);
    assert_eq!(
        UnixTime::from_seconds(i64::MAX).checked_duration_since(UnixTime::from_seconds(i64::MIN)),
        None
    );
    // The difference fits an `i64` of seconds but not a `Duration` of
    // microseconds.
    assert_eq!(
        UnixTime::from_seconds(i64::MAX).checked_duration_since(UnixTime::EPOCH),
        None
    );
}

#[test]
fn the_order_of_two_points_is_the_chronological_one() {
    assert!(UnixTime::from_seconds(-1) < UnixTime::EPOCH);
    assert!(UnixTime::EPOCH < UnixTime::from_seconds(1));
    assert_eq!(UnixTime::default(), UnixTime::EPOCH);
}

#[test]
fn the_two_conversions_are_available_as_try_from() {
    let time = CivilTime::new(2023, 6, 15, 12, 30, 45).expect("a valid time");
    let point = UnixTime::try_from(time).expect("a point inside the calendar");
    assert_eq!(CivilTime::try_from(point), Ok(time));
}

#[test]
fn every_point_of_the_calendar_round_trips_through_a_civil_time() {
    check("unix time round trips", &any_unix_time(), |&point| {
        let time = point.to_civil().map_err(|error| error.to_string())?;
        time.validate().map_err(|error| error.to_string())?;
        let back = UnixTime::from_civil(time).map_err(|error| error.to_string())?;
        if back == point {
            Ok(())
        } else {
            Err(format!("{point:?} came back as {back:?}"))
        }
    });
}

#[test]
fn the_generator_stays_inside_the_calendar() {
    let first = UnixTime::from_seconds(FIRST_DAY.saturating_mul(86_400));
    let last = UnixTime::from_seconds(LAST_DAY.saturating_mul(86_400).saturating_add(86_399));
    check(
        "the generated point is representable",
        &any_unix_time(),
        |&point| {
            if point >= first && point <= last {
                Ok(())
            } else {
                Err(format!("{point:?} is outside the calendar"))
            }
        },
    );
}
