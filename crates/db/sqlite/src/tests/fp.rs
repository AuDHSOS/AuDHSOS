// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::fp`, against the C library's own rendering.
//!
//! `fixtures/fp.corpus` is eight thousand doubles, one per line as the
//! sixteen hex digits of its bits: the values a reader would pick by hand,
//! every power of ten with two neighbours, four hundred quotients, the
//! powers of two across the whole exponent range, and three thousand
//! arbitrary bit patterns from a fixed seed. `fixtures/fp.golden` is what
//! `sqlite3_mprintf` answered for each at three precisions, written by
//! `tools/sqlite-oracle.c`. The test is the comparison.

#![allow(clippy::arithmetic_side_effects)]

use crate::fp::{DIGITS, MAX_DIGITS, Special, decode, text};

/// The doubles of the corpus.
fn corpus() -> Vec<f64> {
    include_str!("fixtures/fp.corpus")
        .lines()
        .map(|line| f64::from_bits(u64::from_str_radix(line, 16).unwrap()))
        .collect()
}

/// What SQLite answered, three renderings per case.
fn golden() -> Vec<[&'static str; 3]> {
    include_str!("fixtures/fp.golden")
        .lines()
        .map(|line| {
            let mut fields = line.split('\t');
            let mut next = || fields.next().unwrap();
            [next(), next(), next()]
        })
        .collect()
}

/// The rendering, as text.
fn rendered(value: f64, significant: i32) -> String {
    String::from_utf8(text(value, significant)).unwrap()
}

#[test]
fn every_double_renders_the_way_the_c_library_renders_it() {
    let cases = corpus();
    let answers = golden();
    assert_eq!(cases.len(), answers.len());
    for (value, answer) in cases.iter().zip(answers) {
        for (at, significant) in [4, DIGITS, MAX_DIGITS].into_iter().enumerate() {
            assert_eq!(
                rendered(*value, significant),
                answer[at],
                "{:016x} at {significant} digits",
                value.to_bits()
            );
        }
    }
}

#[test]
fn the_renderings_read_back_as_the_double_they_were_written_from() {
    // Seventeen digits is enough to round-trip any double, which is why
    // it is the number SQLite prints.
    for value in corpus() {
        if !value.is_finite() {
            continue;
        }
        let text = rendered(value, DIGITS);
        assert_eq!(text.parse::<f64>().unwrap(), value, "{text}");
    }
}

#[test]
fn a_precision_outside_the_range_is_pulled_into_it() {
    for value in [0.0, 1.0, 49.47, 1.0 / 3.0] {
        assert_eq!(rendered(value, 0), rendered(value, 1));
        assert_eq!(rendered(value, -100), rendered(value, 1));
        assert_eq!(rendered(value, 1000), rendered(value, MAX_DIGITS));
    }
}

#[test]
fn the_specials_are_told_apart() {
    assert_eq!(decode(f64::NAN, DIGITS).special, Special::NotANumber);
    assert_eq!(decode(f64::INFINITY, DIGITS).special, Special::Infinity);
    assert_eq!(decode(f64::NEG_INFINITY, DIGITS).special, Special::Infinity);
    assert!(decode(f64::NEG_INFINITY, DIGITS).negative);
    assert_eq!(decode(1.5, DIGITS).special, Special::None);
}

#[test]
fn zero_is_one_digit_with_the_point_after_it() {
    let zero = decode(0.0, DIGITS);
    assert_eq!(zero.digits, b"0");
    assert_eq!(zero.point, 1);
    assert!(!zero.negative);
    // Negative zero is a double but not a sign: SQLite prints "0.0".
    assert!(!decode(-0.0, DIGITS).negative);
}

#[test]
fn rounding_up_the_last_digit_can_carry_into_a_new_one() {
    // Nine hundred and ninety-nine thousandths to two digits is "1.0",
    // which is one digit more than was asked for.
    assert_eq!(rendered(0.999, 2), "1.0");
    assert_eq!(rendered(9.99, 2), "10.0");
    assert_eq!(decode(0.999, 2).point, 1);
}

#[test]
fn a_power_of_ten_outside_the_table_is_zero_or_an_infinity() {
    use crate::fp::from_digits;
    // The table reaches from 1.0e-348 to 1.0e+347. Past either end the
    // answer is the limit rather than a wrong number.
    assert_eq!(from_digits(1, -349), 0.0);
    assert_eq!(from_digits(1, 348), f64::INFINITY);
    // Inside the table, but past what a double holds.
    assert_eq!(from_digits(1, -348), 0.0);
    assert_eq!(from_digits(u64::MAX, 347), f64::INFINITY);
}

#[test]
fn digits_and_a_power_read_back_as_the_double_they_came_from() {
    use crate::fp::from_digits;
    assert_eq!(from_digits(1, 0), 1.0);
    assert_eq!(from_digits(3, -1), 0.3);
    assert_eq!(from_digits(4947, -2), 49.47);
    assert_eq!(from_digits(1, 308), 1e308);
    assert_eq!(from_digits(1, -323), 1e-323);
    for value in corpus() {
        if !value.is_finite() || value == 0.0 {
            continue;
        }
        let decoded = decode(value, DIGITS);
        let mut digits: u64 = 0;
        for byte in &decoded.digits {
            digits = digits * 10 + u64::from(byte - b'0');
        }
        let power = decoded.point - i32::try_from(decoded.digits.len()).unwrap();
        let back = from_digits(digits, power);
        assert_eq!(back, value.abs(), "{:016x}", value.to_bits());
    }
}
