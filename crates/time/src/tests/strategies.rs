// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The generators: every value they produce is one the crate accepts.

use test_support::property::check;

use crate::civil::days_from_civil;
use crate::strategies::{FIRST_DAY, LAST_DAY, any_civil_time, any_day, any_duration, any_instant};

#[test]
fn a_generated_day_is_one_the_calendar_covers() {
    check("the generated day is in range", &any_day(), |&day| {
        if (FIRST_DAY..=LAST_DAY).contains(&day) {
            Ok(())
        } else {
            Err(format!("day {day} is outside the calendar"))
        }
    });
}

#[test]
fn a_generated_civil_time_validates() {
    check("the generated time validates", &any_civil_time(), |time| {
        time.validate()
            .map_err(|error| format!("{time:?}: {error}"))?;
        days_from_civil(time.year, time.month, time.day)
            .map(|_| ())
            .map_err(|error| format!("{time:?}: {error}"))
    });
}

#[test]
fn a_generated_instant_leaves_room_for_a_generated_duration() {
    check(
        "the generated instant does not saturate",
        &any_instant(),
        |&point| {
            if point.checked_add(any_longest_duration()).is_some() {
                Ok(())
            } else {
                Err(format!("{point:?} leaves no room"))
            }
        },
    );
}

/// The longest span [`any_duration`] produces.
fn any_longest_duration() -> crate::instant::Duration {
    crate::instant::Duration::from_secs(86_400)
}

#[test]
fn a_generated_duration_is_at_most_a_day() {
    check(
        "the generated span is at most a day",
        &any_duration(),
        |&span| {
            if span <= any_longest_duration() {
                Ok(())
            } else {
                Err(format!("{span:?} is longer than a day"))
            }
        },
    );
}
