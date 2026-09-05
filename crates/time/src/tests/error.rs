// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The error type: every variant says which field it means.

use crate::error::TimeError;

#[test]
fn every_variant_names_what_is_wrong() {
    for (error, text) in [
        (TimeError::Year(-1), "the year -1 is outside 0 to 9999"),
        (TimeError::Month(13), "the month 13 is not one to twelve"),
        (
            TimeError::Day(32),
            "the day 32 is not one to the length of its month",
        ),
        (
            TimeError::Hour(24),
            "the hour 24 is not zero to twenty-three",
        ),
        (
            TimeError::Minute(60),
            "the minute 60 is not zero to fifty-nine",
        ),
        (
            TimeError::Second(60),
            "the second 60 is not zero to fifty-nine",
        ),
        (
            TimeError::OutOfRange,
            "the result is outside the range of its type",
        ),
    ] {
        assert_eq!(error.to_string(), text);
    }
}

#[test]
fn two_errors_of_the_same_field_and_value_are_equal() {
    assert_eq!(TimeError::Day(31), TimeError::Day(31));
    assert_ne!(TimeError::Day(31), TimeError::Day(30));
    assert_ne!(TimeError::Day(31), TimeError::Month(31));
}
