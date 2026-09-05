// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The calendar: the two conversions, the leap-year rule, and the range of
//! every field.

use test_support::generators::range;
use test_support::property::check;

use crate::civil::{
    CivilTime, MAX_YEAR, MIN_YEAR, civil_from_days, days_from_civil, days_in_month, is_leap_year,
};
use crate::error::TimeError;
use crate::strategies::{FIRST_DAY, LAST_DAY};

/// The day number of 1601-01-01, the lower end the catalog names.
const YEAR_1601: i64 = -134_774;

#[test]
fn the_epoch_is_day_zero() {
    assert_eq!(days_from_civil(1970, 1, 1), Ok(0));
    assert_eq!(civil_from_days(0), Ok((1970, 1, 1)));
}

#[test]
fn the_calendar_agrees_with_hand_computed_days() {
    // The day before the epoch, the leap day of a year that is leap
    // although it is a century, and the first of March of the two
    // centuries that are not leap.
    for (date, day) in [
        ((1969, 12, 31), -1),
        ((2000, 2, 29), 11_016),
        ((1900, 3, 1), -25_508),
        ((2100, 3, 1), 47_541),
        ((1601, 1, 1), YEAR_1601),
        ((9999, 12, 31), LAST_DAY),
        ((0, 1, 1), FIRST_DAY),
        // The first day of the first era, which the March-based
        // arithmetic counts from.
        ((0, 3, 1), -719_468),
    ] {
        let (year, month, day_of_month) = date;
        assert_eq!(
            days_from_civil(year, month, day_of_month),
            Ok(day),
            "{year:04}-{month:02}-{day_of_month:02}"
        );
        assert_eq!(civil_from_days(day), Ok(date), "day {day}");
    }
}

#[test]
fn every_day_of_the_calendar_round_trips() {
    check(
        "civil_from_days inverts days_from_civil",
        &range(YEAR_1601..=LAST_DAY),
        |&day| {
            let (year, month, day_of_month) =
                civil_from_days(day).map_err(|error| format!("{day}: {error}"))?;
            let back = days_from_civil(year, month, day_of_month)
                .map_err(|error| format!("{year:04}-{month:02}-{day_of_month:02}: {error}"))?;
            if back == day {
                Ok(())
            } else {
                Err(format!("day {day} came back as {back}"))
            }
        },
    );
}

#[test]
fn the_calendar_advances_by_one_day_at_a_time() {
    // Stronger than the round trip: every day from 1601-01-01 to
    // 9999-12-31 is the successor of the one before it, so a month length
    // that were wrong by a day would show here even though the round trip
    // through the same wrong length would still close.
    let mut previous = civil_from_days(YEAR_1601).expect("the first day is in the calendar");
    for day in YEAR_1601.saturating_add(1)..=LAST_DAY {
        let current = civil_from_days(day).expect("a day inside the calendar");
        let (year, month, day_of_month) = previous;
        let length = days_in_month(year, month).expect("a validated month");
        let expected = if day_of_month < length {
            (year, month, day_of_month.saturating_add(1))
        } else if month < 12 {
            (year, month.saturating_add(1), 1)
        } else {
            (year.saturating_add(1), 1, 1)
        };
        assert_eq!(current, expected, "after {previous:?}");
        previous = current;
    }
    assert_eq!(previous, (9999, 12, 31));
}

#[test]
fn a_century_is_leap_only_every_fourth_time() {
    assert!(!is_leap_year(1900));
    assert!(!is_leap_year(2100));
    assert!(is_leap_year(2000));
    assert!(is_leap_year(2400));
    assert!(is_leap_year(2024));
    assert!(!is_leap_year(2023));
}

#[test]
fn february_has_twenty_eight_or_twenty_nine_days() {
    assert_eq!(days_in_month(1900, 2), Ok(28));
    assert_eq!(days_in_month(2100, 2), Ok(28));
    assert_eq!(days_in_month(2000, 2), Ok(29));
    assert_eq!(days_in_month(2400, 2), Ok(29));
    assert_eq!(days_in_month(2024, 2), Ok(29));
}

#[test]
fn every_month_has_the_length_the_rule_gives_it() {
    for (month, length) in [
        (1, 31),
        (3, 31),
        (4, 30),
        (5, 31),
        (6, 30),
        (7, 31),
        (8, 31),
        (9, 30),
        (10, 31),
        (11, 30),
        (12, 31),
    ] {
        assert_eq!(days_in_month(2023, month), Ok(length), "month {month}");
    }
}

#[test]
fn a_day_of_zero_or_beyond_its_month_is_rejected() {
    assert_eq!(days_from_civil(2023, 1, 0), Err(TimeError::Day(0)));
    assert_eq!(days_from_civil(2023, 1, 32), Err(TimeError::Day(32)));
    assert_eq!(days_from_civil(2023, 4, 31), Err(TimeError::Day(31)));
    assert_eq!(days_from_civil(2023, 2, 29), Err(TimeError::Day(29)));
    assert_eq!(days_from_civil(2024, 2, 30), Err(TimeError::Day(30)));
    assert_eq!(days_from_civil(2024, 2, 29), Ok(19_782));
}

#[test]
fn a_month_outside_one_to_twelve_is_rejected() {
    assert_eq!(days_in_month(2023, 0), Err(TimeError::Month(0)));
    assert_eq!(days_in_month(2023, 13), Err(TimeError::Month(13)));
    assert_eq!(days_from_civil(2023, 0, 1), Err(TimeError::Month(0)));
    assert_eq!(days_from_civil(2023, 13, 1), Err(TimeError::Month(13)));
}

#[test]
fn a_year_outside_the_calendar_is_rejected() {
    assert_eq!(days_in_month(-1, 1), Err(TimeError::Year(-1)));
    assert_eq!(
        days_in_month(MAX_YEAR.saturating_add(1), 1),
        Err(TimeError::Year(10_000))
    );
    assert_eq!(days_from_civil(-1, 1, 1), Err(TimeError::Year(-1)));
    assert!(days_in_month(MIN_YEAR, 1).is_ok());
}

#[test]
fn the_fields_of_a_time_of_day_are_checked() {
    assert_eq!(
        CivilTime::new(2023, 1, 1, 24, 0, 0),
        Err(TimeError::Hour(24))
    );
    assert_eq!(
        CivilTime::new(2023, 1, 1, 0, 60, 0),
        Err(TimeError::Minute(60))
    );
    assert_eq!(
        CivilTime::new(2023, 1, 1, 0, 0, 60),
        Err(TimeError::Second(60))
    );
    assert!(CivilTime::new(2023, 1, 1, 23, 59, 59).is_ok());
    assert!(CivilTime::new(2023, 1, 1, 0, 0, 0).is_ok());
}

#[test]
fn validate_reports_the_first_field_that_is_wrong() {
    let value = CivilTime {
        year: -1,
        month: 13,
        day: 32,
        hour: 24,
        minute: 60,
        second: 60,
    };
    assert_eq!(value.validate(), Err(TimeError::Year(-1)));
    assert_eq!(
        CivilTime {
            year: 2023,
            ..value
        }
        .validate(),
        Err(TimeError::Month(13))
    );
    assert_eq!(
        CivilTime {
            year: 2023,
            month: 1,
            ..value
        }
        .validate(),
        Err(TimeError::Day(32))
    );
}

#[test]
fn the_epoch_constant_validates() {
    assert_eq!(CivilTime::EPOCH.validate(), Ok(()));
    assert_eq!(CivilTime::EPOCH.seconds_of_day(), Ok(0));
}

#[test]
fn the_seconds_of_a_day_are_the_three_fields() {
    let time = CivilTime::new(2023, 1, 1, 23, 59, 59).expect("a valid time");
    assert_eq!(time.seconds_of_day(), Ok(86_399));
    let noon = CivilTime::new(2023, 1, 1, 12, 0, 0).expect("a valid time");
    assert_eq!(noon.seconds_of_day(), Ok(43_200));
}

#[test]
fn the_seconds_of_a_day_check_their_own_fields() {
    for (time, error) in [
        (
            CivilTime {
                hour: 24,
                ..CivilTime::EPOCH
            },
            TimeError::Hour(24),
        ),
        (
            CivilTime {
                minute: 60,
                ..CivilTime::EPOCH
            },
            TimeError::Minute(60),
        ),
        (
            CivilTime {
                second: 60,
                ..CivilTime::EPOCH
            },
            TimeError::Second(60),
        ),
    ] {
        assert_eq!(time.seconds_of_day(), Err(error));
    }
}

#[test]
fn a_day_outside_the_calendar_is_out_of_range() {
    assert_eq!(
        civil_from_days(LAST_DAY.saturating_add(1)),
        Err(TimeError::Year(10_000))
    );
    assert_eq!(
        civil_from_days(FIRST_DAY.saturating_sub(1)),
        Err(TimeError::Year(-1))
    );
    assert_eq!(civil_from_days(i64::MAX), Err(TimeError::OutOfRange));
    assert_eq!(civil_from_days(i64::MIN), Err(TimeError::OutOfRange));
}

#[test]
fn the_order_of_two_times_is_the_chronological_one() {
    let earlier = CivilTime::new(2023, 1, 1, 0, 0, 0).expect("a valid time");
    let later = CivilTime::new(2023, 1, 1, 0, 0, 1).expect("a valid time");
    let next_day = CivilTime::new(2023, 1, 2, 0, 0, 0).expect("a valid time");
    let next_year = CivilTime::new(2024, 1, 1, 0, 0, 0).expect("a valid time");
    assert!(earlier < later);
    assert!(later < next_day);
    assert!(next_day < next_year);
}
