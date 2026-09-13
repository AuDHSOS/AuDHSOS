// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::number`, against the routines it is a port of.
//!
//! `fixtures/num.corpus` is two hundred and fifteen pieces of text, one per
//! line as `x` and the bytes in hex: every rule of `sqlite3AtoF` and
//! `sqlite3Atoi64` by hand, the edges of the mantissa and of an integer,
//! digit strings of every length, and a point at every place of one.
//! `fixtures/num.golden` is what the two routines answered, written by
//! `tools/sqlite-oracle.c`. The test compares the code each returned, the
//! double, and the integer.

#![allow(clippy::arithmetic_side_effects)]

use crate::fp::{DIGITS, text};
use crate::number::{Outcome, integer, integer_text, real};

/// The cases, as the bytes they are.
fn corpus() -> Vec<Vec<u8>> {
    include_str!("fixtures/num.corpus")
        .lines()
        .map(|line| {
            line.as_bytes()[1..]
                .chunks(2)
                .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                .collect()
        })
        .collect()
}

/// What the C library answered: the code of each reader, the double as
/// text, and the integer.
fn golden() -> Vec<(i32, &'static str, i32, i64)> {
    include_str!("fixtures/num.golden")
        .lines()
        .map(|line| {
            let mut fields = line.split('\t');
            let mut next = || fields.next().unwrap();
            (
                next().parse().unwrap(),
                next(),
                next().parse().unwrap(),
                next().parse().unwrap(),
            )
        })
        .collect()
}

/// The bits `sqlite3AtoF` answers with, put back together.
fn atof_code(case: &[u8]) -> i32 {
    let read = real(case);
    if !read.number() {
        return 0;
    }
    let bits = 1
        | i32::from(read.fractional()) << 1
        | i32::from(read.zero()) << 2
        | i32::from(read.truncated()) << 3;
    if read.complete() { bits } else { bits | -16 }
}

/// The code `sqlite3Atoi64` answers with.
fn atoi64_code(outcome: Outcome) -> i32 {
    match outcome {
        Outcome::Exact => 0,
        Outcome::Empty => -1,
        Outcome::Trailing => 1,
        Outcome::Overflow => 2,
        Outcome::Limit => 3,
    }
}

#[test]
fn every_case_is_read_the_way_the_c_library_reads_it() {
    let cases = corpus();
    let answers = golden();
    assert_eq!(cases.len(), answers.len());
    for (case, (code, double, whole_code, whole)) in cases.iter().zip(answers) {
        let shown = String::from_utf8_lossy(case).into_owned();
        assert_eq!(atof_code(case), code, "the code for {shown:?}");
        let read = real(case);
        assert_eq!(
            String::from_utf8(text(read.value, DIGITS)).unwrap(),
            double,
            "the double of {shown:?}"
        );
        let read = integer(case);
        assert_eq!(
            atoi64_code(read.outcome),
            whole_code,
            "the code for {shown:?}"
        );
        assert_eq!(read.value, whole, "the integer of {shown:?}");
    }
}

#[test]
fn an_integer_is_written_as_the_digits_it_is_read_from() {
    for value in [
        0,
        1,
        -1,
        9,
        10,
        -10,
        99,
        100,
        i64::MAX,
        i64::MIN,
        i64::MAX - 1,
        i64::MIN + 1,
        1_000_000_000_000_000_000,
    ] {
        let written = integer_text(value);
        assert_eq!(
            String::from_utf8(written.clone()).unwrap(),
            format!("{value}")
        );
        let read = integer(&written);
        assert_eq!(read.value, value);
        assert_eq!(read.outcome, Outcome::Exact);
    }
}
