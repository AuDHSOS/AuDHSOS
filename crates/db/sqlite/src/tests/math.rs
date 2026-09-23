// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The functions of the math library, against what the C library answers
//! for the same calls.

use alloc::string::String;

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;
use crate::value::Value;

/// The values one statement answers, each as the tester writes it: a text
/// as it stands, a double to fifteen digits, and nothing as `NULL`.
fn answered(sql: &str) -> String {
    let writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    let held = database.query(sql.as_bytes()).unwrap();
    let mut out = String::new();
    for row in &held.rows {
        for value in row {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(&shown(value));
        }
    }
    out
}

/// One value as the tester writes it.
fn shown(value: &Value) -> String {
    match value {
        Value::Null => String::from("NULL"),
        Value::Text(bytes) | Value::Blob(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        Value::Int(number) => alloc::format!("{number}"),
        Value::Real(number) => String::from_utf8_lossy(&crate::fp::text(*number, 15)).into_owned(),
    }
}

/// Every call answers what the C library answers for it.
fn same_as_the_c_library(cases: &[(&str, &str)]) {
    for (sql, want) in cases {
        assert_eq!(&answered(sql), want, "{sql}");
    }
}

/// `func7-100` through `func7-210` of `test/func7.test`.
#[test]
fn what_the_logarithms_answer() {
    same_as_the_c_library(&[
        (
            "SELECT round(ln(5),2), log(100.0), log(100)",
            "1.61 2.0 2.0",
        ),
        // The base of `log(B,X)` is the first argument, and the second is
        // read as a double whatever it is.
        ("SELECT log(2,'256'), log(2.0, 64.0)", "8.0 6.0"),
        // A logarithm of nought or under it, and a base of one or under
        // it, each answer nothing.
        (
            "SELECT quote(ln(-5)), quote(log(-5,100.0)), quote(ln(0)), quote(log(1,10))",
            "NULL NULL NULL NULL",
        ),
        (
            "SELECT quote(ln('x')), quote(log(2,'x')), quote(log(2,-1))",
            "NULL NULL NULL",
        ),
        // A power of ten and a power of two are answered exactly.
        (
            "SELECT format('%.30f', log10(100.0))",
            "2.000000000000000000000000000000",
        ),
        (
            "SELECT format('%.30f', ln(exp(2.0)))",
            "2.000000000000000000000000000000",
        ),
        ("SELECT log10(1000.0), log2(8), log2(0.5)", "3.0 3.0 -1.0"),
        (
            "SELECT round(ln(2.0),7), round(exp(1.0),7)",
            "0.6931472 2.7182818",
        ),
    ]);
}

/// `func7-pg-*` and `func7-mysql-*` of `test/func7.test`.
#[test]
fn what_the_circular_functions_answer() {
    same_as_the_c_library(&[
        (
            "SELECT acos(1), format('%f',degrees(acos(0.5)))",
            "0.0 60.000000",
        ),
        (
            "SELECT round(asin(1),7), format('%f',degrees(asin(0.5)))",
            "1.5707963 30.000000",
        ),
        (
            "SELECT round(atan(1),7), degrees(atan(1))",
            "0.7853982 45.0",
        ),
        (
            "SELECT round(atan2(1,0),7), degrees(atan2(1,0))",
            "1.5707963 90.0",
        ),
        ("SELECT cos(0), cos(radians(60.0))", "1.0 0.5"),
        ("SELECT round(sin(1),7), sin(radians(30))", "0.841471 0.5"),
        (
            "SELECT round(tan(1),7), round(tan(radians(45)),10)",
            "1.5574077 1.0",
        ),
        (
            "SELECT cos(pi()), degrees(pi()), degrees(pi()/2)",
            "-1.0 180.0 90.0",
        ),
        (
            "SELECT round(atan2(-2,2),7), round(atan2(pi(),0),7)",
            "-0.7853982 1.5707963",
        ),
        (
            "SELECT round(asin(0.2),7), round(atan(2),7), round(atan(-2),7)",
            "0.2013579 1.1071487 -1.1071487",
        ),
        // An argument outside the range of the arc sine and one that is no
        // number both answer nothing.
        (
            "SELECT quote(acos(1.0001)), quote(asin('foo')), quote(asin(-1.5))",
            "NULL NULL NULL",
        ),
        (
            "SELECT quote(atan2('x',1)), quote(atan2(1,'x'))",
            "NULL NULL",
        ),
    ]);
}

/// `func7-pg-500` through `func7-pg-550` of `test/func7.test`.
#[test]
fn what_the_hyperbolic_functions_answer() {
    same_as_the_c_library(&[
        (
            "SELECT round(sinh(1),7), round(cosh(0),7), round(tanh(1),7)",
            "1.1752012 1.0 0.7615942",
        ),
        (
            "SELECT round(asinh(1),7), round(acosh(1),7), round(atanh(0.5),7)",
            "0.8813736 0.0 0.5493061",
        ),
        // No argument under one has an inverse hyperbolic cosine, and none
        // over one in magnitude an inverse hyperbolic tangent.
        (
            "SELECT quote(acosh(0.5)), quote(atanh(1.5)), atanh(1)",
            "NULL NULL Inf",
        ),
    ]);
}

/// `func7-pg-130` through `func7-pg-280` of `test/func7.test`.
#[test]
fn what_the_root_and_the_power_answer() {
    same_as_the_c_library(&[
        (
            "SELECT round(sqrt(2),7), sqrt(4), quote(sqrt(-1))",
            "1.4142136 2.0 NULL",
        ),
        ("SELECT power(9,3), pow(2,0.5)", "729.0 1.4142135623731"),
        // A negative base raised to a power that is no whole number has no
        // answer, and a base of one or a power of nought answers one.
        (
            "SELECT quote(pow(-2,0.5)), pow(-2,3), quote(pow(1,'x')), pow(2,0)",
            "NULL -8.0 NULL 1.0",
        ),
        ("SELECT pow(1,2), mod(9,4), quote(mod(9,0))", "1.0 1.0 NULL"),
        (
            "SELECT round(exp(2),7), round(exp(-2),7), exp(0)",
            "7.3890561 0.1353353 1.0",
        ),
        ("SELECT exp(1000), quote(exp('x'))", "Inf NULL"),
    ]);
}
