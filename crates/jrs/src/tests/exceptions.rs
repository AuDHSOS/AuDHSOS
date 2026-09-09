// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::eval;
use crate::{Error, Limits, Runtime, SilentHost, Value, compile};

#[test]
fn thrown_values_and_runtime_errors_reach_catch() {
    for (source, expected) in [
        ("try{throw 42;}catch(e){e}", Value::Number(42.0)),
        (
            "let x={};try{throw x;}catch(e){e===x}",
            Value::Boolean(true),
        ),
        (
            "function f(){throw 42;}try{f();}catch(e){e}",
            Value::Number(42.0),
        ),
        ("try{null.x;}catch(e){e.name}", Value::string("TypeError")),
        (
            "try{absent;}catch(e){e.name}",
            Value::string("ReferenceError"),
        ),
        ("try{throw 1;}catch{42}", Value::Number(42.0)),
        (
            "try{throw 1;}catch(e){try{throw e+1;}catch(x){x+40;}}",
            Value::Number(42.0),
        ),
        (
            "try{try{throw 1;}catch(e){throw e+41;}}catch(e){e}",
            Value::Number(42.0),
        ),
        ("let e=42;try{throw 1;}catch(e){e=2;}e", Value::Number(42.0)),
        ("try{var x=42;}catch(e){}x", Value::Number(42.0)),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
    assert_eq!(
        eval("throw 42"),
        Err(Error::Thrown {
            value: Value::Number(42.0)
        })
    );
    assert_eq!(
        eval("try{1;}finally{throw 42;}"),
        Err(Error::Thrown {
            value: Value::Number(42.0)
        })
    );
}

#[test]
fn finally_runs_on_all_abrupt_completions_and_can_override_them() {
    for (source, expected) in [
        ("function f(){try{return 1;}finally{return 42;}}f()", 42.0),
        ("function f(){try{return 42;}finally{1;}}f()", 42.0),
        ("function f(){try{throw 1;}finally{return 42;}}f()", 42.0),
        (
            "function f(){try{return 1;}finally{throw 42;}}try{f();}catch(e){e}",
            42.0,
        ),
        ("let n=0;while(true){try{break;}finally{n=42;}}n", 42.0),
        (
            "let n=0;for(let i=0;i<3;i++){try{continue;}finally{n+=14;}}n",
            42.0,
        ),
        (
            "let n=0;try{while(n<2){n++;continue;}}finally{n+=40;}n",
            42.0,
        ),
        (
            "let n=0;function f(){try{try{return 1;}finally{n+=2;}}finally{n+=40;}}f();n",
            42.0,
        ),
        ("try{try{throw 1;}finally{2;}}catch(e){e+41;}", 42.0),
        ("try{42;}finally{1;}", 42.0),
        ("try{throw 1;}catch(e){42;}finally{2;}", 42.0),
        (
            "function f(){try{return 42;}finally{for(let i=0;i<2;i++){continue;}}}f()",
            42.0,
        ),
    ] {
        assert_eq!(eval(source), Ok(Value::Number(expected)), "{source}");
    }
}

#[test]
fn exception_early_errors_and_resource_failure_are_not_swallowed() -> Result<(), Error> {
    for source in [
        "throw\n1",
        "try{}",
        "try{}catch(e){let e;}",
        "try{}catch(){ }",
        "try 1;catch(e){}",
        "try{}finally 1;",
    ] {
        assert!(compile(source, Limits::default()).is_err(), "{source}");
    }
    let limits = Limits {
        fuel: 100,
        ..Limits::default()
    };
    let program = compile("try{while(true){}}catch(e){42;}", limits)?;
    assert!(matches!(
        Runtime::new(limits).run(&program, &mut SilentHost),
        Err(Error::Limit { .. })
    ));
    Ok(())
}

#[test]
fn pending_return_and_throw_values_survive_gc_in_finally() -> Result<(), Error> {
    let limits = Limits {
        // Allow the added toString intrinsic while retaining forced collection.
        // Account for the two GC-owned global numeric parser functions.
        heap_entries: 29,
        ..Limits::default()
    };
    for source in [
        "function f(){try{return {x:42};}finally{for(let i=0;i<100;i++){let o={};o.o=o;}}}f().x",
        "try{try{throw {x:42};}finally{for(let i=0;i<100;i++){let o={};o.o=o;}}}catch(e){e.x}",
    ] {
        assert_eq!(
            Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
            Value::Number(42.0)
        );
    }
    Ok(())
}
