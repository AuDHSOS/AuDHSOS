// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The two time forms, as far as this crate checks them.

use audhsos_time::{CivilTime, UnixTime};

use crate::error::DerError;
use crate::reader::Reader;
use crate::time::{from_generalized_time, from_utc_time};

/// A time value with the given tag and text.
fn encode(tag: u8, text: &str) -> Vec<u8> {
    let content = text.as_bytes();
    let mut out = vec![tag, u8::try_from(content.len()).unwrap_or(0)];
    out.extend_from_slice(content);
    out
}

#[test]
fn a_utc_time_is_read_with_its_two_digit_year_window() {
    let value = from_utc_time(b"230405060708Z").expect("a well formed time");
    assert_eq!(
        value,
        CivilTime {
            year: 2023,
            month: 4,
            day: 5,
            hour: 6,
            minute: 7,
            second: 8,
        }
    );

    // RFC 5280: a two-digit year of fifty or more is nineteen hundred,
    // below it two thousand.
    assert_eq!(
        from_utc_time(b"500101000000Z")
            .expect("a well formed time")
            .year,
        1950
    );
    assert_eq!(
        from_utc_time(b"490101000000Z")
            .expect("a well formed time")
            .year,
        2049
    );
    assert_eq!(
        from_utc_time(b"991231235959Z")
            .expect("a well formed time")
            .year,
        1999
    );
}

#[test]
fn a_generalized_time_carries_its_century() {
    let value = from_generalized_time(b"20230405060708Z").expect("a well formed time");
    assert_eq!(value.year, 2023);
    assert_eq!(value.second, 8);
    assert_eq!(
        from_generalized_time(b"19500101000000Z")
            .expect("a well formed time")
            .year,
        1950
    );
}

#[test]
fn the_forms_the_profile_forbids_are_refused() {
    for text in [
        "2304050607Z",       // no seconds
        "230405060708",      // no zone
        "230405060708z",     // a lower-case zone
        "230405060708+0000", // an offset instead of the zone
        "230405060708Z ",    // trailing space
        "2304050607O8Z",     // a letter where a digit belongs
        "",                  // empty
    ] {
        assert_eq!(
            from_utc_time(text.as_bytes()),
            Err(DerError::BadTime),
            "utc time {text:?}"
        );
    }

    for text in [
        "20230405060708",      // no zone
        "202304050607Z",       // no seconds
        "20230405060708.5Z",   // fractional seconds
        "20230405060708+0100", // an offset
        "230405060708Z",       // the two-digit form
    ] {
        assert_eq!(
            from_generalized_time(text.as_bytes()),
            Err(DerError::BadTime),
            "generalized time {text:?}"
        );
    }
}

#[test]
fn a_field_out_of_range_is_refused() {
    for text in [
        "230005060708Z", // month zero
        "231305060708Z", // month thirteen
        "230400060708Z", // day zero
        "230432060708Z", // day thirty-two
        "230405240708Z", // hour twenty-four
        "230405066008Z", // minute sixty
        "230405060760Z", // second sixty
    ] {
        assert_eq!(
            from_utc_time(text.as_bytes()),
            Err(DerError::BadTime),
            "{text:?}"
        );
    }
}

#[test]
fn a_day_beyond_the_length_of_its_month_is_refused() {
    // The seam of decision D-46 is closed: the day goes to the calendar of
    // `audhsos-time`, which knows how long its month is.
    for text in [
        "230231000000Z", // the thirty-first of February
        "230431000000Z", // the thirty-first of April
        "230229000000Z", // the twenty-ninth of a year that is not leap
    ] {
        assert_eq!(
            from_utc_time(text.as_bytes()),
            Err(DerError::BadTime),
            "{text:?}"
        );
    }
    assert!(from_utc_time(b"240229000000Z").is_ok(), "a leap year");
    assert!(
        from_generalized_time(b"20000229000000Z").is_ok(),
        "a leap century"
    );
    assert_eq!(
        from_generalized_time(b"19000229000000Z"),
        Err(DerError::BadTime),
        "a century that is not leap"
    );
}

#[test]
fn a_time_the_parser_accepts_is_a_point_on_the_unix_scale() {
    // What the seam was for: the fields become an instant without a second
    // parser and without a second calendar.
    let time = from_generalized_time(b"20010909014640Z").expect("a well formed time");
    assert_eq!(
        UnixTime::from_civil(time),
        Ok(UnixTime::from_seconds(1_000_000_000))
    );
}

#[test]
fn the_reader_takes_either_form_and_refuses_a_third() {
    let utc = encode(0x17, "230405060708Z");
    assert_eq!(
        Reader::new(&utc)
            .read_time()
            .expect("a well formed time")
            .year,
        2023
    );

    let generalized = encode(0x18, "20230405060708Z");
    assert_eq!(
        Reader::new(&generalized)
            .read_time()
            .expect("a well formed time")
            .year,
        2023
    );

    let other = encode(0x04, "230405060708Z");
    assert_eq!(
        Reader::new(&other).read_time(),
        Err(DerError::UnexpectedTag)
    );
    assert_eq!(Reader::new(&[]).read_time(), Err(DerError::EndOfInput));

    let malformed = encode(0x17, "not a time!!!");
    assert_eq!(Reader::new(&malformed).read_time(), Err(DerError::BadTime));
}

#[test]
fn times_compare_in_the_order_they_happen() {
    let earlier = from_utc_time(b"230405060708Z").expect("a well formed time");
    let later = from_utc_time(b"230405060709Z").expect("a well formed time");
    let much_later = from_generalized_time(b"20240405060708Z").expect("a well formed time");
    assert!(earlier < later);
    assert!(later < much_later);
    assert!(!format!("{earlier:?}").is_empty());
}
