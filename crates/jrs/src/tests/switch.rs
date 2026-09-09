// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::eval;
use crate::{Error, Limits, Value, compile};

#[test]
fn switch_matching_default_fallthrough_and_selector_order() {
    for (source, expected) in [
        ("switch(1){}", Value::Undefined),
        (
            "switch(2){case 1:1;break;case 2:42;break;default:3;}",
            Value::Number(42.0),
        ),
        ("switch(9){case 1:1;break;default:42;}", Value::Number(42.0)),
        (
            "switch(1){case '1':1;break;default:42;}",
            Value::Number(42.0),
        ),
        (
            "switch(NaN){case NaN:1;break;default:42;}",
            Value::Number(42.0),
        ),
        (
            "let n=0;switch(1){case 1:n++;default:n+=10;case 2:n+=31;}n",
            Value::Number(42.0),
        ),
        (
            "let n=0;switch(2){case 1:n=1;break;default:n=2;case 2:n=42;}n",
            Value::Number(42.0),
        ),
        (
            "let n=0;switch(9){default:n=40;case 2:n+=2;}n",
            Value::Number(42.0),
        ),
        (
            "let s='';function sel(c,n){s+=c;return n;}switch(2){case sel('a',1):break;case sel('b',2):break;case sel('c',3):break;}s",
            Value::string("ab"),
        ),
        (
            "let o={};switch(o){case o:42;break;default:0;}",
            Value::Number(42.0),
        ),
        (
            "switch(1){case 1:let x=42;x;break;case 2:x;}",
            Value::Number(42.0),
        ),
        ("switch(1){case 1:var x=42;break;}x", Value::Number(42.0)),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
}

#[test]
fn switch_loop_control_and_exception_unwinding() {
    for (source, expected) in [
        (
            "let n=0;for(let i=0;i<4;i++){switch(i){case 1:continue;case 2:break;default:n+=i;}n+=10;}n",
            33.0,
        ),
        (
            "function f(){switch(1){case 1:try{return 1;}finally{return 42;}}}f()",
            42.0,
        ),
        (
            "let n=0;switch(1){case 1:try{break;}finally{n=42;}case 2:n=0;}n",
            42.0,
        ),
        (
            "switch(1){case 1:switch(2){case 2:42;break;}break;default:0;}",
            42.0,
        ),
    ] {
        assert_eq!(eval(source), Ok(Value::Number(expected)), "{source}");
    }
    assert!(matches!(
        eval("switch(1){case 1:x;break;case 2:let x=2;}"),
        Err(Error::Reference { .. })
    ));
    for source in [
        "switch(1){default:1;default:2;}",
        "switch(1){case 1:let x;case 2:let x;}",
        "switch(1){case 1:let x;case 2:var x;}",
        "switch(1){case 1:continue;}",
        "switch(1){case 1:function f(){break;}}",
    ] {
        assert!(compile(source, Limits::default()).is_err(), "{source}");
    }
}
