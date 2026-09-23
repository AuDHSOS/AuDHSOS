// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! JSON read out of text and written back out, against what the C
//! library's `json()` answers for the same text.

#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]

use alloc::vec::Vec;

/// What `json(text)` answers, or nothing where the text is no JSON.
fn minified(text: &[u8]) -> Option<Vec<u8>> {
    let (blob, _) = crate::json::read(text).ok()?;
    let mut out = Vec::new();
    crate::json::write(&blob, 0, &mut out)?;
    Some(out)
}

/// Every text answers what the C library answers for it.
fn same_as_the_c_library(cases: &[(&str, Option<&str>)]) {
    for (text, want) in cases {
        let mine = minified(text.as_bytes());
        let want = want.map(|held| held.as_bytes().to_vec());
        assert_eq!(mine, want, "{text}");
    }
}

#[test]
fn every_value_json_holds_is_read_and_written_again() {
    same_as_the_c_library(&[
        ("null", Some("null")),
        ("true", Some("true")),
        ("false", Some("false")),
        ("0", Some("0")),
        ("-1", Some("-1")),
        ("1.5", Some("1.5")),
        ("1e3", Some("1e3")),
        ("1E+3", Some("1E+3")),
        (r#""x""#, Some(r#""x""#)),
        (r#""a\nb""#, Some(r#""a\nb""#)),
        ("[]", Some("[]")),
        ("{}", Some("{}")),
        ("[1,2,3]", Some("[1,2,3]")),
        (
            r#"{"a":1,"b":[2,{"c":null}]}"#,
            Some(r#"{"a":1,"b":[2,{"c":null}]}"#),
        ),
        ("  [ 1 , 2 ]  ", Some("[1,2]")),
        ("", None),
        ("[", None),
        ("[1,", None),
        (r#"{"a"}"#, None),
        ("nul", None),
        ("01", None),
        ("[1] x", None),
    ]);
}

#[test]
fn every_value_json5_holds_is_read_and_written_as_json() {
    same_as_the_c_library(&[
        ("{a:1}", Some(r#"{"a":1}"#)),
        ("{$a_1:1}", Some(r#"{"$a_1":1}"#)),
        ("[1,2,]", Some("[1,2]")),
        ("{a:1,}", Some(r#"{"a":1}"#)),
        ("'x'", Some(r#""x""#)),
        (r#"'a"b'"#, Some(r#""a\"b""#)),
        ("+1", Some("1")),
        (".5", Some("0.5")),
        ("5.", Some("5.0")),
        ("-.5", Some("-0.5")),
        ("0x1f", Some("31")),
        ("-0xff", Some("-255")),
        ("Infinity", Some("9e999")),
        ("-Infinity", Some("-9e999")),
        ("NaN", Some("null")),
        ("[1/*two*/,2]", Some("[1,2]")),
        ("[1,//two\n2]", Some("[1,2]")),
        (r"'a\'b'", Some(r#""a'b""#)),
        (r"'a\x41b'", Some(r#""a\u0041b""#)),
        (r"'a\0b'", Some(r#""a\u0000b""#)),
    ]);
}

/// The text a connection answers for `SELECT quote(<expr>)`.
fn quoted(sql: &str) -> alloc::string::String {
    use crate::change::Writer;
    use crate::db::Database;
    use crate::header::Encoding;
    let writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    let mut text = alloc::string::String::from("SELECT quote(");
    text.push_str(sql);
    text.push(')');
    match database.query(text.as_bytes()) {
        Ok(answer) => {
            let value = answer.rows.first().and_then(|row| row.first()).cloned();
            alloc::string::String::from_utf8_lossy(
                &value
                    .unwrap_or(crate::value::Value::Null)
                    .text()
                    .unwrap_or_default(),
            )
            .into_owned()
        }
        Err(error) => error.message(),
    }
}

/// Every call answers what the C library answers for it.
fn calls_answer(cases: &[(&str, &str)]) {
    for (sql, want) in cases {
        assert_eq!(&quoted(sql), want, "{sql}");
    }
}

#[test]
fn every_json_function_answers_what_the_c_library_answers() {
    calls_answer(&[
        (r#"json('{"a":1}')"#, r#"'{"a":1}'"#),
        (r"json_array(1,2,'three',null)", r#"'[1,2,"three",null]'"#),
        (r"json_array(json('[1]'))", r"'[[1]]'"),
        (
            r"json_object('a',1,'b',json('[2]'))",
            r#"'{"a":1,"b":[2]}'"#,
        ),
        (r#"json_extract('{"a":{"b":7}}','$.a.b')"#, r"7"),
        (r#"json_extract('{"a":{"b":7}}','$.a')"#, r#"'{"b":7}'"#),
        (r"json_extract('[1,2,3]','$[1]','$[2]')", r"'[2,3]'"),
        (r"json_extract('[1,2,3]','$[#-1]')", r"3"),
        (r#"json_type('{"a":[1]}')"#, r"'object'"),
        (r#"json_type('{"a":[1]}','$.a')"#, r"'array'"),
        (r#"json_type('{"a":[1]}','$.b')"#, r"NULL"),
        (r"json_array_length('[1,2,3]')", r"3"),
        (r#"json_array_length('{"a":[1,2]}','$.a')"#, r"2"),
        (r#"json_valid('{"a":1}')"#, r"1"),
        (r"json_valid('{a:1}')", r"0"),
        (r"json_valid('{a:1}',2)", r"1"),
        (r"json_quote(3.5)", r"'3.5'"),
        (r"json_quote('x')", r#"'"x"'"#),
        (r"json_quote(json('[1]'))", r"'[1]'"),
    ]);
}

#[test]
fn every_json_function_that_writes_answers_what_the_c_library_answers() {
    calls_answer(&[
        (r#"json_set('{"a":1}','$.b',2)"#, r#"'{"a":1,"b":2}'"#),
        (
            r#"json_insert('{"a":1}','$.a',9,'$.c',3)"#,
            r#"'{"a":1,"c":3}'"#,
        ),
        (r#"json_replace('{"a":1}','$.a',9,'$.c',3)"#, r#"'{"a":9}'"#),
        (r#"json_remove('{"a":1,"b":2}','$.a')"#, r#"'{"b":2}'"#),
        (r"json_remove('[1,2,3]','$[1]')", r"'[1,3]'"),
        (r"json_set('{}','$.a.b.c',1)", r#"'{"a":{"b":{"c":1}}}'"#),
        (r"json_set('[]','$[0]',7)", r"'[7]'"),
        (
            r#"json_patch('{"a":1,"b":2}','{"b":null,"c":3}')"#,
            r#"'{"a":1,"c":3}'"#,
        ),
        (
            r#"json_patch('{"a":{"x":1}}','{"a":{"y":2}}')"#,
            r#"'{"a":{"x":1,"y":2}}'"#,
        ),
        (r"json_error_position('[1,2')", r"5"),
        (r"json_error_position('[1,2]')", r"0"),
    ]);
}

#[test]
fn every_arrow_answers_what_the_c_library_answers() {
    calls_answer(&[
        (r#"'{"a":[1,2]}' -> '$.a'"#, r"'[1,2]'"),
        (r#"'{"a":[1,2]}' ->> '$.a[1]'"#, r"2"),
        (r#"'{"a":7}' -> 'a'"#, r"'7'"),
        (r"'[1,2,3]' ->> 1", r"2"),
        (r"'[1,2,3]' ->> -1", r"3"),
        (r#"'{"a":"x"}' ->> 'a'"#, r"'x'"),
        (r#"'{"a":"x"}' -> 'a'"#, "'\"x\"'"),
    ]);
}

#[test]
fn every_jsonb_function_answers_what_the_c_library_answers() {
    calls_answer(&[
        (r"jsonb('[1,2]')", r"X'4B13311332'"),
        (r"json(jsonb('[1,2]'))", r"'[1,2]'"),
        (r"json(jsonb_array(1,'x'))", "'[1,\"x\"]'"),
        (
            r#"json(jsonb_set('{"a":1}','$.b',2))"#,
            r#"'{"a":1,"b":2}'"#,
        ),
        (r#"jsonb_extract('{"a":[1]}','$.a')"#, r"X'2B1331'"),
        (r"json_valid(jsonb('[1]'),8)", r"1"),
    ]);
}

#[test]
fn every_array_insert_answers_what_the_c_library_answers() {
    calls_answer(&[
        (
            r"json_array_insert('[1,2,3]','$[0]',999,'$[0]',888)",
            r"'[888,999,1,2,3]'",
        ),
        (r"json_array_insert('[1,2,3]','$[#]',888)", r"'[1,2,3,888]'"),
        (r"json_array_insert('[1,2,3]','$[1]',888)", r"'[1,888,2,3]'"),
        (
            r#"json_array_insert('{"a":1}','$.a',2)"#,
            r"not an array element: '$.a'",
        ),
    ]);
}

#[test]
fn every_patch_answers_what_the_c_library_answers() {
    calls_answer(&[
        (
            r#"json_patch('{"a":"b","c":{"d":"e","f":"g"}}','{"a":"z","c":{"f":null}}')"#,
            r#"'{"a":"z","c":{"d":"e"}}'"#,
        ),
        (
            r#"json_patch('{"a":{"b":"c"}}','{"a":{"b":"d"}}')"#,
            r#"'{"a":{"b":"d"}}'"#,
        ),
        (
            r#"json_patch('{"x":{"one":1,"two":2}}','{"x":"three"}')"#,
            r#"'{"x":"three"}'"#,
        ),
        (
            r#"json_patch('{"x":{"one":1}}','{"x":{"two":2},"x":"three"}')"#,
            r#"'{"x":"three"}'"#,
        ),
        (
            r#"json_patch('[1,2]','{"a":"b","c":null}')"#,
            r#"'{"a":"b"}'"#,
        ),
        (
            r#"json_patch('{}','{"a":{"bb":{"ccc":null}}}')"#,
            r#"'{"a":{"bb":{}}}'"#,
        ),
    ]);
}

#[test]
fn a_json_function_with_nothing_to_read_answers_nothing() {
    calls_answer(&[
        ("json(NULL)", "NULL"),
        ("json_type(NULL)", "NULL"),
        ("json_array(NULL)", "'[null]'"),
        (r"json_patch('{}',NULL)", "NULL"),
        (r"json_patch(NULL,'{}')", "NULL"),
        (r"json_set(NULL,'$.a',1)", "NULL"),
        ("json_quote(NULL)", "'null'"),
        (r"json_extract(NULL,'$')", "NULL"),
        ("json_valid(NULL)", "NULL"),
        ("json_error_position(NULL)", "NULL"),
    ]);
}
#[test]
fn every_json_call_answers_the_same_1() {
    calls_answer(&[
        (
            r#"json('["aaaaaaaaaaaaaaaa"]')"#,
            r#"'["aaaaaaaaaaaaaaaa"]'"#,
        ),
        (r"json(char(9,10,13,32)||'[1]')", r"'[1]'"),
        (r"json(char(11,12)||'[1]')", r"'[1]'"),
        (r"json(char(160)||'[1]')", r"'[1]'"),
        (r"json(char(5760)||'[1]')", r"'[1]'"),
        (r"json(char(8232)||'[1]')", r"'[1]'"),
        (r"json(char(8233)||'[1]')", r"'[1]'"),
        (r"json(char(8287)||'[1]')", r"'[1]'"),
        (r"json(char(12288)||'[1]')", r"'[1]'"),
        (r"json(char(65279)||'[1]')", r"'[1]'"),
        (r"json(char(8192)||'[1]')", r"'[1]'"),
        (r"json(char(8202)||'[1]')", r"'[1]'"),
        (r"json(char(8239)||'[1]')", r"'[1]'"),
        (r"json(char(5761)||'[1]')", r"malformed JSON"),
        (r"json(char(8288)||'[1]')", r"malformed JSON"),
        (r"json(char(12289)||'[1]')", r"malformed JSON"),
        (r"json(char(65278)||'[1]')", r"malformed JSON"),
        (r"json(char(194)||'[1]')", r"malformed JSON"),
        (r"json(char(8203)||'[1]')", r"malformed JSON"),
        (r"json('[1'||char(160)||',2]')", r"'[1,2]'"),
        (r"json('{a:1'||char(160)||',b:2}')", r#"'{"a":1,"b":2}'"#),
        (r"json('{a'||char(160)||':1}')", r#"'{"a":1}'"#),
        (r"json('/*a*/[1]')", r"'[1]'"),
        (r"json('[1]/*a*/')", r"'[1]'"),
        (r"json('//a'||char(10)||'[1]')", r"'[1]'"),
        (r"json('[1]//a')", r"'[1]'"),
        (r"json('[1]//a'||char(226,128,168)||'')", r"'[1]'"),
        (r"json('/*unterminated [1]')", r"malformed JSON"),
        (r"json('/[1]')", r"malformed JSON"),
        (r"json('/*')", r"malformed JSON"),
        (r#"json('"a\nb"')"#, r#"'"a\nb"'"#),
        (r#"json('"a\u0041b"')"#, r#"'"a\u0041b"'"#),
        (r#"json('"a\ud83d\ude00b"')"#, r#"'"a\ud83d\ude00b"'"#),
        (r#"json('"a\qb"')"#, r"malformed JSON"),
        (r#"json('"a\\b"')"#, r#"'"a\\b"'"#),
        (r#"json('"a\/b"')"#, r#"'"a\/b"'"#),
        (r#"json('"a\u00e9b"')"#, r#"'"a\u00e9b"'"#),
        (r#"json('"a\ud83db"')"#, r#"'"a\ud83db"'"#),
        (r#"json('"a\u00"')"#, r"malformed JSON"),
        (r#"json('"a\x4"')"#, r"malformed JSON"),
    ]);
}

#[test]
fn every_json_call_answers_the_same_2() {
    calls_answer(&[
        (r"json('''a'||char(1)||'b''')", r#"'"a\u0001b"'"#),
        (r#"json('''a"b''')"#, r#"'"a\"b"'"#),
        (r"json('''a\vb''')", r#"'"a\u000bb"'"#),
        (r"json('''a\'||char(10)||'b''')", r#"'"ab"'"#),
        (r"json('''a\'||char(13)||char(10)||'b''')", r#"'"ab"'"#),
        (r"json('''a\'||char(13)||'b''')", r#"'"ab"'"#),
        (r"json('''a\'||char(226,128,168)||'b''')", r"malformed JSON"),
        (r#"json('"abc')"#, r"malformed JSON"),
        (r"json('0')", r"'0'"),
        (r"json('-0')", r"'-0'"),
        (r"json('0.5')", r"'0.5'"),
        (r"json('-0.5')", r"'-0.5'"),
        (r"json('1e5')", r"'1e5'"),
        (r"json('1E5')", r"'1E5'"),
        (r"json('1e+5')", r"'1e+5'"),
        (r"json('1e-5')", r"'1e-5'"),
        (r"json('0x0')", r"'0'"),
        (r"json('0X1F')", r"'31'"),
        (r"json('-0x1f')", r"'-31'"),
        (r"json('+0x1f')", r"'31'"),
        (r"json('0xffffffffffffffff')", r"'18446744073709551615'"),
        (r"json('0x10000000000000000')", r"'9.0e999'"),
        (r"json('9223372036854775807')", r"'9223372036854775807'"),
        (r"json('-9223372036854775808')", r"'-9223372036854775808'"),
        (r"json('99999999999999999999')", r"'99999999999999999999'"),
        (r"json('1.')", r"'1.0'"),
        (r"json('.1')", r"'0.1'"),
        (r"json('-1.')", r"'-1.0'"),
        (r"json('+1.5')", r"'1.5'"),
        (r"json('1.5e3')", r"'1.5e3'"),
        (r"json('1.e3')", r"'1.0e3'"),
        (r"json('1e')", r"malformed JSON"),
        (r"json('1e+')", r"malformed JSON"),
        (r"json('1ee1')", r"malformed JSON"),
        (r"json('1.2.3')", r"malformed JSON"),
        (r"json('01')", r"malformed JSON"),
        (r"json('-01')", r"malformed JSON"),
        (r"json('inf')", r"'9e999'"),
        (r"json('INF')", r"'9e999'"),
        (r"json('-inf')", r"'-9e999'"),
    ]);
}

#[test]
fn every_json_call_answers_the_same_3() {
    calls_answer(&[
        (r"json('+Infinity')", r"'9e999'"),
        (r"json('nan')", r"'null'"),
        (r"json('QNaN')", r"'null'"),
        (r"json('SNaN')", r"'null'"),
        (r"json('-x')", r"malformed JSON"),
        (r"json('+')", r"malformed JSON"),
        (r"json('.')", r"malformed JSON"),
        (r"json('-')", r"malformed JSON"),
        (r"json('true')", r"'true'"),
        (r"json('false')", r"'false'"),
        (r"json('null')", r"'null'"),
        (r"json('truex')", r"malformed JSON"),
        (r"json('nullx')", r"malformed JSON"),
        (r"json('xyz')", r"malformed JSON"),
        (r"json('[]')", r"'[]'"),
        (r"json('[ ]')", r"'[]'"),
        (r"json('[1,]')", r"'[1]'"),
        (r"json('[,1]')", r"malformed JSON"),
        (r"json('[1 2]')", r"malformed JSON"),
        (r"json('[1/*c*/,2]')", r"'[1,2]'"),
        (r"json('[1,2/*c*/]')", r"'[1,2]'"),
        (r"json('[1'||char(10)||',2]')", r"'[1,2]'"),
        (r"json('[1,2'||char(10)||']')", r"'[1,2]'"),
        (r"json('{}')", r"'{}'"),
        (r"json('{ }')", r"'{}'"),
        (r"json('{a:1}')", r#"'{"a":1}'"#),
        (r"json('{a:1,}')", r#"'{"a":1}'"#),
        (r#"json('{"a":1}')"#, r#"'{"a":1}'"#),
        (r"json('{a:1,b:2}')", r#"'{"a":1,"b":2}'"#),
        (r"json('{a :1}')", r#"'{"a":1}'"#),
        (r"json('{a/*c*/:1}')", r#"'{"a":1}'"#),
        (r"json('{a: 1}')", r#"'{"a":1}'"#),
        (r"json('{a:1 ,b:2}')", r#"'{"a":1,"b":2}'"#),
        (r"json('{a:1/*c*/,b:2}')", r#"'{"a":1,"b":2}'"#),
        (r"json('{1:2}')", r"malformed JSON"),
        (r"json('{a}')", r"malformed JSON"),
        (r"json('{a:}')", r"malformed JSON"),
        (r"json('{a:1,}')", r#"'{"a":1}'"#),
        (r"json('{\u0041:1}')", r#"'{"\u0041":1}'"#),
        (r"json('{$a:1,_b:2,c1:3}')", r#"'{"$a":1,"_b":2,"c1":3}'"#),
    ]);
}

#[test]
fn every_json_call_answers_the_same_4() {
    calls_answer(&[
        (r"json('{a\u0041b:1}')", r#"'{"a\u0041b":1}'"#),
        (r"json('[[[[1]]]]')", r"'[[[[1]]]]'"),
        (r"json('{a:{b:{c:1}}}')", r#"'{"a":{"b":{"c":1}}}'"#),
        (r"json(printf('%.2000c','[')||'1')", r"malformed JSON"),
        (
            r#"json_pretty('{"a":[1,{"b":2}],"c":{}}')"#,
            r#"'{
    "a": [
        1,
        {
            "b": 2
        }
    ],
    "c": {}
}'"#,
        ),
        (r"json_pretty('[]')", r"'[]'"),
        (r"json_pretty('{}')", r"'{}'"),
        (r"json_pretty('1')", r"'1'"),
        (
            r#"json_pretty('{"a":1}','  ')"#,
            r#"'{
  "a": 1
}'"#,
        ),
        (
            r#"json_pretty('{"a":1}',NULL)"#,
            r#"'{
    "a": 1
}'"#,
        ),
        (r"json_type('1')", r"'integer'"),
        (r"json_type('1.5')", r"'real'"),
        (r#"json_type('"x"')"#, r"'text'"),
        (r"json_type('true')", r"'true'"),
        (r"json_type('false')", r"'false'"),
        (r"json_type('null')", r"'null'"),
        (r"json_type('0x1f')", r"'integer'"),
        (r"json_type('1.')", r"'real'"),
        (r"json_type('''x''')", r"'text'"),
        (r"json_array_length('{}')", r"0"),
        (r"json_array_length('1')", r"0"),
        (r"json_array_length('[1,[2,3]]','$[1]')", r"2"),
        (r"json_array_length('[1]','$[5]')", r"NULL"),
        (r#"json_extract('{"a":1}','$')"#, r#"'{"a":1}'"#),
        (r"json_extract('[1,2,3]','$[0]')", r"1"),
        (r"json_extract('[1,2,3]','$[#]')", r"NULL"),
        (r"json_extract('[1,2,3]','$[#-4]')", r"NULL"),
        (r"json_extract('[1,2,3]','$[99]')", r"NULL"),
        (r#"json_extract('{"a":1}','$.b')"#, r"NULL"),
        (r#"json_extract('{"a":1}','$."a"')"#, r"1"),
        (r#"json_extract('{"a b":1}','$."a b"')"#, r"1"),
        (r#"json_extract('{"a\"b":1}','$."a\"b"')"#, r"1"),
        (r#"json_extract('{"a":1}','a')"#, r"bad JSON path: 'a'"),
        (r#"json_extract('{"a":1}','$.')"#, r"bad JSON path: '$.'"),
        (r#"json_extract('{"a":1}','$[0]')"#, r"NULL"),
        (r"json_extract('[1]','$.a')", r"NULL"),
        (
            r#"json_extract('{"a":1}','$."a')"#,
            r#"bad JSON path: '$."a'"#,
        ),
        (r"json_extract('[1]','$[a]')", r"bad JSON path: '$[a]'"),
        (r"json_extract('[1]','$[-1]')", r"bad JSON path: '$[-1]'"),
        (r#"json_extract('{"a":{"b":[1,2]}}','$.a.b[1]')"#, r"2"),
    ]);
}

#[test]
fn every_json_call_answers_the_same_5() {
    calls_answer(&[
        (
            r"json_extract('[1,2]','$[0]','$[1]','$[9]')",
            r"'[1,2,null]'",
        ),
        (r#"json_extract('{"a":null}','$.a')"#, r"NULL"),
        (r#"json_extract('{"a":true}','$.a')"#, r"1"),
        (r#"json_extract('{"a":1.5}','$.a')"#, r"1.5"),
        (r#"json_extract('{"a":"x"}','$.a')"#, r"'x'"),
        (r#"json_extract('{"a":"a\u0041"}','$.a')"#, r"'aA'"),
        (r#"json_extract('{"a":[1]}','$.a')"#, r"'[1]'"),
        (r"json_quote(1)", r"'1'"),
        (r"json_quote(1.5)", r"'1.5'"),
        (r"json_quote('x')", r#"'"x"'"#),
        (r"json_quote(NULL)", r"'null'"),
        (r"json_quote(x'41')", r"JSON cannot hold BLOB values"),
        (r"json_array()", r"'[]'"),
        (r"json_object()", r"'{}'"),
        (r"json_array(1.0)", r"'[1.0]'"),
        (r"json_array(9e999)", r"'[9.0e+999]'"),
        (r"json_array(-9e999)", r"'[-9.0e+999]'"),
        (r"json_object('a',1,'a',2)", r#"'{"a":1,"a":2}'"#),
        (r"json_object(1,2)", r"json_object() labels must be TEXT"),
        (
            r"json_object('a')",
            r"json_object() requires an even number of arguments",
        ),
        (r"json_valid('1')", r"1"),
        (r"json_valid('x')", r"0"),
        (r"json_valid('{a:1}',1)", r"0"),
        (r"json_valid('{a:1}',3)", r"1"),
        (r"json_valid('1',4)", r"0"),
        (r"json_valid(x'0b',4)", r"1"),
        (r"json_valid(x'0b',8)", r"1"),
        (r"json_valid(x'ff',1)", r"0"),
        (r"json_valid(1)", r"1"),
        (
            r"json_valid('1',0)",
            r"FLAGS parameter to json_valid() must be between 1 and 15",
        ),
        (
            r"json_valid('1',16)",
            r"FLAGS parameter to json_valid() must be between 1 and 15",
        ),
        (r"json_error_position('[1,2')", r"5"),
        (r"json_error_position('[1,2]')", r"0"),
        (r"json_error_position(x'0b')", r"0"),
        (r"json_error_position(x'ff')", r"1"),
        (
            r"json_error_position('[' || char(226,130,172) || ']')",
            r"2",
        ),
        (r"json_insert('[1]','$[#]',2)", r"'[1,2]'"),
        (r"json_insert('{}','$.a[0]',1)", r#"'{"a":[1]}'"#),
        (r#"json_set('{"a":1}','$.a',json('[2]'))"#, r#"'{"a":[2]}'"#),
        (r"json_set('[1,2]','$[5]',9)", r"'[1,2]'"),
    ]);
}

#[test]
fn every_json_call_answers_the_same_6() {
    calls_answer(&[
        (r#"json_remove('{"a":1}','$.b')"#, r#"'{"a":1}'"#),
        (r"json_remove('[1,2]','$[5]')", r"'[1,2]'"),
        (r"json_remove('[1,2]','$')", r"NULL"),
        (r"json_replace('{}','$.a',1)", r"'{}'"),
        (r#"json_set('{"a":1}','$.a.b',2)"#, r#"'{"a":1}'"#),
        (r"json_set('[1]','$[0].a',2)", r"'[1]'"),
        (r#"json_set('{"a":[1]}','$.a[1]',2)"#, r#"'{"a":[1,2]}'"#),
        (r#"json_set('{"a":1}','b',2)"#, r"bad JSON path: 'b'"),
        (
            r#"json_set('{"a":1}','$.a',1,'$.b')"#,
            r"json_set() needs an odd number of arguments",
        ),
        (
            r#"json_array_insert('{"a":[1]}','$.a[0]',9)"#,
            r#"'{"a":[9,1]}'"#,
        ),
        (r"json_array_insert('{}','$.a[0]',9)", r#"'{"a":[9]}'"#),
        (
            r"json_array_insert('{}','$.a.b',9)",
            r"not an array element: '$.a.b'",
        ),
        (r"json_patch('1','2')", r"'2'"),
        (r#"json_patch('{"a":1}','1')"#, r"'1'"),
        (r#"json_patch('1','{"a":1}')"#, r#"'{"a":1}'"#),
        (r#"json_patch('{"a":1}','{"b":null}')"#, r#"'{"a":1}'"#),
        (r#"json_patch('{"a":{"b":1}}','{"a":null}')"#, r"'{}'"),
        (r#"'{"a":1}' -> '$.a'"#, r"'1'"),
        (r#"'{"a":1}' -> 'a'"#, r"'1'"),
        (r#"'{"a b":1}' -> 'a b'"#, r"'1'"),
        (r"'[1,2]' -> '[1]'", r"'2'"),
        (r"'[1,2]' -> 1", r"'2'"),
        (r"'[1,2]' -> -1", r"'2'"),
        (r"'[1,2]' ->> 0", r"1"),
        (r#"'{"a":null}' ->> 'a'"#, r"NULL"),
        (r#"'{"a":1}' -> 'b'"#, r"NULL"),
        (r"jsonb('1')", r"X'1331'"),
        (r"jsonb('[1,2]')", r"X'4B13311332'"),
        (r#"jsonb('{"a":1}')"#, r"X'4C17611331'"),
        (r"json(jsonb('{a:1}'))", r#"'{"a":1}'"#),
        (r"jsonb_extract('[1,2]','$[0]')", r"1"),
        (r"jsonb_extract('[1,2]','$[0]','$[1]')", r"X'4B13311332'"),
        (r"jsonb_extract('[1,2]','$[9]')", r"NULL"),
        (
            r#"json(jsonb_patch('{"a":1}','{"b":2}'))"#,
            r#"'{"a":1,"b":2}'"#,
        ),
        (r"json(jsonb_object('a',1))", r#"'{"a":1}'"#),
        (r"json(jsonb_array_insert('[1]','$[0]',0))", r"'[0,1]'"),
        (r"json(jsonb_remove('[1,2]','$[0]'))", r"'[2]'"),
        (r"json(jsonb_insert('[1]','$[1]',2))", r"'[1,2]'"),
        (r"json(jsonb_replace('[1]','$[0]',2))", r"'[2]'"),
    ]);
}

/// The text a connection answers for one statement over a table of
/// four rows.
fn over_rows(sql: &str) -> alloc::string::String {
    use crate::change::Writer;
    use crate::db::Database;
    use crate::header::Encoding;
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a,b)").unwrap();
    writer
        .run(b"INSERT INTO t VALUES(1,'x'),(2.5,'y'),(NULL,'z'),('s','w')")
        .unwrap();
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    match database.query(sql.as_bytes()) {
        Ok(answer) => {
            let value = answer.rows.first().and_then(|row| row.first()).cloned();
            alloc::string::String::from_utf8_lossy(
                &value
                    .unwrap_or(crate::value::Value::Null)
                    .text()
                    .unwrap_or_default(),
            )
            .into_owned()
        }
        Err(error) => error.message(),
    }
}

#[test]
fn every_json_aggregate_answers_what_the_c_library_answers() {
    for (sql, want) in [
        (
            "SELECT quote(json_group_array(a)) FROM t",
            r#"'[1,2.5,null,"s"]'"#,
        ),
        (
            "SELECT quote(json_group_object(b,a)) FROM t",
            r#"'{"x":1,"y":2.5,"z":null,"w":"s"}'"#,
        ),
        (
            "SELECT quote(jsonb_group_array(a)) FROM t",
            r"X'9B133135322E35001773'",
        ),
        (
            "SELECT quote(json_group_array(json_object('k',a))) FROM t",
            r#"'[{"k":1},{"k":2.5},{"k":null},{"k":"s"}]'"#,
        ),
        (
            "SELECT quote(json_group_array(DISTINCT a)) FROM t",
            r#"'[1,2.5,null,"s"]'"#,
        ),
        (
            "SELECT quote(json_group_array(a) OVER ()) FROM t",
            r#"'[1,2.5,null,"s"]'"#,
        ),
        (
            "SELECT quote(json_group_array(CAST(b AS BLOB))) FROM t",
            "JSON cannot hold BLOB values",
        ),
        (
            "SELECT quote(jsonb_group_object(b,a)) FROM t",
            r"X'CC1117781331177935322E35177A0017771773'",
        ),
    ] {
        assert_eq!(over_rows(sql), want, "{sql}");
    }
}

#[test]
fn a_payload_of_every_width_is_written_and_read_again() {
    // The length of a payload is written into the header itself up to
    // eleven bytes, and into one, two or four bytes after it beyond
    // that.
    for width in [1_usize, 11, 12, 300, 70000] {
        let mut text = alloc::vec![b'x'; width];
        text.insert(0, b'"');
        text.push(b'"');
        let (blob, _) = crate::json::read(&text).unwrap();
        let mut out = Vec::new();
        crate::json::write(&blob, 0, &mut out).unwrap();
        assert_eq!(out.len(), width + 2, "{width}");
    }
}
#[test]
fn every_json_call_answers_the_same_7() {
    calls_answer(&[
        (r"json_quote(char(1))", r#"'"\u0001"'"#),
        (r"json_quote(char(8))", r#"'"\b"'"#),
        (r"json_quote(char(9))", r#"'"\t"'"#),
        (r"json_quote(char(10))", r#"'"\n"'"#),
        (r"json_quote(char(12))", r#"'"\f"'"#),
        (r"json_quote(char(13))", r#"'"\r"'"#),
        (r"json_quote(char(11))", r#"'"\u000b"'"#),
        (r"json_quote(char(31))", r#"'"\u001f"'"#),
        (r#"json_quote('a"b')"#, r#"'"a\"b"'"#),
        (r"json_quote('a\b')", r#"'"a\\b"'"#),
        (r"json_quote('a''b')", r#"'"a''b"'"#),
        (r#"json_array(char(1),'a"b')"#, r#"'["\u0001","a\"b"]'"#),
        (r"json_object('a'||char(1),1)", r#"'{"a\u0001":1}'"#),
        (r#"json_extract('{"a\u0041":1}','$."a\u0041"')"#, r"1"),
        (r#"json_extract('{"aA":1}','$."a\u0041"')"#, r"1"),
        (r#"json_extract('{"a\u0041":1}','$.aA')"#, r"1"),
        (r#"json_extract('{"a\tb":1}','$."a\tb"')"#, r"1"),
        (r#"json_extract('{"a\u00e9":1}','$."a\u00e9"')"#, r"1"),
        (
            r#"json_extract('{"a\ud83d\ude00":1}','$."a\ud83d\ude00"')"#,
            r"1",
        ),
        (r#"json_extract('{"a\ud83d":1}','$."a\ud83d"')"#, r"1"),
        (
            r#"json_extract('{"a\qb":1}','$."a\qb"')"#,
            r"malformed JSON",
        ),
        (r"json_extract('{''a\x41'':1}','$.aA')", r"1"),
        (r#"json_extract('{''a\0b'':1}','$."a\u0000b"')"#, r"1"),
        (r#"json_extract('{''a\''b'':1}','$."a''b"')"#, r"1"),
        (r"json_extract('{''a\'||char(10)||'b'':1}','$.ab')", r"1"),
        (
            r"json_extract('{''a\'||char(13)||char(10)||'b'':1}','$.ab')",
            r"1",
        ),
        (
            r"json_extract('{''a\'||char(226,128,168)||'b'':1}','$.ab')",
            r"malformed JSON",
        ),
        (r#"json_extract('{''a\vb'':1}','$."a\u000bb"')"#, r"1"),
        (r#"json_extract('{"a\u":1}','$.x')"#, r"malformed JSON"),
        (r#"json_extract('{"a\x4":1}','$.x')"#, r"malformed JSON"),
        (r#"json_extract('{"a":0x1f}','$.a')"#, r"31"),
        (r#"json_extract('{"a":-0x1f}','$.a')"#, r"-31"),
        (
            r#"json_extract('{"a":0xffffffffffffffff}','$.a')"#,
            r"1.8446744073709552e+19",
        ),
        (
            r#"json_extract('{"a":-9223372036854775808}','$.a')"#,
            r"-9223372036854775808",
        ),
        (
            r#"json_extract('{"a":99999999999999999999}','$.a')"#,
            r"1.0e+20",
        ),
        (r#"json_extract('{"a":1.5e3}','$.a')"#, r"1500.0"),
        (r#"json_extract('{"a":1.}','$.a')"#, r"1.0"),
        (r#"json_extract('{"a":"x\ty"}','$.a')"#, r"'x	y'"),
        (r#"json_extract('{"a":''x''}','$.a')"#, r"'x'"),
        (r#"json_type('{"a":0x1f}','$.a')"#, r"'integer'"),
    ]);
}

#[test]
fn every_json_call_answers_the_same_8() {
    calls_answer(&[
        (r"json_array(jsonb('[1]'))", r"'[[1]]'"),
        (r#"json_quote(jsonb('{"a":1}'))"#, r#"'{"a":1}'"#),
        (r"json(x'0b')", r"'[]'"),
        (r#"json_extract(jsonb('{"a":1}'),'$.a')"#, r"1"),
        (r#"jsonb_extract(jsonb('{"a":1}'),'$.a')"#, r"1"),
        (r"json_valid(x'0b0b',4)", r"0"),
        (
            r"json_object('a',1,2,3)",
            r"json_object() labels must be TEXT",
        ),
        (
            r"json_insert('{}','$.a')",
            r"json_insert() needs an odd number of arguments",
        ),
        (
            r"json_replace('{}','$.a')",
            r"json_replace() needs an odd number of arguments",
        ),
        (
            r"json_set('{}','$.a')",
            r"json_set() needs an odd number of arguments",
        ),
        (
            r"json_array_insert('[]','$[0]')",
            r"json_array_insert() needs an odd number of arguments",
        ),
        (
            r"json_valid('1',-1)",
            r"FLAGS parameter to json_valid() must be between 1 and 15",
        ),
        (r"json_extract('{}','x')", r"bad JSON path: 'x'"),
        (r"json_remove('{}','x')", r"bad JSON path: 'x'"),
        (r"json('[1,2,3')", r"malformed JSON"),
        (
            r"json_set('{}','$.a.b.c.d',1)",
            r#"'{"a":{"b":{"c":{"d":1}}}}'"#,
        ),
        (r"json_set('{}','$.a[0][1]',1)", r"'{}'"),
        (r"json_set('[]','$[0].a',1)", r#"'[{"a":1}]'"#),
        (r#"json_insert('{"a":[1]}','$.a[#]',2)"#, r#"'{"a":[1,2]}'"#),
        (r#"json_insert('{"a":{}}','$.a.b',2)"#, r#"'{"a":{"b":2}}'"#),
        (r#"json_remove('{"a":{"b":1}}','$.a.b')"#, r#"'{"a":{}}'"#),
        (r"json_remove('[[1,2]]','$[0][1]')", r"'[[1]]'"),
        (r#"json_replace('{"a":[1]}','$.a[0]',9)"#, r#"'{"a":[9]}'"#),
        (r#"json_set('{"a":1}','$[0]',9)"#, r#"'{"a":1}'"#),
        (r"json_set('[1]','$.a',9)", r"'[1]'"),
        (r#"json_patch('{"a":[1]}','{"a":[2]}')"#, r#"'{"a":[2]}'"#),
        (r"json_patch('[1]','[2]')", r"'[2]'"),
        (r#"json_patch('{"a":1}','{}')"#, r#"'{"a":1}'"#),
        (r#"json_patch('{}','{"a":{"b":1}}')"#, r#"'{"a":{"b":1}}'"#),
        (r#"'{"a-b":1}' -> 'a-b'"#, r"'1'"),
        (r"'[1]' -> '[0]'", r"'1'"),
        (r"'[1]' -> '[9]'", r"NULL"),
        (r#"'{"1":2}' -> 1"#, r"NULL"),
        (r"'[[1]]' ->> '$[0][0]'", r"1"),
    ]);
}

#[test]
fn the_text_json_pretty_writes_is_the_text_the_c_library_writes() {
    let breaks = |text: &str| text.replace('|', "\n");
    for (sql, want) in [
        (r"json_pretty('[{},[],1]')", "'[|    {},|    [],|    1|]'"),
        (r#"json_pretty('{"a":{}}','')"#, "'{|\"a\": {}|}'"),
    ] {
        assert_eq!(quoted(sql), breaks(want), "{sql}");
    }
}
#[test]
fn every_json_call_answers_the_same_9() {
    calls_answer(&[
        (r"json(x'0d')", r"malformed JSON"),
        (r"json(x'031f')", r"malformed JSON"),
        (r"json(x'0300')", r"malformed JSON"),
        (r"json(x'1b31')", r"malformed JSON"),
        (r"json(x'2c1731')", r"malformed JSON"),
        (r"json(x'1431')", r"'0'"),
        (r"json(x'2b1331')", r"'[1]'"),
        (r"json_type(x'0d')", r"malformed JSON"),
        (r"json(x'25312e')", r"'1.'"),
        (r"json(x'0b13')", r"malformed JSON"),
        (r"json(x'0500')", r"malformed JSON"),
        (r"json(x'1531')", r"'1'"),
        (r"json(x'05')", r"malformed JSON"),
        (r"json(x'03')", r"malformed JSON"),
        (r"json(x'cb01')", r"malformed JSON"),
        (r"json(x'2c17611773')", r"malformed JSON"),
        (r"json(x'1c1761')", r"malformed JSON"),
        (r"json(x'4b1331133213331334')", r"malformed JSON"),
        (r"json_extract(x'2b1331','$[0]')", r"1"),
        (r"json_extract(x'2c17611331','$.a')", r"malformed JSON"),
        (r"json_array_length(x'2b1331')", r"1"),
        (r"json_valid(x'2b1331')", r"0"),
        (r"json_valid(x'0d',4)", r"0"),
        (r"json_error_position(x'2b1331')", r"0"),
        (r"json('['||char(0)||'1]')", r"malformed JSON"),
        (r#"json('"a'||char(0)||'b"')"#, r"malformed JSON"),
        (r"json(char(0))", r"malformed JSON"),
        (r"json('''a\'||char(226,128,169)||'b''')", r"malformed JSON"),
        (r"json('''a\'||char(226,128)||'b''')", r"malformed JSON"),
        (r"json('''a\'||char(226,128,170)||'b''')", r"malformed JSON"),
        (r"json('-0x1F')", r"'-31'"),
        (r"json('+0X1f')", r"'31'"),
        (r"json('-0xg')", r"malformed JSON"),
        (r"json('0xg')", r"malformed JSON"),
        (r"json('-00')", r"malformed JSON"),
        (r"json('0e1')", r"'0e1'"),
        (r"json('1e1e1')", r"malformed JSON"),
        (r"json('.e1')", r"malformed JSON"),
        (r"json('1.e')", r"malformed JSON"),
        (r"json('-.e1')", r"malformed JSON"),
    ]);
}

#[test]
fn every_json_call_answers_the_same_10() {
    calls_answer(&[
        (r"json('1.2e')", r"malformed JSON"),
        (r"json('[1/*c*/]')", r"'[1]'"),
        (r"json('{a:1/*c*/}')", r#"'{"a":1}'"#),
        (r"json('{a:1,/*c*/}')", r#"'{"a":1}'"#),
        (r"json('[1,/*c*/]')", r"'[1]'"),
        (r"json('{a'||char(10)||':1}')", r#"'{"a":1}'"#),
        (r"json('{a:/*c*/1}')", r#"'{"a":1}'"#),
        (r#"json('{"a"'||char(10)||':1}')"#, r#"'{"a":1}'"#),
        (r"json('[1,2')", r"malformed JSON"),
        (r"json('{a:1')", r"malformed JSON"),
        (r"json('{a:1,')", r"malformed JSON"),
        (r"json('[')", r"malformed JSON"),
        (r"json('{')", r"malformed JSON"),
        (r"json('{:1}')", r"malformed JSON"),
        (r"json('[1]]')", r"malformed JSON"),
        (r"json('{a:1}}')", r"malformed JSON"),
        (
            r"json(printf('%.1001c','[')||printf('%.1001c',']'))",
            r"malformed JSON",
        ),
        (r"json_extract('{}','$..a')", r"bad JSON path: '$..a'"),
        (r"json_extract('{}','$[')", r"NULL"),
        (r"json_extract('{}','$[x]')", r"NULL"),
        (r"json_extract('[]','$[0')", r"bad JSON path: '$[0'"),
        (r#"json_extract('{}','$."')"#, r#"bad JSON path: '$."'"#),
        (r"json_extract('{}','$a')", r"bad JSON path: '$a'"),
        (r"json_extract('[[1]]','$[0][0]')", r"1"),
        (r"json_set('{}','$.a''b',1)", r#"'{"a''b":1}'"#),
        (r"'[1,2]' -> '[1]'", r"'2'"),
        (r#"'{"a":1}' -> '$.a'"#, r"'1'"),
        (r#"'{"a":1}' ->> '$.a'"#, r"1"),
        (r#"'{"a":[1]}' -> '$.a'"#, r"'[1]'"),
        (r"json_quote(1e400)", r"'9.0e+999'"),
        (r"json_quote(-1e400)", r"'-9.0e+999'"),
        (r"json_array(1e400,-1e400)", r"'[9.0e+999,-9.0e+999]'"),
    ]);
}

/// The value the text holds, as the binary form.
fn blob_of(text: &[u8]) -> Vec<u8> {
    crate::json::read(text).unwrap().0
}

#[test]
fn the_whitespace_and_the_comments_json5_allows_are_read() {
    // Every whitespace Unicode names, one per shape the reader has a
    // case for.
    for spaces in [
        "\u{a0}".as_bytes(),
        "\u{1680}".as_bytes(),
        "\u{2000}".as_bytes(),
        "\u{2028}".as_bytes(),
        "\u{202f}".as_bytes(),
        "\u{205f}".as_bytes(),
        "\u{3000}".as_bytes(),
        "\u{feff}".as_bytes(),
        b"\x0b\x0c",
        b"/*c*/",
        b"//c\n",
        b"//c\xe2\x80\xa8",
    ] {
        let mut text = spaces.to_vec();
        text.extend_from_slice(b"[1]");
        assert_eq!(blob_of(&text), blob_of(b"[1]"), "{spaces:?}");
    }
    // A comment that nothing closes runs to the end of the text, and a
    // solidus that begins no comment is no whitespace.
    assert_eq!(crate::json::read(b"[1]/*c").unwrap().0, blob_of(b"[1]"));
    assert!(crate::json::read(b"/[1]").is_err());
    assert!(crate::json::read(b"[1]/").is_err());
    assert!(crate::json::read(b"/*").is_err());
}

#[test]
fn the_escapes_json5_allows_are_read_and_written() {
    // A line break after a backslash carries no character of its own,
    // and the two paragraph breaks Unicode names are read the same
    // way.
    for (held, want) in [
        (b"'a\\\xe2\x80\xa9b'".as_slice(), r#""ab""#),
        (b"'a\\\xe2\x80\xa8b'", r#""ab""#),
        (b"'a\\\rb'", r#""ab""#),
        (b"'a\\\r\nb'", r#""ab""#),
        (b"'a\\\nb'", r#""ab""#),
    ] {
        let blob = blob_of(held);
        let mut out = Vec::new();
        crate::json::write(&blob, 0, &mut out).unwrap();
        assert_eq!(out, want.as_bytes(), "{held:?}");
    }
    // A backslash before something that is neither is no escape.
    assert!(crate::json::read(b"'a\\\xe2\x80\x80b'").is_err());
    assert!(crate::json::read(b"'a\\\xe2b'").is_err());
}

#[test]
fn what_the_binary_form_holds_that_no_text_holds_is_refused() {
    use crate::value::Value;
    for blob in [
        // `null`, `true` and `false` carry no payload.
        b"\x10\x00".as_slice(),
        b"\x11\x00",
        b"\x12\x00",
        // An element that does not end where the blob does.
        b"\x1b\x13",
        b"\x2c\x17\x61",
        b"\x0b\x13",
        // A pair of an object with no value after its label.
        b"\x2c\x17\x61\x17",
        // A number with no digits at all.
        b"\x03",
        b"\x05",
    ] {
        let held = Value::Blob(blob.to_vec());
        assert!(crate::json::minified(&held).is_err(), "{blob:?}");
    }
    // A blob that begins with a byte JSON text begins with is read
    // whole before it is taken for the binary form: this one holds an
    // element that runs past its end.
    let held = Value::Blob(b"\x5b\x13\x31\x13\x32\x13".to_vec());
    assert!(crate::json::minified(&held).is_err());
    let held = Value::Blob(b"\x5b\x13\x31\x13\x32\x10".to_vec());
    assert!(crate::json::minified(&held).is_err());
    // The same blob with whole elements is read as the binary form.
    let held = Value::Blob(b"\x5b\x13\x31\x13\x32\x00".to_vec());
    assert_eq!(
        crate::json::minified(&held),
        Ok(Value::Text(b"[1,2,null]".to_vec()))
    );
    // A label that is not text stands where the blob is not read
    // whole.
    let held = Value::Blob(b"\x4c\x13\x31\x13\x32".to_vec());
    assert_eq!(
        crate::json::minified(&held),
        Ok(Value::Text(b"{1:2}".to_vec()))
    );
}

#[test]
fn every_text_the_reader_stops_at_is_refused() {
    for text in [
        // A byte that begins a space of Unicode and is followed by
        // something else.
        b"\xc2\x41[1]".as_slice(),
        b"\xe1\x9a\x41[1]",
        b"\xe2\x80\x41[1]",
        b"\xe2\x41[1]",
        b"\xe3\x80\x41[1]",
        b"\xef\xbb\x41[1]",
        // An escape of a nought that a digit follows.
        b"'a\\05b'",
        // A number with no digit before the point of it.
        b"-.e5",
        // A colon where a comma or a bracket belongs.
        b"[1/*c*/:2]",
        // A label with no colon after it.
        b"{a 1}",
        b"{a/*c*/1}",
    ] {
        assert!(crate::json::read(text).is_err(), "{text:?}");
    }
    for (text, want) in [
        // A solidus inside a comment that closes it nowhere.
        (b"/*a/b*/[1]".as_slice(), b"[1]".as_slice()),
        (b"//a\n[1]", b"[1]"),
        (b"//a\r[1]", b"[1]"),
        (b"//a\xe2\x41\n[1]", b"[1]"),
        (b"5.", b"5.0"),
        (b".5", b"0.5"),
    ] {
        let blob = blob_of(text);
        let mut out = Vec::new();
        crate::json::write(&blob, 0, &mut out).unwrap();
        assert_eq!(out, want, "{text:?}");
    }
}

#[test]
fn a_value_that_nests_deeper_than_the_reader_walks_is_refused() {
    for (open, close) in [(b'[', b']'), (b'{', b'}')] {
        let mut text = alloc::vec![open; 1001];
        if open == b'{' {
            // Every object needs a label to hold the next one.
            text = Vec::new();
            for _ in 0..1001 {
                text.extend_from_slice(b"{a:");
            }
            text.extend_from_slice(b"1");
        }
        text.extend(core::iter::repeat_n(close, 1001));
        assert!(crate::json::read(&text).is_err());
    }
}

#[test]
fn a_binary_form_whose_element_runs_past_the_one_that_holds_it_is_refused() {
    // The array inside says it holds two bytes and its element holds
    // five, which the walk of the outer array reaches.
    let held = crate::value::Value::Blob(b"\xcb\x05\x2b\x33\x31\x32\x33".to_vec());
    assert!(crate::json::minified(&held).is_err());
}
#[test]
fn every_json_call_answers_the_same_11() {
    calls_answer(&[
        (r"json('-.')", r"malformed JSON"),
        (r"json('{a\qb:1}')", r"malformed JSON"),
        (r"json('{a\u00:1}')", r"malformed JSON"),
        (r"json(x'162d')", r"malformed JSON"),
        (
            r"json(char(34)||'a'||char(39)||'b'||char(1)||char(34))",
            r#"'"a''b\u0001"'"#,
        ),
        (r"json(x'395ce241')", r"malformed JSON"),
        (r#"json_extract('{}','$."a\')"#, r#"bad JSON path: '$."a\'"#),
        (r"json_extract('[1]','$[99999999999999]')", r"NULL"),
        (r"json_extract('[1]','$[#-]')", r"bad JSON path: '$[#-]'"),
        (r"json_extract('[1]','$[#-99999999999999]')", r"NULL"),
        (r"json_extract('[1]','$[#x]')", r"bad JSON path: '$[#x]'"),
        (r"json_extract(x'4c13311332','$.a')", r"malformed JSON"),
    ]);
}
#[test]
fn every_json_call_answers_the_same_12() {
    calls_answer(&[
        (r"json_extract(x'5c28615c1331','$.x')", r"NULL"),
        (r"json(x'01')", r"'true'"),
        (r"json(x'7b13311332133313341335')", r"malformed JSON"),
        (r"json(x'31616263')", r"malformed JSON"),
        (r"json_array(-1.5)", r"'[-1.5]'"),
        (r"json_set('{}','$x',1)", r"bad JSON path: '$x'"),
        (r#"json_set('{"a":1,"b":2}','$.b',3)"#, r#"'{"a":1,"b":3}'"#),
        (r#"json_set('{}','$."a\u0041"',1)"#, r#"'{"a\u0041":1}'"#),
        (r"json_extract('{}','x''y')", r"bad JSON path: 'x''y'"),
        (
            r#"json_patch('{"a\u0041":1}','{"aA":2}')"#,
            r#"'{"a\u0041":2}'"#,
        ),
        (r"json_insert('[1,2]','$[9]',3)", r"'[1,2]'"),
    ]);
}
#[test]
fn every_json_call_answers_the_same_13() {
    calls_answer(&[
        (r"json(x'495ce24142')", r"malformed JSON"),
        (
            r#"json_extract('{"a":-0xffffffffffffffff}','$.a')"#,
            r"-1.8446744073709552e+19",
        ),
        (
            r#"json_set('{"a\u0041":1}','$.x',2)"#,
            r#"'{"a\u0041":1,"x":2}'"#,
        ),
        (r"json_remove('[1]','$[1]')", r"'[1]'"),
        (r"json_replace('[1]','$[1]',9)", r"'[1]'"),
        (
            r#"json_patch('{"a":1}','{"a\u0041":2}')"#,
            r#"'{"a":1,"a\u0041":2}'"#,
        ),
        (r"'[1]' -> '[]'", r"NULL"),
        (r"'[1]' -> '[1'", r"NULL"),
        (r#"jsonb_extract('{"a":{"b":1}}','$.a')"#, r"X'4C17621331'"),
        (
            r"json_extract(x'5c28615c30311331','$.x')",
            r"malformed JSON",
        ),
        (r"json(x'7c17611331')", r"malformed JSON"),
    ]);
}

#[test]
fn a_path_that_steps_deeper_than_the_walk_goes_is_refused() {
    use crate::value::Value;
    // A value of a thousand levels, which is as deep as the reader
    // holds, and a path of one step more than that. The walk is run on
    // a stack of its own, because a thousand frames of it are more
    // than a test thread is given.
    let refused = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let mut deep = Vec::new();
            for _ in 0..1000 {
                deep.extend_from_slice(b"{\"a\":");
            }
            deep.extend_from_slice(b"1");
            deep.extend(core::iter::repeat_n(b'}', 1000));
            let mut path = alloc::string::String::from("$");
            for _ in 0..1001 {
                path.push_str(".a");
            }
            let held = Value::Text(deep);
            let paths = alloc::vec![held.clone(), Value::Text(path.clone().into_bytes())];
            let read = crate::json::extract(&paths).unwrap_err().message();
            let values = alloc::vec![held, Value::Text(path.into_bytes()), Value::Int(1)];
            let written = crate::json::changed(&values, &[], crate::json::Edit::Set)
                .unwrap_err()
                .message();
            (read, written)
        })
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(refused.0, "JSON path too deep");
    assert_eq!(refused.1, "JSON path too deep");
}

#[test]
fn a_patch_that_nests_deeper_than_the_merge_goes_is_refused() {
    use crate::value::Value;
    let mut patch = Vec::new();
    for _ in 0..1001 {
        patch.extend_from_slice(b"{\"a\":");
    }
    patch.extend_from_slice(b"1");
    patch.extend(core::iter::repeat_n(b'}', 1001));
    // The patch itself is deeper than a value nests, so reading it is
    // what refuses first.
    let held = alloc::vec![Value::Text(b"{}".to_vec()), Value::Text(patch.clone()),];
    assert_eq!(
        crate::json::patched(&held).unwrap_err().message(),
        "malformed JSON"
    );
    // A patch the reader holds, merged into a target that nests as
    // deep, reaches the depth the merge stops at. The merge is run on
    // a stack of its own, because a thousand frames of it are more
    // than a test thread is given.
    let held = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let mut deep = Vec::new();
            for _ in 0..999 {
                deep.extend_from_slice(b"{\"a\":");
            }
            deep.extend_from_slice(b"1");
            deep.extend(core::iter::repeat_n(b'}', 999));
            let held = alloc::vec![Value::Text(deep.clone()), Value::Text(deep)];
            crate::json::patched(&held).is_ok()
        })
        .unwrap()
        .join()
        .unwrap();
    assert!(held);
}

#[test]
fn an_element_that_runs_past_the_one_that_holds_it_is_refused_by_a_path() {
    use crate::value::Value;
    // An object whose pair has no value, and an array whose element
    // runs past its end, each reached by a path rather than by the
    // walk that writes the text.
    for (blob, path) in [
        (b"\xcb\x03\x2c\x17\x61".as_slice(), b"$[0].a".as_slice()),
        (b"\xcb\x05\x2b\x33\x31\x32\x33", b"$[0][1]"),
        (b"\xcb\x04\x2c\x17\x61\x13", b"$[0].b"),
    ] {
        let held = alloc::vec![Value::Blob(blob.to_vec()), Value::Text(path.to_vec())];
        assert_eq!(
            crate::json::extract(&held).unwrap_err().message(),
            "malformed JSON",
            "{blob:?}"
        );
    }
}

#[test]
fn every_escape_a_label_of_a_blob_holds_is_read() {
    calls_answer(&[
        (
            r"json_extract(x'7C48615C62621337','$.a'||char(8)||'b')",
            r"7",
        ),
        (
            r"json_extract(x'7C48615C66621337','$.a'||char(12)||'b')",
            r"7",
        ),
        (
            r"json_extract(x'7C48615C6E621337','$.a'||char(10)||'b')",
            r"7",
        ),
        (
            r"json_extract(x'7C48615C72621337','$.a'||char(13)||'b')",
            r"7",
        ),
        (r"json_extract(x'7C49615C30311337','$.a')", r"NULL"),
        (r"json_extract(x'6C39615C781337','$.a')", r"NULL"),
        (r"json_extract(x'7C49615C0D621337','$.ab')", r"7"),
        (r"json_extract(x'8C59615C0D0A621337','$.ab')", r"7"),
        (r"json_extract(x'9C69615CE280A8621337','$.ab')", r"7"),
        (r"json_extract(x'6C39615C711337','$.a')", r"NULL"),
        (r"json_extract(x'8C58615C7531321337','$.a')", r"NULL"),
        (
            r"json_extract(x'CC12C80E615C75643833645C7564653030621337','$.a'||char(128512)||'b')",
            r"7",
        ),
        (r"json_extract(x'BC88615C75643833647A1337','$.ax')", r"NULL"),
        (
            r"json_extract(x'CC0DA8615C75643833645C747A1337','$.ax')",
            r"NULL",
        ),
        (
            r"json_extract(x'CC0EB8615C75643833645C7531321337','$.ax')",
            r"NULL",
        ),
        (
            r"json_extract(x'CC11C80D615C75643833645C75303034311337','$.ax')",
            r"NULL",
        ),
        (r"json_extract(x'8C59615C0A20621337','$.a b')", r"7"),
    ]);
}

#[test]
fn every_json_call_answers_the_same_14() {
    calls_answer(&[
        (r#"json_extract('{"a":false}','$.a')"#, r"0"),
        (
            r"json_extract('[-9223372036854775808]','$[0]')",
            r"-9223372036854775808",
        ),
        (r"json(x'04')", r"malformed JSON"),
        (r"jsonb_extract('[1]','$[0]','$[9]')", r"X'3B133100'"),
        (r#"'{"[1x":5}' -> '[1x'"#, r"'5'"),
        (r"json(1)", r"'1'"),
        (r"json(1.5)", r"'1.5'"),
        (
            r"json_extract(x'6B4C1761233132','$[0].b')",
            r"malformed JSON",
        ),
        (
            r"json_extract(x'6B4C1761233132','$[0].a')",
            r"malformed JSON",
        ),
        (r"json_group_array(1e400)", r"'[9.0e+999]'"),
        (r"json_group_array(-1e400)", r"'[-9.0e+999]'"),
        (r"json_group_array(jsonb('[1]'))", r"'[[1]]'"),
        (
            r"json_group_array(x'0102')",
            r"JSON cannot hold BLOB values",
        ),
    ]);
}

#[test]
fn every_element_of_a_blob_is_read_again_where_the_flag_asks_for_it() {
    calls_answer(&[
        (r"json_valid(x'0C',8)", r"1"),
        (r"json_valid(x'1B33',8)", r"0"),
        (r"json_valid(x'2B1731',8)", r"1"),
        (r"json_valid(x'4C13311332',8)", r"0"),
        (r"json_valid(x'2C1761',8)", r"0"),
        (r"json_valid(x'4C17611331',8)", r"1"),
        (r"json_valid(x'4B1B233132',8)", r"0"),
    ]);
}

/// A blob of `levels` arrays, one inside the next, holding `1`.
fn nested(levels: usize) -> Vec<u8> {
    let mut blob = Vec::new();
    crate::json::append(&mut blob, crate::json::INT, b"1");
    for _ in 0..levels {
        let mut over = Vec::new();
        crate::json::append(&mut over, crate::json::ARRAY, &blob);
        blob = over;
    }
    blob
}

/// A blob of `levels` objects, one the value of the label `a` of the
/// next, holding `1`.
fn nested_pairs(levels: usize) -> Vec<u8> {
    let mut blob = Vec::new();
    crate::json::append(&mut blob, crate::json::INT, b"1");
    for _ in 0..levels {
        let mut body = Vec::new();
        crate::json::append(&mut body, crate::json::TEXT, b"a");
        body.extend_from_slice(&blob);
        blob = Vec::new();
        crate::json::append(&mut blob, crate::json::OBJECT, &body);
    }
    blob
}

#[test]
fn a_blob_that_nests_deeper_than_the_walk_goes_is_no_whole_one() {
    use crate::value::Value;
    // The walk is run on a stack of its own, because a thousand frames
    // of it are more than a test thread is given.
    let held = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let valid =
                |blob: Vec<u8>| crate::json::valid(&[Value::Blob(blob), Value::Int(8)]).unwrap();
            let patch = alloc::vec![
                Value::Text(br#"{"a":1}"#.to_vec()),
                Value::Blob(nested_pairs(1001)),
            ];
            (
                valid(nested(1001)),
                valid(nested(998)),
                crate::json::patched(&patch).unwrap_err().message(),
            )
        })
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(held.0, Value::Int(0));
    assert_eq!(held.1, Value::Int(1));
    assert_eq!(held.2, "JSON nested too deep");
}

#[test]
fn every_json_call_answers_the_same_15() {
    calls_answer(&[
        (r"json_extract(x'8C59615CE241621337','$.a')", r"NULL"),
        (r"json_group_array(-1.5)", r"'[-1.5]'"),
        (r"json(x'7B13311332233333')", r"'[1,2,33]'"),
        (r"json_object('a',x'0102')", r"JSON cannot hold BLOB values"),
        (
            r"json_extract('[9223372036854775808]','$[0]')",
            r"9.2233720368547758e+18",
        ),
    ]);
}

/// The fifteen cases of appendix A of RFC 7396, *JSON Merge Patch*,
/// P. Hoffman, J. Snell, October 2014, at `docs/rfc/rfc7396.txt` line
/// 399, which `json_patch` runs.
#[test]
fn every_example_of_rfc_7396_is_patched_as_the_document_says() {
    calls_answer(&[
        (r#"json_patch('{"a":"b"}','{"a":"c"}')"#, r#"'{"a":"c"}'"#),
        (
            r#"json_patch('{"a":"b"}','{"b":"c"}')"#,
            r#"'{"a":"b","b":"c"}'"#,
        ),
        (r#"json_patch('{"a":"b"}','{"a":null}')"#, r"'{}'"),
        (
            r#"json_patch('{"a":"b","b":"c"}','{"a":null}')"#,
            r#"'{"b":"c"}'"#,
        ),
        (r#"json_patch('{"a":["b"]}','{"a":"c"}')"#, r#"'{"a":"c"}'"#),
        (
            r#"json_patch('{"a":"c"}','{"a":["b"]}')"#,
            r#"'{"a":["b"]}'"#,
        ),
        (
            r#"json_patch('{"a":{"b":"c"}}','{"a":{"b":"d","c":null}}')"#,
            r#"'{"a":{"b":"d"}}'"#,
        ),
        (
            r#"json_patch('{"a":[{"b":"c"}]}','{"a":[1]}')"#,
            r#"'{"a":[1]}'"#,
        ),
        (r#"json_patch('["a","b"]','["c","d"]')"#, r#"'["c","d"]'"#),
        (r#"json_patch('{"a":"b"}','["c"]')"#, r#"'["c"]'"#),
        (r#"json_patch('{"a":"foo"}','null')"#, r"'null'"),
        (r#"json_patch('{"a":"foo"}','"bar"')"#, r#"'"bar"'"#),
        (
            r#"json_patch('{"e":null}','{"a":1}')"#,
            r#"'{"e":null,"a":1}'"#,
        ),
        (
            r#"json_patch('[1,2]','{"a":"b","c":null}')"#,
            r#"'{"a":"b"}'"#,
        ),
        (
            r#"json_patch('{}','{"a":{"bb":{"ccc":null}}}')"#,
            r#"'{"a":{"bb":{}}}'"#,
        ),
    ]);
}

#[test]
fn a_label_written_raw_is_read_by_the_path_that_names_it() {
    calls_answer(&[
        (r"json_extract(x'4C1A611337','$.a')", r"7"),
        (r"json_set(x'4C1A611337','$.a',9)", r#"'{"a":9}'"#),
        (r"json_patch(x'4C1A611337',x'4C1A611338')", r#"'{"a":8}'"#),
        (
            r"json_patch(x'4C1A611337',x'4C1A621338')",
            r#"'{"a":7,"b":8}'"#,
        ),
        (r#"'{"a_b":5}' -> 'a_b'"#, r"'5'"),
    ]);
}

/// A call that answers one of its arguments as it stands answers the
/// subtype of that argument, which `sqlite3_result_value` copies with the
/// value, so JSON reaches the call around it as JSON and not as text.
#[test]
fn what_carries_the_json_of_an_argument() {
    calls_answer(&[
        (
            "json_insert('{}','$.a',coalesce(null,json('{b:5}')))->>'$.a.b'",
            "5",
        ),
        // Text no `json()` answered is text, which the call around it
        // writes as one string.
        (
            "json_insert('{}','$.a',coalesce(null,'{b:5}'))->>'$.a.b'",
            "NULL",
        ),
        (
            "json_insert('{}','$.a',iif(1,json('{b:5}'),123))->>'$.a.b'",
            "5",
        ),
        (
            "json_insert('{}','$.a',iif(0,123,json('{b:5}')))->>'$.a.b'",
            "5",
        ),
        (
            "json_insert('{}','$.a',ifnull(NULL,json('{b:5}')))->>'$.a.b'",
            "5",
        ),
        (
            "json_insert('{}','$.a',nullif(json('{b:5}'),8))->>'$.a.b'",
            "5",
        ),
        (
            "json_insert('{}','$.a',min('~',json('{b:5}')))->>'$.a.b'",
            "5",
        ),
        (
            "json_insert('{}','$.a',max('...',json('{b:5}')))->>'$.a.b'",
            "5",
        ),
        (
            "json_insert('{}','$.a',likely(json('{b:5}')))->>'$.a.b'",
            "5",
        ),
        // A sign before the value carries the subtype, and a sign that
        // computes a number answers a number.
        (
            "json_array(+json_extract('{\"x\":[1,2]}','$.x'))",
            "'[[1,2]]'",
        ),
        (
            "json_array(++json_extract('{\"x\":[1,2]}','$.x'))",
            "'[[1,2]]'",
        ),
        ("json_array(-json_extract('{\"x\":[1,2]}','$.x'))", "'[0]'"),
        // A call that answers none of its arguments answers no subtype.
        (
            "json_array(nullif(json('[1,2]'),json('[1,2]')))",
            "'[null]'",
        ),
        ("json_array(min(json('[1,2]'),'~'))", "'[[1,2]]'"),
    ]);
}
