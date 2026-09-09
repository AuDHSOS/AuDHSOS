// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::eval;
use crate::{Error, Limits, Runtime, SilentHost, Value, compile};

#[test]
fn regexp_literals_constructor_and_basic_methods() {
    for (source, expected) in [
        ("/a+/.test('aaa')", Value::Boolean(true)),
        ("/a+/.test('bbb')", Value::Boolean(false)),
        ("/a(b+)/.exec('xxabbb')[1]", Value::string("bbb")),
        ("/x/.exec('abc')", Value::Null),
        ("/a/.exec('ba').index", Value::Number(1.0)),
        ("/a/.exec('ba').input", Value::string("ba")),
        ("/(a)?b/.exec('b')[1]", Value::Undefined),
        ("new RegExp('a+').test('baaa')", Value::Boolean(true)),
        ("RegExp('a','ms').flags", Value::string("ms")),
        ("RegExp().source", Value::string("(?:)")),
        ("/a\\/b/.source", Value::string("a\\/b")),
        ("/[a/]+/.test('a/a')", Value::Boolean(true)),
        ("String(/a/g)", Value::string("/a/g")),
        ("/a/g.global", Value::Boolean(true)),
        ("/./s.test('\\n')", Value::Boolean(true)),
        ("/^a$/m.test('x\\na\\ny')", Value::Boolean(true)),
        ("8 / 2 / 2", Value::Number(2.0)),
        ("let n=8;n/=2;n", Value::Number(4.0)),
        (
            "function f(){return /a/;}f().test('a')",
            Value::Boolean(true),
        ),
        (
            "let yes=false;if(true)yes=/a/.test('a');yes",
            Value::Boolean(true),
        ),
        ("'xxabbb'.match(/a(b+)/)[1]", Value::string("bbb")),
        ("'a1a2'.match(/a./g).join()", Value::string("a1,a2")),
        ("'abc'.search(/b/)", Value::Number(1.0)),
        ("'abc'.search(/z/)", Value::Number(-1.0)),
        ("'abc'.match('b')[0]", Value::string("b")),
        ("'ab'.match(/(?:)/g).length", Value::Number(3.0)),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
}

#[test]
fn last_index_sticky_and_indices_follow_utf16_offsets() {
    for (source, expected) in [
        ("let r=/a/g;r.exec('ba');r.lastIndex", Value::Number(2.0)),
        (
            "let r=/a/g;r.lastIndex=2;r.exec('a');r.lastIndex",
            Value::Number(0.0),
        ),
        (
            "let r=/a/;r.lastIndex=3;r.exec('a');r.lastIndex",
            Value::Number(3.0),
        ),
        ("let r=/a/y;r.test('ba')", Value::Boolean(false)),
        (
            "let r=/a/y;r.lastIndex=1;r.test('ba')",
            Value::Boolean(true),
        ),
        (
            "let r=/a/g;r.lastIndex=-1;r.exec('a')[0]",
            Value::string("a"),
        ),
        (
            "let r=/a/g;r.lastIndex=Infinity;r.exec('a');r.lastIndex",
            Value::Number(0.0),
        ),
        (
            "let r=/a/g;r.lastIndex=2;'ba'.search(r);r.lastIndex",
            Value::Number(2.0),
        ),
        (
            "/(a)(b)?/d.exec('xa').indices[1].join()",
            Value::string("1,2"),
        ),
        ("/(a)(b)?/d.exec('xa').indices[2]", Value::Undefined),
        ("/./d.exec('😀').indices[0].join()", Value::string("0,1")),
        ("let r=/(?:)/g;r.exec('a');r.lastIndex", Value::Number(0.0)),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
}

#[test]
fn unsupported_patterns_never_fall_back_and_budgets_terminate() -> Result<(), Error> {
    for source in [
        "/a/gg",
        "/a/z",
        "/(a)\\1/",
        "/(?=ab)/",
        "/a/u",
        "/a/i",
        "/a/v",
        "/(a?)+/",
        "/unterminated",
        "/a\nb/",
        "/[abc/",
    ] {
        assert!(compile(source, Limits::default()).is_err(), "{source}");
    }
    assert!(matches!(
        eval("RegExp('(a)\\\\1')"),
        Err(Error::Unsupported { .. })
    ));
    let limits = Limits {
        fuel: 100,
        ..Limits::default()
    };
    let source = "/(a+)+$/.test('aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!')";
    assert!(matches!(
        Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost),
        Err(Error::Limit { .. })
    ));
    assert_eq!(
        eval("let r=/a/g;Object.freeze(r);try{r.exec('a');}catch(e){e.name}"),
        Ok(Value::string("TypeError"))
    );
    Ok(())
}

#[test]
fn wpt_harness_first_pattern_matches_arrow_source() {
    assert_eq!(
        eval("let r=/^\\(\\)\\s*=>\\s*(?:{(.*)}\\s*|(.*))$/;r.exec('() => {42}')[1]"),
        Ok(Value::string("42"))
    );
    assert_eq!(
        eval(
            "let r=/([\\ud800-\\udbff]+)(?![\\udc00-\\udfff])|(^|[^\\ud800-\\udbff])([\\udc00-\\udfff]+)/g;r.exec('\\ud800\\ud800\\udc00')[1].length"
        ),
        Ok(Value::Number(1.0))
    );
}

#[test]
fn regex_split_handles_captures_empty_matches_limits_and_original_last_index() {
    for (source, expected) in [
        ("'a1b2c'.split(/(\\d)/).join('|')", "a|1|b|2|c"),
        ("'a1b2c'.split(/(\\d)/,2).join('|')", "a|1"),
        ("'a1b2c'.split(/(\\d)/,0).join('|')", ""),
        ("'ab'.split(/a*/).join('|')", "|b"),
        ("'ab'.split(/a*?/).join('|')", "a|b"),
        ("'ab'.split(/()/).join('|')", "a||b"),
        ("'ab'.split(/$/).join('|')", "ab"),
        ("'ab'.split(/^/).join('|')", "ab"),
        ("'ab'.split(/b/).join('|')", "a|"),
        (
            "'a1b'.split(/(x)?1/).map(x=>String(x)).join('|')",
            "a|undefined|b",
        ),
        ("'a,b'.split(/,/y).join('|')", "a|b"),
        ("'a,b'.split(/,/g).join('|')", "a|b"),
        (
            "'${name}: ${value}'.split(/\\$\\{([^ }]*)\\}/g).join('|')",
            "|name|: |value|",
        ),
        (
            "let r=/,/g;r.lastIndex=8;Object.freeze(r);'a,b'.split(r);String(r.lastIndex)",
            "8",
        ),
    ] {
        assert_eq!(eval(source), Ok(Value::string(expected)), "{source}");
    }
    for (source, count) in [
        ("''.split(/a/).length", 1.0),
        ("''.split(/(?:)/).length", 0.0),
        ("'😀'.split(/(?:)/).length", 2.0),
    ] {
        assert_eq!(eval(source), Ok(Value::Number(count)), "{source}");
    }
}

#[test]
fn regex_split_observes_flags_limit_order_and_resource_failures() -> Result<(), Error> {
    assert_eq!(
        eval(
            "let log='';let r=/,/;Object.defineProperty(r,'flags',{get(){log+='f';return ''}});'a,b'.split(r,{valueOf(){log+='l';return 2}});log"
        ),
        Ok(Value::string("fl"))
    );
    assert_eq!(
        eval(
            "let r=/,/;Object.defineProperty(r,'flags',{get(){throw 9}});try{'a,b'.split(r,0)}catch(e){e}"
        ),
        Ok(Value::Number(9.0))
    );
    assert_eq!(
        eval(
            "let r=/,/;Object.defineProperty(r,'flags',{value:'\\ud800'});try{'a,b'.split(r)}catch(e){e.name}"
        ),
        Ok(Value::string("SyntaxError"))
    );
    let limits = Limits {
        fuel: 150,
        ..Limits::default()
    };
    let source = "'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!'.split(/(a+)+$/)";
    assert!(matches!(
        Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost),
        Err(Error::Limit { .. })
    ));
    Ok(())
}
