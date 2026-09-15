// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::value`.

use crate::value::{Value, tuple};

#[test]
fn a_null_and_an_integer_are_written_as_they_stand() {
    assert_eq!(Value::Null.literal(), "NULL");
    assert_eq!(Value::Int(-3).literal(), "-3");
    assert_eq!(Value::Int(i64::MIN).literal(), "-9223372036854775808");
}

#[test]
fn a_double_keeps_a_decimal_point_so_it_reads_back_as_one() {
    assert_eq!(Value::Real(2.0).literal(), "2.0");
    assert_eq!(Value::Real(0.5).literal(), "0.5");
    assert_eq!(Value::Real(1e300).literal(), "1e300");
}

#[test]
fn a_double_that_is_not_finite_is_written_as_null() {
    assert_eq!(Value::Real(f64::NAN).literal(), "NULL");
    assert_eq!(Value::Real(f64::INFINITY).literal(), "NULL");
}

#[test]
fn a_quote_in_text_is_doubled_and_nothing_else_is_escaped() {
    assert_eq!(Value::Text("a'b".to_owned()).literal(), "'a''b'");
    assert_eq!(Value::Text(String::new()).literal(), "''");
    assert_eq!(Value::Text("a\\b".to_owned()).literal(), "'a\\b'");
}

#[test]
fn a_row_is_written_as_a_parenthesized_list() {
    let row = vec![Value::Int(1), Value::Null, Value::Text("x".to_owned())];
    assert_eq!(tuple(&row), "(1, NULL, 'x')");
    assert_eq!(tuple(&[]), "()");
}
