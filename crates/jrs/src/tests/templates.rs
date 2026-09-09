// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::eval;
use crate::{Error, Limits, Value, compile};

#[test]
fn untagged_templates_preserve_text_escapes_and_nested_substitutions() {
    for (source, expected) in [
        ("``", ""),
        ("`hello`", "hello"),
        ("`a${40+2}b`", "a42b"),
        ("`${true}${null}${undefined}`", "truenullundefined"),
        ("`a\r\nb\rc\nd`", "a\nb\nc\nd"),
        ("`\\`\\${x}\\n\\u{1f600}`", "`${x}\n😀"),
        ("`a${`b${42}c`}d`", "ab42cd"),
        ("`x${({a:42}).a}y`", "x42y"),
        ("`x${(()=>{return 42;})()}y`", "x42y"),
        ("`${/}/.test('}')} ${8/2}`", "true 4"),
        ("`a${1 /* } */}b${2 // }\n}c`", "a1b2c"),
        ("`a${'{}'}b`", "a{}b"),
        ("`${[1,2]}`", "1,2"),
        (
            "`${{toString(){return 'string';},valueOf(){return 'number';}}}`",
            "string",
        ),
        ("`use strict`;function f(a,a){return a;}`${f(1,2)}`", "2"),
    ] {
        assert_eq!(eval(source), Ok(Value::string(expected)), "{source}");
    }
    assert_eq!(eval("`\\ud800`"), Ok(Value::String(vec![0xd800].into())));
}

#[test]
fn template_substitution_converts_before_next_expression() {
    assert_eq!(
        eval("let order='';let o={toString(){order+='a';return 'x';}};`${o}${order+='b'}`;order"),
        Ok(Value::string("ab"))
    );
    assert_eq!(
        eval("let n=0;try{`${{toString(){throw 42;}}}${n++}`;}catch(e){n+e;}"),
        Ok(Value::Number(42.0))
    );
    assert_eq!(eval("let String=()=>99;`${42}`"), Ok(Value::string("42")));
}

#[test]
fn invalid_templates_are_early_errors_and_nesting_is_bounded() {
    for source in [
        "`", "`${", "`${1", "`${}`", "`\\1`", "`\\8`", "`\\u{}`", "`\\xGG`", "`${1;}`", "tag`x`",
    ] {
        assert!(compile(source, Limits::default()).is_err(), "{source}");
    }
    let source = format!("{}1{}", "`${".repeat(1000), "}`".repeat(1000));
    assert!(matches!(
        compile(&source, Limits::default()),
        Err(Error::Limit { .. })
    ));
    assert!(matches!(
        compile(
            "`abc`",
            Limits {
                string_units: 2,
                ..Limits::default()
            }
        ),
        Err(Error::Limit { .. })
    ));
}
