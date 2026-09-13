// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::eval` and `crate::value`, against the engine they
//! are a port of.
//!
//! `fixtures/eval.corpus` is six thousand seven hundred expressions, one
//! per line: every binary operator between every pair of sixteen
//! operands, every one-operand form and every cast over thirty-eight
//! values and nineteen type names, and the shapes no cross product
//! writes. `fixtures/eval.golden` is what SQLite answered for each —
//! `typeof` and `quote`, or the message it refused the expression with —
//! written by `tools/sqlite-oracle.c`. The test is the comparison.

#![allow(clippy::arithmetic_side_effects)]

use crate::eval::{Error, evaluate};
use crate::fp::{DIGITS, text};
use crate::parse::{MAX_DEPTH, expression};
use crate::value::Value;

/// The expressions.
fn corpus() -> Vec<&'static str> {
    include_str!("fixtures/eval.corpus").lines().collect()
}

/// What SQLite answered: the type and the quoted value, or `!` and the
/// message it refused with.
fn golden() -> Vec<(&'static str, &'static str)> {
    include_str!("fixtures/eval.golden")
        .lines()
        .map(|line| {
            let mut fields = line.split('\t');
            let mut next = || fields.next().unwrap_or_default();
            (next(), next())
        })
        .collect()
}

/// The name `typeof` answers with.
fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Int(_) => "integer",
        Value::Real(_) => "real",
        Value::Text(_) => "text",
        Value::Blob(_) => "blob",
    }
}

/// The text `quote` answers with, which is `%!0.17g` for a real: the
/// zero flag is what shows an infinity as `9.0e+999`.
fn quoted(value: &Value) -> String {
    match value {
        Value::Null => "NULL".to_owned(),
        Value::Int(number) => format!("{number}"),
        Value::Real(number) => {
            if number.is_nan() {
                "null".to_owned()
            } else if number.is_infinite() {
                let sign = if *number < 0.0 { "-" } else { "" };
                format!("{sign}9.0e+999")
            } else {
                String::from_utf8(text(*number, DIGITS)).unwrap()
            }
        }
        Value::Text(bytes) => {
            // `quote` builds its answer from a C string, so text stops at
            // the first NUL whatever follows it.
            let bytes = bytes.split(|byte| *byte == 0).next().unwrap_or_default();
            let mut out = String::from("'");
            for byte in bytes {
                if *byte == b'\'' {
                    out.push('\'');
                }
                out.push(char::from(*byte));
            }
            out.push('\'');
            out
        }
        Value::Blob(bytes) => {
            use std::fmt::Write;
            let mut out = String::from("X'");
            for byte in bytes {
                let _ = write!(out, "{byte:02X}");
            }
            out.push('\'');
            out
        }
    }
}

/// What this engine answers for one expression, or nothing where it
/// refuses it.
fn answer(sql: &str) -> Option<(&'static str, String)> {
    let (arena, root) = expression(sql.as_bytes()).ok()?;
    let value = evaluate(&arena, root, sql.as_bytes()).ok()?;
    Some((type_name(&value), quoted(&value)))
}

#[test]
fn every_expression_answers_what_the_c_library_answers() {
    let cases = corpus();
    let answers = golden();
    assert_eq!(cases.len(), answers.len());
    for (sql, (kind, value)) in cases.iter().zip(answers) {
        match answer(sql) {
            Some((mine, written)) => {
                assert_ne!(kind, "!", "{sql} is refused by SQLite and answered here");
                assert_eq!(mine, kind, "the type of {sql}");
                assert_eq!(written, value, "the value of {sql}");
            }
            None => assert_eq!(kind, "!", "{sql} is answered by SQLite and refused here"),
        }
    }
}

/// What this engine refuses an expression with.
fn refusal(sql: &str) -> Error {
    let (arena, root) = expression(sql.as_bytes()).unwrap();
    evaluate(&arena, root, sql.as_bytes()).unwrap_err()
}

#[test]
fn what_is_not_written_yet_refuses_rather_than_guessing() {
    assert_eq!(refusal("a"), Error::NoColumn);
    assert_eq!(refusal("t.a"), Error::NoColumn);
    assert_eq!(refusal("abs(1)"), Error::NoFunction);
    assert_eq!(refusal("'a' LIKE 'b'"), Error::NoFunction);
    assert_eq!(refusal("'a' GLOB 'b'"), Error::NoFunction);
    assert_eq!(refusal("'{}' -> 'a'"), Error::NoFunction);
    assert_eq!(refusal("'{}' ->> 'a'"), Error::NoFunction);
    assert_eq!(refusal("CURRENT_TIME"), Error::Unsupported);
    assert_eq!(refusal("?"), Error::Unsupported);
    assert_eq!(refusal("(1,2)"), Error::Unsupported);
    assert_eq!(refusal("(SELECT 1)"), Error::Unsupported);
    assert_eq!(refusal("EXISTS (SELECT 1)"), Error::Unsupported);
    assert_eq!(refusal("1 IN (SELECT 1)"), Error::Unsupported);
    assert_eq!(refusal("1 IN t"), Error::Unsupported);
    assert_eq!(refusal("-0x8000000000000000"), Error::HexTooBig);
    assert_eq!(refusal("0xFFFFFFFFFFFFFFFFF"), Error::HexTooBig);
}

#[test]
fn a_node_the_arena_does_not_hold_is_refused() {
    let (small, _) = expression(b"1").unwrap();
    let (_, deep) = expression(b"1+2").unwrap();
    assert_eq!(evaluate(&small, deep, b"1"), Err(Error::Malformed));
}

#[test]
fn a_tree_deeper_than_the_walk_is_refused() {
    use crate::ast::{Arena, Literal, Node, Span, UnaryOp};
    // The parser will not build one this deep, so it is built by hand:
    // a tree handed in from outside must be refused, not walked until
    // the stack runs out.
    let mut arena = Arena::default();
    let mut id = arena.push(Node::Literal(Literal::Integer(Span { start: 0, len: 1 })));
    for _ in 0..MAX_DEPTH + 2 {
        id = arena.push(Node::Unary {
            op: UnaryOp::Not,
            operand: id,
        });
    }
    assert_eq!(evaluate(&arena, id, b"1"), Err(Error::TooDeep));
}
