// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::report`.

use core::fmt::Write;

use crate::report::Line;

#[test]
fn an_empty_line_holds_nothing() {
    let line: Line<16> = Line::new();
    assert!(line.is_empty());
    assert_eq!(line.len(), 0);
    assert_eq!(line.as_bytes(), b"");
    assert!(!line.is_truncated());
}

#[test]
fn what_was_written_comes_back_in_order() {
    let mut line: Line<16> = Line::new();
    line.put(b"ab");
    line.put(b"cd");
    assert_eq!(line.as_bytes(), b"abcd");
    assert_eq!(line.len(), 4);
    assert!(!line.is_truncated());
}

#[test]
fn a_line_that_does_not_fit_is_cut_and_says_so() {
    let mut line: Line<4> = Line::new();
    line.put(b"abcdef");
    assert_eq!(line.as_bytes(), b"abcd");
    assert!(line.is_truncated());
}

#[test]
fn a_write_into_a_full_line_adds_nothing() {
    let mut line: Line<4> = Line::new();
    line.put(b"abcd");
    assert!(!line.is_truncated());
    line.put(b"e");
    assert_eq!(line.as_bytes(), b"abcd");
    assert!(line.is_truncated());
}

#[test]
fn formatting_fills_a_fresh_line() {
    let line: Line<32> = Line::of(format_args!("{} of {}", 3, 7));
    assert_eq!(line.as_bytes(), b"3 of 7");
    assert!(!line.is_truncated());
}

#[test]
fn formatting_that_does_not_fit_is_cut() {
    let line: Line<4> = Line::of(format_args!("{}", "abcdefgh"));
    assert_eq!(line.as_bytes(), b"abcd");
    assert!(line.is_truncated());
}

#[test]
fn a_line_written_through_the_formatter_is_the_same_line() {
    let mut line: Line<16> = Line::new();
    write!(line, "{:04x}", 0xBEEFu32).unwrap();
    assert_eq!(line.as_bytes(), b"beef");
}

#[test]
fn clearing_forgets_the_bytes_and_the_mark() {
    let mut line: Line<2> = Line::new();
    line.put(b"abc");
    assert!(line.is_truncated());
    line.clear();
    assert!(line.is_empty());
    assert!(!line.is_truncated());
    line.put(b"z");
    assert_eq!(line.as_bytes(), b"z");
}

#[test]
fn a_default_line_is_an_empty_one() {
    let line: Line<8> = Line::default();
    assert!(line.is_empty());
}
