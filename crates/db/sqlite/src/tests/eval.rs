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
use crate::func::{Function, call};
use crate::parse::{MAX_DEPTH, expression};
use crate::value::{Collation, Value};

/// The expressions.
fn corpus() -> Vec<&'static str> {
    include_str!("fixtures/eval.corpus").lines().collect()
}

/// What SQLite answered: the type and the quoted value, or `!` and the
/// message it refused with. It is read as bytes, because an expression
/// may answer text that is not UTF-8 and `quote` writes it out as it is.
fn golden() -> Vec<(&'static [u8], &'static [u8])> {
    let bytes: &'static [u8] = include_bytes!("fixtures/eval.golden");
    let mut lines: Vec<&[u8]> = bytes.split(|byte| *byte == b'\n').collect();
    lines.pop();
    lines
        .into_iter()
        .map(|line| {
            let mut fields = line.split(|byte| *byte == b'\t');
            let mut next = || fields.next().unwrap_or_default();
            (next(), next())
        })
        .collect()
}

/// The name `typeof` answers with, and the text `quote` answers with,
/// both from the port's own functions so that they are compared against
/// the C library as well.
fn shown(value: &Value) -> (Vec<u8>, Vec<u8>) {
    let of = |function| {
        call(function, core::slice::from_ref(value), Collation::Binary)
            .expect("a function that always answers")
            .text()
            .unwrap_or_default()
    };
    (of(Function::Typeof), of(Function::Quote))
}

/// What this engine answers for one expression, or nothing where it
/// refuses it.
fn answer(sql: &str) -> Option<(Vec<u8>, Vec<u8>)> {
    let (arena, root) = expression(sql.as_bytes()).ok()?;
    let value = evaluate(&arena, root, sql.as_bytes()).ok()?;
    Some(shown(&value))
}

#[test]
fn every_expression_answers_what_the_c_library_answers() {
    let cases = corpus();
    let answers = golden();
    assert_eq!(cases.len(), answers.len());
    for (sql, (kind, value)) in cases.iter().zip(answers) {
        match answer(sql) {
            Some((mine, written)) => {
                assert_ne!(kind, b"!", "{sql} is refused by SQLite and answered here");
                assert_eq!(mine, kind, "the type of {sql}");
                assert_eq!(
                    String::from_utf8_lossy(&written),
                    String::from_utf8_lossy(value),
                    "the value of {sql}"
                );
                assert_eq!(written, value, "the value of {sql}");
            }
            None => assert_eq!(kind, b"!", "{sql} is answered by SQLite and refused here"),
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
    assert_eq!(refusal("nosuchfunction(1)"), Error::NoFunction);
    assert_eq!(refusal("'a' REGEXP 'b'"), Error::NoFunction);
    assert_eq!(refusal("'a' MATCH 'b'"), Error::NoFunction);
    assert_eq!(refusal("printf('%d',1)"), Error::NoFunction);
    assert_eq!(refusal("zeroblob(2)"), Error::NoFunction);
    assert_eq!(refusal("random()"), Error::NoFunction);
    assert_eq!(refusal("abs(1,2)"), Error::WrongArguments);
    assert_eq!(refusal("substr('a')"), Error::WrongArguments);
    assert_eq!(refusal("abs(-9223372036854775807-1)"), Error::Overflow);
    assert_eq!(refusal("'a' LIKE 'b' ESCAPE 'xy'"), Error::BadEscape);
    assert_eq!(refusal("likelihood(1,0)"), Error::BadProbability);
    assert_eq!(refusal("count(*)"), Error::Unsupported);
    assert_eq!(refusal("count(DISTINCT 1)"), Error::Unsupported);
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

#[test]
fn a_pattern_longer_than_the_engine_takes_is_refused() {
    use crate::func::MAX_PATTERN;
    // The limit is what bounds how deep the comparison recurses, which is
    // why it is a refusal rather than a slow answer.
    let long = "a".repeat(MAX_PATTERN + 1);
    assert_eq!(refusal(&format!("'a' LIKE '{long}'")), Error::PatternTooBig);
    assert_eq!(refusal(&format!("'a' GLOB '{long}'")), Error::PatternTooBig);
    // One byte under it is answered.
    let long = "a".repeat(MAX_PATTERN);
    assert!(answer(&format!("'a' LIKE '{long}'")).is_some());
}

#[test]
fn a_pattern_that_branches_at_every_step_still_answers() {
    // Each wildcard is one more frame of the walk. A thousand of them is
    // far past anything a statement would carry and well inside what the
    // limit above allows.
    let pattern = "%a".repeat(1000);
    let subject = "a".repeat(1000);
    let sql = format!("'{subject}' LIKE '{pattern}'");
    assert_eq!(
        answer(&sql).map(|(kind, _)| kind),
        Some(b"integer".to_vec())
    );
}
