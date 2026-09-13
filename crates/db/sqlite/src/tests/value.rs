// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::value` where an expression cannot reach it.
//!
//! What an expression reaches is checked against the C library in
//! `tests/eval.rs`, over seven thousand recorded cases. What is here is
//! the rest of the module: the conversions a column does on the way in,
//! which the column layer will call and no expression does, and the
//! answers at the edges of what a double and an integer hold.

#![allow(clippy::arithmetic_side_effects)]

use crate::value::{
    Affinity, Class, Collation, Value, apply, apply_comparison, compare, compare_affinity,
    integer_affinity, integer_as_real, real_as_integer,
};

/// The value after the affinity is applied to it.
fn applied(value: Value, affinity: Affinity) -> Value {
    let mut value = value;
    apply(&mut value, affinity);
    value
}

#[test]
fn every_value_is_in_the_class_it_sorts_by() {
    assert_eq!(Value::Null.class(), Class::Null);
    assert_eq!(Value::Int(1).class(), Class::Number);
    assert_eq!(Value::Real(1.0).class(), Class::Number);
    assert_eq!(Value::Text(b"a".to_vec()).class(), Class::Text);
    assert_eq!(Value::Blob(b"a".to_vec()).class(), Class::Blob);
    assert!(Class::Null < Class::Number);
    assert!(Class::Number < Class::Text);
    assert!(Class::Text < Class::Blob);
}

#[test]
fn nothing_reads_as_zero() {
    assert_eq!(Value::Null.to_real(), 0.0);
    assert_eq!(Value::Null.to_integer(), 0);
    assert_eq!(Value::Null.stringify(), None);
    assert_eq!(Value::Null.bytes(), None);
}

#[test]
fn nothing_is_whatever_the_caller_says_it_is() {
    assert!(Value::Null.truth(true));
    assert!(!Value::Null.truth(false));
    assert!(Value::Int(1).truth(false));
    assert!(!Value::Int(0).truth(true));
    assert!(Value::Text(b"1".to_vec()).truth(false));
    assert!(!Value::Text(b"abc".to_vec()).truth(true));
}

#[test]
fn nothing_sorts_before_everything() {
    use core::cmp::Ordering;
    assert_eq!(
        compare(&Value::Null, &Value::Null, Collation::Binary),
        Ordering::Equal
    );
    assert_eq!(
        compare(&Value::Null, &Value::Int(0), Collation::Binary),
        Ordering::Less
    );
    assert_eq!(
        compare(&Value::Int(0), &Value::Null, Collation::Binary),
        Ordering::Greater
    );
}

#[test]
fn a_double_becomes_the_integer_it_is_nearest_below() {
    assert_eq!(real_as_integer(2.9), 2);
    assert_eq!(real_as_integer(-2.9), -2);
    assert_eq!(real_as_integer(0.5), 0);
    assert_eq!(real_as_integer(-0.5), 0);
    // Below one is a subnormal, which truncates to zero like any other
    // fraction.
    assert_eq!(real_as_integer(f64::MIN_POSITIVE / 2.0), 0);
    // Past what an integer holds the answer is the end of the range.
    assert_eq!(real_as_integer(1e300), i64::MAX);
    assert_eq!(real_as_integer(-1e300), i64::MIN);
    assert_eq!(real_as_integer(f64::INFINITY), i64::MAX);
    assert_eq!(real_as_integer(f64::NEG_INFINITY), i64::MIN);
    // A NaN is not a number SQLite computes with; zero is the answer
    // that keeps the conversion total.
    assert_eq!(real_as_integer(f64::NAN), 0);
    // The largest double the range check lets through converts exactly,
    // and it is not the largest integer.
    assert_eq!(
        real_as_integer(9_223_372_036_854_774_784.0),
        9_223_372_036_854_774_784
    );
    assert_eq!(real_as_integer(-9_223_372_036_854_775_808.0), i64::MIN);
}

#[test]
fn an_integer_becomes_the_double_nearest_it() {
    assert_eq!(integer_as_real(0), 0.0);
    assert_eq!(integer_as_real(1), 1.0);
    assert_eq!(integer_as_real(-1), -1.0);
    assert_eq!(integer_as_real(i64::MAX), 9_223_372_036_854_775_808.0);
    assert_eq!(integer_as_real(i64::MIN), -9_223_372_036_854_775_808.0);
    // Exact below two to the fifty-third, rounded above it.
    assert_eq!(
        integer_as_real(9_007_199_254_740_993),
        9_007_199_254_740_992.0
    );
}

#[test]
fn a_column_converts_what_it_can_and_leaves_the_rest() {
    // These are `applyAffinity`, which is what storing a value in a
    // column of that affinity does. A REAL column runs one more step
    // after it, which is where `3` becomes `3.0`.
    assert_eq!(
        applied(Value::Text(b"3.0".to_vec()), Affinity::Integer),
        Value::Int(3)
    );
    assert_eq!(
        applied(Value::Text(b"1".to_vec()), Affinity::Integer),
        Value::Int(1)
    );
    assert_eq!(
        applied(Value::Text(b"0".to_vec()), Affinity::Integer),
        Value::Int(0)
    );
    assert_eq!(
        applied(Value::Text(b"-0".to_vec()), Affinity::Integer),
        Value::Int(0)
    );
    assert_eq!(
        applied(Value::Text(b"3".to_vec()), Affinity::Real),
        Value::Int(3)
    );
    assert_eq!(
        applied(Value::Text(b"1.5".to_vec()), Affinity::Numeric),
        Value::Real(1.5)
    );
    // Text that is not a number is left as it is.
    assert_eq!(
        applied(Value::Text(b"abc".to_vec()), Affinity::Numeric),
        Value::Text(b"abc".to_vec())
    );
    assert_eq!(
        applied(Value::Text(b"1 x".to_vec()), Affinity::Numeric),
        Value::Text(b"1 x".to_vec())
    );
    // Where the double no longer holds every integer, the digits are
    // read again rather than the double being trusted.
    assert_eq!(
        applied(Value::Text(b"9007199254740993".to_vec()), Affinity::Integer),
        Value::Int(9_007_199_254_740_993)
    );
    // A number past what an integer holds stays a double.
    assert_eq!(
        applied(
            Value::Text(b"9223372036854775808".to_vec()),
            Affinity::Integer
        ),
        Value::Real(9_223_372_036_854_775_808.0)
    );
    // Numbers already in a class the affinity asks for.
    assert_eq!(applied(Value::Int(3), Affinity::Real), Value::Int(3));
    assert_eq!(applied(Value::Real(3.0), Affinity::Integer), Value::Int(3));
    assert_eq!(
        applied(Value::Real(1.5), Affinity::Integer),
        Value::Real(1.5)
    );
    // Text affinity writes a number out and leaves everything else.
    assert_eq!(
        applied(Value::Int(3), Affinity::Text),
        Value::Text(b"3".to_vec())
    );
    assert_eq!(
        applied(Value::Real(3.5), Affinity::Text),
        Value::Text(b"3.5".to_vec())
    );
    assert_eq!(
        applied(Value::Blob(b"1".to_vec()), Affinity::Text),
        Value::Blob(b"1".to_vec())
    );
    assert_eq!(applied(Value::Null, Affinity::Text), Value::Null);
    // Neither of the two that convert nothing.
    assert_eq!(applied(Value::Int(3), Affinity::Blob), Value::Int(3));
    assert_eq!(
        applied(Value::Text(b"1".to_vec()), Affinity::None),
        Value::Text(b"1".to_vec())
    );
    assert_eq!(applied(Value::Null, Affinity::Numeric), Value::Null);
    assert_eq!(
        applied(Value::Blob(b"1".to_vec()), Affinity::Numeric),
        Value::Blob(b"1".to_vec())
    );
}

#[test]
fn a_double_becomes_an_integer_only_where_nothing_is_lost() {
    let integral = |value: Value| {
        let mut value = value;
        integer_affinity(&mut value);
        value
    };
    assert_eq!(integral(Value::Real(3.0)), Value::Int(3));
    assert_eq!(integral(Value::Real(-3.0)), Value::Int(-3));
    assert_eq!(integral(Value::Real(3.5)), Value::Real(3.5));
    // The two ends of the range are left alone, because adding to either
    // of them wraps.
    assert_eq!(
        integral(Value::Real(9_223_372_036_854_775_808.0)),
        Value::Real(9_223_372_036_854_775_808.0)
    );
    assert_eq!(
        integral(Value::Real(-9_223_372_036_854_775_808.0)),
        Value::Real(-9_223_372_036_854_775_808.0)
    );
    // Anything that is not a double already.
    assert_eq!(integral(Value::Int(3)), Value::Int(3));
    assert_eq!(integral(Value::Null), Value::Null);
}

#[test]
fn the_affinity_of_a_comparison_is_the_column_side() {
    use Affinity::{Blob, Integer, None, Numeric, Real, Text};
    assert_eq!(compare_affinity(None, None), None);
    assert_eq!(compare_affinity(Text, None), Text);
    assert_eq!(compare_affinity(None, Text), Text);
    assert_eq!(compare_affinity(None, Blob), Blob);
    // Two columns: a number beats text, and text against a blob is
    // neither.
    assert_eq!(compare_affinity(Text, Text), Blob);
    assert_eq!(compare_affinity(Text, Integer), Numeric);
    assert_eq!(compare_affinity(Numeric, Text), Numeric);
    assert_eq!(compare_affinity(Blob, Real), Numeric);
    assert_eq!(compare_affinity(Blob, Blob), Blob);
}

#[test]
fn a_type_name_names_an_affinity_by_the_letters_in_it() {
    assert_eq!(Affinity::of_type(b"INTEGER"), Affinity::Integer);
    assert_eq!(Affinity::of_type(b"int"), Affinity::Integer);
    assert_eq!(Affinity::of_type(b"POINT"), Affinity::Integer);
    assert_eq!(Affinity::of_type(b"VARCHAR(255)"), Affinity::Text);
    assert_eq!(Affinity::of_type(b"CLOB"), Affinity::Text);
    assert_eq!(Affinity::of_type(b"BLOB"), Affinity::Blob);
    assert_eq!(Affinity::of_type(b"REAL"), Affinity::Real);
    assert_eq!(Affinity::of_type(b"FLOATING"), Affinity::Real);
    assert_eq!(Affinity::of_type(b"DOUBLE"), Affinity::Real);
    assert_eq!(Affinity::of_type(b"DECIMAL(10,5)"), Affinity::Numeric);
    assert_eq!(Affinity::of_type(b""), Affinity::Numeric);
    // The order the letters come in decides, not which word is longer.
    assert_eq!(Affinity::of_type(b"BLOBREAL"), Affinity::Blob);
    assert_eq!(Affinity::of_type(b"REALBLOB"), Affinity::Blob);
    assert_eq!(Affinity::of_type(b"TEXTBLOB"), Affinity::Text);
    assert_eq!(Affinity::of_type(b"BLOBTEXT"), Affinity::Text);
    assert_eq!(Affinity::of_type(b"CHARREAL"), Affinity::Text);
    assert!(Affinity::Numeric.numeric());
    assert!(!Affinity::Text.numeric());
}

#[test]
fn the_three_collations_are_found_by_name_whatever_case_it_is_in() {
    assert_eq!(Collation::of_name(b"binary"), Some(Collation::Binary));
    assert_eq!(Collation::of_name(b"NOCASE"), Some(Collation::NoCase));
    assert_eq!(Collation::of_name(b"RTrim"), Some(Collation::Rtrim));
    assert_eq!(Collation::of_name(b"german"), None);
    assert_eq!(Collation::default(), Collation::Binary);
}

#[test]
fn text_compares_under_the_collation_it_is_given() {
    use core::cmp::Ordering;
    let text = |bytes: &[u8]| Value::Text(bytes.to_vec());
    assert_eq!(
        compare(&text(b"a"), &text(b"A"), Collation::Binary),
        Ordering::Greater
    );
    assert_eq!(
        compare(&text(b"a"), &text(b"A"), Collation::NoCase),
        Ordering::Equal
    );
    assert_eq!(
        compare(&text(b"a "), &text(b"a"), Collation::Rtrim),
        Ordering::Equal
    );
    assert_eq!(
        compare(&text(b"a "), &text(b"a"), Collation::Binary),
        Ordering::Greater
    );
    assert_eq!(
        compare(&text(b"ab"), &text(b"aa"), Collation::Rtrim),
        Ordering::Greater
    );
}

#[test]
fn a_number_and_a_double_compare_without_either_losing_precision() {
    use core::cmp::Ordering;
    // The integer and the double nearest it are not the same number, and
    // comparing them as doubles would say they are.
    assert_eq!(
        compare(&Value::Int(i64::MAX), &Value::Real(9e18), Collation::Binary),
        Ordering::Greater
    );
    assert_eq!(
        compare(&Value::Real(9e18), &Value::Int(i64::MAX), Collation::Binary),
        Ordering::Less
    );
    assert_eq!(
        compare(&Value::Int(1), &Value::Real(1.0), Collation::Binary),
        Ordering::Equal
    );
    assert_eq!(
        compare(&Value::Int(1), &Value::Real(1.5), Collation::Binary),
        Ordering::Less
    );
    // Past either end of what an integer holds.
    assert_eq!(
        compare(&Value::Int(0), &Value::Real(1e300), Collation::Binary),
        Ordering::Less
    );
    assert_eq!(
        compare(&Value::Int(0), &Value::Real(-1e300), Collation::Binary),
        Ordering::Greater
    );
    // A NaN is read as nothing, and every number is above nothing.
    assert_eq!(
        compare(&Value::Int(0), &Value::Real(f64::NAN), Collation::Binary),
        Ordering::Greater
    );
    assert_eq!(
        compare(&Value::Real(f64::NAN), &Value::Int(0), Collation::Binary),
        Ordering::Less
    );
}

#[test]
fn a_comparison_writes_a_number_out_where_the_other_side_is_text() {
    let mut left = Value::Int(1);
    let mut right = Value::Text(b"1".to_vec());
    apply_comparison(&mut left, &mut right, Affinity::Text);
    assert_eq!(left, Value::Text(b"1".to_vec()));
    assert_eq!(right, Value::Text(b"1".to_vec()));
    // With no text on either side there is nothing to write out.
    let mut left = Value::Int(1);
    let mut right = Value::Real(1.0);
    apply_comparison(&mut left, &mut right, Affinity::Text);
    assert_eq!(left, Value::Int(1));
    assert_eq!(right, Value::Real(1.0));
    // The same for a numeric affinity.
    let mut left = Value::Int(1);
    let mut right = Value::Blob(b"1".to_vec());
    apply_comparison(&mut left, &mut right, Affinity::Numeric);
    assert_eq!(right, Value::Blob(b"1".to_vec()));
}
