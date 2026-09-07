// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::json`, covering the catalog items 6.6.28.

use std::collections::BTreeMap;

use crate::json::{JsonError, MAX_DEPTH, Value, parse};

/// An object of these members.
fn object(members: &[(&str, Value)]) -> Value {
    Value::Object(
        members
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect::<BTreeMap<String, Value>>(),
    )
}

#[test]
fn every_kind_of_value_reads_back() {
    let text = r#"{"a":null,"b":true,"c":false,"d":-12,"e":"x","f":[1,2],"g":{"h":0}}"#;
    let value = parse(text).unwrap();
    assert_eq!(value.get("a"), Some(&Value::Null));
    assert_eq!(value.get("b"), Some(&Value::Bool(true)));
    assert_eq!(value.get("c"), Some(&Value::Bool(false)));
    assert_eq!(value.get("d"), Some(&Value::Int(-12)));
    assert_eq!(value.get("e").and_then(Value::text), Some("x"));
    assert_eq!(
        value.get("f"),
        Some(&Value::Array(vec![Value::Int(1), Value::Int(2)]))
    );
    assert_eq!(value.get("g"), Some(&object(&[("h", Value::Int(0))])));
    assert_eq!(value.get("nothing"), None);
    assert_eq!(Value::Int(1).get("a"), None);
    assert_eq!(Value::Int(1).text(), None);
}

#[test]
fn spaces_and_line_ends_between_the_pieces_are_stepped_over() {
    let value = parse(" {\n  \"a\" : [ 1 , 2 ] ,\t\"b\" : { } \r\n} ").unwrap();
    assert_eq!(
        value.get("a"),
        Some(&Value::Array(vec![Value::Int(1), Value::Int(2)]))
    );
    assert_eq!(value.get("b"), Some(&object(&[])));
}

#[test]
fn the_escapes_of_the_subset_are_read() {
    let value = parse(r#""a\"b\\c\/d\be\ff\ng\rh\tiA""#).unwrap();
    assert_eq!(value.text(), Some("a\"b\\c/d\u{8}e\u{c}f\ng\rh\ti\u{41}"));
}

#[test]
fn an_escape_the_subset_does_not_read_is_an_error() {
    assert_eq!(parse(r#""a\qb""#), Err(JsonError::Escape(3)));
    assert!(matches!(parse(r#""a\u00""#), Err(JsonError::End)));
    assert!(matches!(parse(r#""a\uzzzz""#), Err(JsonError::Escape(3))));
    assert!(matches!(parse(r#""a\ud800""#), Err(JsonError::Escape(3))));
}

#[test]
fn a_number_that_is_no_whole_number_is_an_error() {
    assert_eq!(parse("1.5"), Err(JsonError::Number("1".to_owned())));
    assert_eq!(parse("1e3"), Err(JsonError::Number("1".to_owned())));
    assert!(matches!(parse("-"), Err(JsonError::Number(_))));
    assert!(matches!(
        parse("99999999999999999999"),
        Err(JsonError::Number(_))
    ));
}

#[test]
fn what_begins_no_value_and_what_follows_one_are_errors() {
    assert_eq!(parse("%"), Err(JsonError::Unexpected(0)));
    assert_eq!(parse("truth"), Err(JsonError::Unexpected(0)));
    assert_eq!(parse("1 2"), Err(JsonError::Trailing(2)));
    assert_eq!(parse(""), Err(JsonError::End));
    assert_eq!(parse("{1:2}"), Err(JsonError::Unexpected(1)));
    assert_eq!(parse(r#"{"a" 2}"#), Err(JsonError::Unexpected(5)));
    assert_eq!(parse(r#"{"a":2;}"#), Err(JsonError::Unexpected(6)));
    assert_eq!(parse("[1;2]"), Err(JsonError::Unexpected(2)));
    assert_eq!(parse(r#"{"a":2"#), Err(JsonError::End));
    assert_eq!(parse("[1"), Err(JsonError::End));
    assert_eq!(parse(r#""abc"#), Err(JsonError::End));
    assert_eq!(parse("tru"), Err(JsonError::End));
}

#[test]
fn nesting_deeper_than_the_bound_is_refused_and_does_not_run_away() {
    let deep = format!(
        "{}1{}",
        "[".repeat(MAX_DEPTH + 2),
        "]".repeat(MAX_DEPTH + 2)
    );
    assert_eq!(parse(&deep), Err(JsonError::Depth));
    let shallow = format!("{}1{}", "[".repeat(MAX_DEPTH), "]".repeat(MAX_DEPTH));
    assert!(parse(&shallow).is_ok());
}

#[test]
fn an_empty_object_and_an_empty_array_are_values() {
    assert_eq!(parse("{}"), Ok(object(&[])));
    assert_eq!(parse("[]"), Ok(Value::Array(Vec::new())));
}

#[test]
fn what_the_writer_writes_reads_back_as_the_same_value() {
    let values = [
        Value::Null,
        Value::Bool(true),
        Value::Bool(false),
        Value::Int(0),
        Value::Int(-9_007_199_254_740_993),
        Value::Int(i64::MAX),
        Value::Text(String::new()),
        Value::Text("a\"b\\c\nd\te\r\u{1}".to_owned()),
        Value::Array(vec![Value::Int(1), Value::Text("two".to_owned())]),
        object(&[
            ("execute", Value::Text("screendump".to_owned())),
            (
                "arguments",
                object(&[("filename", Value::Text("/tmp/x.ppm".to_owned()))]),
            ),
        ]),
    ];
    for value in values {
        let written = value.write();
        assert_eq!(parse(&written), Ok(value.clone()), "{written}");
    }
}

#[test]
fn a_command_is_one_line_of_json() {
    let command = Value::object([(
        "execute".to_owned(),
        Value::Text("qmp_capabilities".to_owned()),
    )]);
    assert_eq!(command.write(), r#"{"execute":"qmp_capabilities"}"#);
    assert!(!command.write().contains('\n'));
}

#[test]
fn every_reason_a_text_is_no_value_reads_as_a_sentence() {
    let errors = [
        JsonError::End,
        JsonError::Unexpected(3),
        JsonError::Number("x".to_owned()),
        JsonError::Escape(4),
        JsonError::Depth,
        JsonError::Trailing(5),
    ];
    for error in errors {
        assert!(!format!("{error}").is_empty());
    }
}
