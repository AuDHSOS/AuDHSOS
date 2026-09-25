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
        call(
            function,
            core::slice::from_ref(value),
            &[],
            Collation::Binary,
            crate::header::Encoding::Utf8,
            crate::func::Given {
                random: None,
                counted: crate::func::Counted::default(),
                clock: crate::date::Told::default(),
                sensitive: false,
                limits: crate::db::Limits::new(),
            },
        )
        .expect("a function that always answers")
        .0
        .text()
        .unwrap_or_default()
    };
    (of(Function::Typeof), of(Function::Quote))
}

/// What this engine answers for one expression, or nothing where it
/// refuses it.
fn answer(sql: &str) -> Option<(Vec<u8>, Vec<u8>)> {
    let (arena, root) = expression(sql.as_bytes()).ok()?;
    let value = evaluate(&arena, root, sql.as_bytes(), None).ok()?;
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

/// What this engine answers an expression with, which a parameter that
/// no caller bound answers a null for.
#[test]
fn what_a_parameter_no_caller_bound_answers() {
    let (arena, root) = expression(b"?").unwrap();
    assert_eq!(
        evaluate(&arena, root, b"?", None).unwrap(),
        crate::value::Value::Null
    );
}

/// What this engine refuses an expression with.
fn refusal(sql: &str) -> Error {
    let (arena, root) = expression(sql.as_bytes()).unwrap();
    evaluate(&arena, root, sql.as_bytes(), None).unwrap_err()
}

#[test]
fn what_is_not_written_yet_refuses_rather_than_guessing() {
    assert_eq!(refusal("a"), Error::NoColumn(b"a".to_vec()));
    assert_eq!(refusal("t.a"), Error::NoColumn(b"t.a".to_vec()));
    // `resolveExprStep` asks whether a name written in double quotes
    // was meant to be text, which it is where no table answers it.
    assert_eq!(
        refusal("\"hello\""),
        Error::NoColumn(b"\"hello\" - should this be a string literal in single-quotes?".to_vec())
    );
    assert_eq!(refusal("\"t\".\"a\""), Error::NoColumn(b"t.a".to_vec()));
    assert_eq!(
        refusal("nosuchfunction(1)"),
        Error::NoFunction(b"nosuchfunction".to_vec())
    );
    assert_eq!(
        refusal("'a' REGEXP 'b'"),
        Error::NoFunction(b"REGEXP".to_vec())
    );
    assert_eq!(
        refusal("'a' MATCH 'b'"),
        Error::NoFunction(b"MATCH".to_vec())
    );
    assert_eq!(
        refusal("('a' COLLATE nosuch)='a'"),
        Error::NoCollation(b"nosuch".to_vec())
    );
    assert_eq!(refusal("random()"), Error::NoRandom);
    assert_eq!(refusal("randomblob(4)"), Error::NoRandom);
    assert_eq!(refusal("zeroblob(1000000001)"), Error::TooBig);
    assert_eq!(refusal("zeroblob(9223372036854775807)"), Error::TooBig);
    assert_eq!(refusal("abs(1,2)"), Error::WrongArguments(b"abs".to_vec()));
    assert_eq!(
        refusal("substr('a')"),
        Error::WrongArguments(b"substr".to_vec())
    );
    assert_eq!(refusal("abs(-9223372036854775807-1)"), Error::Overflow);
    assert_eq!(refusal("'a' LIKE 'b' ESCAPE 'xy'"), Error::BadEscape);
    assert_eq!(refusal("unistr('\\xyz')"), Error::BadUnicode);
    assert_eq!(refusal("likelihood(1,0)"), Error::BadProbability);
    assert_eq!(
        refusal("count(*)"),
        Error::MisusedAggregate(b"count".to_vec())
    );
    assert_eq!(
        refusal("count(DISTINCT 1)"),
        Error::MisusedAggregate(b"count".to_vec())
    );
    assert_eq!(refusal("CURRENT_TIME"), Error::Unsupported);
    assert_eq!(refusal("(1,2)"), Error::RowValue);
    assert_eq!(refusal("(SELECT 1)"), Error::Unsupported);
    assert_eq!(refusal("EXISTS (SELECT 1)"), Error::Unsupported);
    assert_eq!(refusal("1 IN (SELECT 1)"), Error::Unsupported);
    assert_eq!(refusal("1 IN t"), Error::Unsupported);
    assert_eq!(
        refusal("-0x8000000000000000"),
        Error::HexTooBig(b"-0x8000000000000000".to_vec())
    );
    assert_eq!(
        refusal("0xFFFFFFFFFFFFFFFFF"),
        Error::HexTooBig(b"0xFFFFFFFFFFFFFFFFF".to_vec())
    );
    // The words the refusal carries name the literal as it was written.
    assert_eq!(
        refusal("0xFFFFFFFFFFFFFFFFF").message(),
        "hex literal too big: 0xFFFFFFFFFFFFFFFFF"
    );
    assert_eq!(
        refusal("'a' LIKE 'b' ESCAPE 'xy'").message(),
        "ESCAPE expression must be a single character"
    );
    assert_eq!(
        refusal("abs(-9223372036854775808)").message(),
        "integer overflow"
    );
}

#[test]
fn a_node_the_arena_does_not_hold_is_refused() {
    let (small, _) = expression(b"1").unwrap();
    let (_, deep) = expression(b"1+2").unwrap();
    assert_eq!(evaluate(&small, deep, b"1", None), Err(Error::Malformed));
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
    assert_eq!(evaluate(&arena, id, b"1", None), Err(Error::TooDeep));
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

/// `ieee754.test`: the `ieee754` family reads a binary64 number apart
/// into a mantissa and an exponent of two and writes it back.
#[test]
fn what_the_ieee754_family_answers() {
    let answered = |sql: &str| {
        let writer = crate::change::Writer::new(1024, 0, crate::header::Encoding::Utf8).unwrap();
        let bytes = writer.written();
        crate::db::Database::open(&bytes)
            .unwrap()
            .query(sql.as_bytes())
            .unwrap()
            .rows
            .remove(0)
            .remove(0)
    };
    let text = |held: &str| Value::Text(held.as_bytes().to_vec());
    assert_eq!(answered("SELECT ieee754(2.0)"), text("ieee754(2,0)"));
    assert_eq!(answered("SELECT ieee754(45.25)"), text("ieee754(181,-2)"));
    assert_eq!(answered("SELECT ieee754(-0.5)"), text("ieee754(-1,-1)"));
    assert_eq!(answered("SELECT ieee754(0.0)"), text("ieee754(0,-1075)"));
    // A nought that carries the sign is the one pair the bits of no
    // mantissa answer.
    assert_eq!(answered("SELECT ieee754(-0.0)"), text("ieee754(-1,-3071)"));
    // A number the bits hold no whole mantissa for, which is every
    // number below the smallest one the bits name whole.
    assert_eq!(answered("SELECT ieee754(5e-324)"), text("ieee754(1,-1074)"));
    // A mantissa past what the bits hold is shifted down, and one the
    // exponent leaves below the smallest number is shifted away.
    assert_eq!(
        answered("SELECT ieee754(9223372036854775807, 0)"),
        Value::Real(9.223_372_036_854_775e18)
    );
    assert_eq!(answered("SELECT ieee754(1, -1074)"), Value::Real(5e-324));
    assert_eq!(answered("SELECT ieee754(1, -1080)"), Value::Real(0.0));
    // A number the bits hold no whole mantissa for, which is every
    // number below the smallest one the bits name whole.
    assert_eq!(answered("SELECT ieee754(5e-324)"), text("ieee754(1,-1074)"));
    // A mantissa past what the bits hold is shifted down, and one the
    // exponent leaves below the smallest number is shifted away.
    assert_eq!(
        answered("SELECT ieee754(9223372036854775807, 0)"),
        Value::Real(9.223_372_036_854_775e18)
    );
    assert_eq!(answered("SELECT ieee754(1, -1074)"), Value::Real(5e-324));
    assert_eq!(answered("SELECT ieee754(1, -1080)"), Value::Real(0.0));
    // A blob of another width than a binary64 number is read as the
    // number the value stands for, which is nought.
    assert_eq!(answered("SELECT ieee754(x'00')"), text("ieee754(0,-1075)"));
    // A mantissa of nought the exponent leaves outside the numbers a
    // nought names answers the largest number and the smallest.
    assert_eq!(
        answered("SELECT ieee754(0, 2000)"),
        Value::Real(f64::INFINITY)
    );
    assert_eq!(answered("SELECT ieee754(0, -2000)"), Value::Real(0.0));
    // A mantissa and an exponent that name no number answer nothing.
    assert_eq!(
        answered("SELECT ieee754(4503599627370495, 973)"),
        Value::Null
    );
    assert_eq!(
        answered("SELECT ieee754(9007199254740992.0)"),
        text("ieee754(4503599627370496,1)")
    );
    // The two-argument form answers the number the mantissa and the
    // exponent name.
    assert_eq!(answered("SELECT ieee754(2, 0)"), Value::Real(2.0));
    assert_eq!(answered("SELECT ieee754(181, -2)"), Value::Real(45.25));
    assert_eq!(answered("SELECT ieee754(0, 0)"), Value::Real(0.0));
    assert_eq!(answered("SELECT ieee754(-181, -2)"), Value::Real(-45.25));
    // An exponent past what a binary64 number holds answers the largest
    // and the smallest it holds.
    assert_eq!(
        answered("SELECT ieee754(1, 20000)"),
        Value::Real(f64::INFINITY)
    );
    assert_eq!(answered("SELECT ieee754(1, -20000)"), Value::Real(0.0));
    // The mantissa and the exponent alone.
    assert_eq!(answered("SELECT ieee754_mantissa(45.25)"), Value::Int(181));
    assert_eq!(answered("SELECT ieee754_exponent(45.25)"), Value::Int(-2));
    // The eight bytes of a number, and the number they are.
    assert_eq!(
        answered("SELECT ieee754_to_blob(1.0)"),
        Value::Blob(alloc::vec![0x3f, 0xf0, 0, 0, 0, 0, 0, 0])
    );
    assert_eq!(
        answered("SELECT ieee754_from_blob(x'3ff0000000000000')"),
        Value::Real(1.0)
    );
    assert_eq!(
        answered("SELECT ieee754(x'3ff0000000000000')"),
        text("ieee754(1,0)")
    );
    // A value of another kind and a blob of another width answer
    // nothing.
    assert_eq!(answered("SELECT ieee754_to_blob('x')"), Value::Null);
    assert_eq!(answered("SELECT ieee754_from_blob(x'00')"), Value::Null);
    // The options the build holds, which a name is read against without
    // the `SQLITE_` in front of it and without regard to case: a name
    // matches an option that carries more than the name only where the
    // byte after it opens no name.
    for (sql, want) in [
        ("SELECT sqlite_compileoption_used('THREADSAFE')", 1),
        ("SELECT sqlite_compileoption_used('SQLITE_THREADSAFE')", 1),
        ("SELECT sqlite_compileoption_used('threadsafe=0')", 1),
        ("SELECT sqlite_compileoption_used('THREADSAFE=')", 0),
        ("SELECT sqlite_compileoption_used('THREADSAFE=1')", 0),
        ("SELECT sqlite_compileoption_used('SQLITE_OMIT_TRIGGER')", 0),
        ("SELECT sqlite_compileoption_used('')", 0),
        ("SELECT sqlite_compileoption_used(0)", 0),
        (
            "SELECT sqlite_compileoption_used(sqlite_compileoption_get(0))",
            1,
        ),
    ] {
        assert_eq!(answered(sql), Value::Int(want));
    }
    assert_eq!(
        answered("SELECT sqlite_compileoption_get(0)"),
        text("ENABLE_URI_00_ERROR")
    );
    assert_eq!(
        answered("SELECT sqlite_compileoption_get(1)"),
        text("THREADSAFE=0")
    );
    // A place past the list, a place below nought and a name that is
    // null each answer nothing.
    assert_eq!(answered("SELECT sqlite_compileoption_get(2)"), Value::Null);
    assert_eq!(answered("SELECT sqlite_compileoption_get(-1)"), Value::Null);
    assert_eq!(
        answered("SELECT sqlite_compileoption_used(NULL)"),
        Value::Null
    );
}

/// The version of the format the crate writes, which a `*` for the
/// arguments leaves the call with none of, and the name of the crate in
/// place of a check-in of the C library.
#[test]
fn what_the_version_of_the_library_answers() {
    let answered = |sql: &str| {
        let writer = crate::change::Writer::new(1024, 0, crate::header::Encoding::Utf8).unwrap();
        let bytes = writer.written();
        crate::db::Database::open(&bytes)
            .unwrap()
            .query(sql.as_bytes())
            .unwrap()
            .rows
            .remove(0)
            .remove(0)
    };
    let text = |held: &str| Value::Text(held.as_bytes().to_vec());
    assert_eq!(answered("SELECT sqlite_version()"), text("3.53.4"));
    assert_eq!(answered("SELECT sqlite_version(*)"), text("3.53.4"));
    assert_eq!(
        answered("SELECT sqlite_source_id()"),
        text("db-sqlite 3.53.4")
    );
}
