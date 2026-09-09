// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::eval;
use crate::{Error, Limits, Runtime, SilentHost, Value, compile};

#[test]
fn ordinary_functions_calls_return_and_hoisting() {
    for (source, expected) in [
        (
            "function add(a,b){return a+b;} add(20,22)",
            Value::Number(42.0),
        ),
        ("let x=f(); function f(){return 42;} x", Value::Number(42.0)),
        ("(function(x){return x+1;})(41)", Value::Number(42.0)),
        ("function f(a){return a;} f()", Value::Undefined),
        ("function f(){42;} f()", Value::Undefined),
        ("function f(){return;} f()", Value::Undefined),
        ("function f(){return\n42;} f()", Value::Undefined),
        ("function f(a,a){return a;} f(1,2)", Value::Number(2.0)),
        (
            "function f(){if(true){return 42;}return 0;} f()",
            Value::Number(42.0),
        ),
        ("function f(){for(;;){return 42;}} f()", Value::Number(42.0)),
        (
            "function f(x){if(x<2)return 1;return x*f(x-1);} f(6)",
            Value::Number(720.0),
        ),
        (
            "let f=function fact(x){return x<2?1:x*fact(x-1);}; let g=f; f=0; g(5)",
            Value::Number(120.0),
        ),
        (
            "let f=function inner(){return typeof inner;}; f()",
            Value::string("function"),
        ),
        (
            "let f=function inner(){}; typeof inner",
            Value::string("undefined"),
        ),
        ("let n=Number; n('42')", Value::Number(42.0)),
        ("typeof Number", Value::string("function")),
        (
            "let Number=function(){return 42;}; Number()",
            Value::Number(42.0),
        ),
        ("function f(){return 7;} f === f", Value::Boolean(true)),
        ("(function(){}) === (function(){})", Value::Boolean(false)),
        ("Boolean(function(){})", Value::Boolean(true)),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
}

#[test]
fn closures_share_bindings_and_outlive_calls() {
    for (source, expected) in [
        (
            "function counter(){let n=0;return function(){return ++n;};}let c=counter();c();c()",
            2.0,
        ),
        (
            "function counter(){let n=0;return ()=>++n;}let a=counter();let b=counter();a();a();b()",
            1.0,
        ),
        ("let x=1;let f=()=>x; x=42;f()", 42.0),
        (
            "function a(x){return function(){return ()=>x;};}a(42)()()",
            42.0,
        ),
        ("let f; {let x=42;f=()=>x;}f()", 42.0),
        (
            "function f(x){return ()=>x;}let a=f(1);let b=f(2);a()+b()",
            3.0,
        ),
        (
            "let a;let b;for(let i=0;i<2;i++){if(i==0)a=()=>i;else b=()=>i;}a()*10+b()",
            1.0,
        ),
        (
            "let a;let b;let i=0;while(i<2){let x=i++;if(x==0)a=()=>x;else b=()=>x;}a()*10+b()",
            1.0,
        ),
        (
            "let a;let b;for(let i=0;i<2;i++){if(i==0){a=()=>i;continue;}b=()=>i;}a()+b()",
            1.0,
        ),
        ("function f(){function g(){return 42;}return g;}f()()", 42.0),
        ("let f=(a,b)=>a+b;f(20,22)", 42.0),
        ("let f=a=>b=>a+b;f(20)(22)", 42.0),
        ("let f=()=>{return 42;}; f()", 42.0),
        ("let f=()=>42; let g=(f);g()", 42.0),
    ] {
        assert_eq!(eval(source), Ok(Value::Number(expected)), "{source}");
    }
}

#[test]
fn function_early_errors_and_runtime_errors_are_distinct() {
    for source in [
        "function (){}",
        "function f(1){}",
        "function f(a){let a;}",
        "(a,a)=>a",
        "x\n=>x",
        "()\n=>1",
        "while(true){function f(){break;}}",
        "function f(){continue;}",
        "()=>{return 1",
        "function f()",
        "return 1",
        "if(true) function f(){}",
    ] {
        assert!(compile(source, Limits::default()).is_err(), "{source}");
    }
    for source in ["1()", "let print=1;print()", "(2)(3)"] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
    for source in [
        "foo()",
        "let f=()=>x;f();let x=1",
        "function f(){return x;}f();let x=1",
    ] {
        assert!(
            matches!(eval(source), Err(Error::Reference { .. })),
            "{source}"
        );
    }
}

#[test]
fn tracing_collects_cycles_and_keeps_live_closures() -> Result<(), Error> {
    let limits = Limits {
        // The global object retains parseInt/parseFloat Function objects too.
        heap_entries: 36,
        fuel: 1_000_000,
        ..Limits::default()
    };
    let source = "let keep;for(let i=0;i<1000;i++){let f=()=>f;if(i==3)keep=()=>i;}keep()";
    assert_eq!(
        Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
        Value::Number(3.0)
    );
    let source = "function make(x){let f=()=>f;return ()=>x;}let keep=make(42);for(let i=0;i<1000;i++)make(i);keep()";
    assert_eq!(
        Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
        Value::Number(42.0)
    );
    Ok(())
}

#[test]
fn calls_share_fuel_and_frames_are_bounded() -> Result<(), Error> {
    let limits = Limits {
        call_frames: 8,
        ..Limits::default()
    };
    let recursive = compile("function f(){return f();} f()", limits)?;
    let mut runtime = Runtime::new(limits);
    assert!(matches!(
        runtime.run(&recursive, &mut SilentHost),
        Err(Error::Limit {
            resource: "call frames"
        })
    ));
    let fuel = Limits {
        fuel: 100,
        ..limits
    };
    assert!(matches!(
        Runtime::new(fuel).run(
            &compile("function f(){return 1;}while(true)f()", fuel)?,
            &mut SilentHost
        ),
        Err(Error::Limit {
            resource: "execution fuel"
        })
    ));
    assert_eq!(
        runtime.run(&compile("42", limits)?, &mut SilentHost)?,
        Value::Number(42.0)
    );
    let heap = Limits {
        heap_entries: 1,
        ..limits
    };
    assert!(matches!(
        Runtime::new(heap).run(&compile("let a=1;let b=()=>a", heap)?, &mut SilentHost),
        Err(Error::Limit {
            resource: "heap entries"
        })
    ));
    Ok(())
}

#[test]
fn var_bindings_are_function_scoped_and_hoisted() {
    for (source, expected) in [
        ("x; var x", Value::Undefined),
        ("var x=42;var x;x", Value::Number(42.0)),
        ("if(false){var x=3;} x", Value::Undefined),
        ("function f(x){var x;return x;}f(42)", Value::Number(42.0)),
        (
            "function f(){if(true){var x=42;}return x;}f()",
            Value::Number(42.0),
        ),
        (
            "let f;for(var i=0;i<3;i++){f=()=>i;}f()",
            Value::Number(3.0),
        ),
        (
            "function f(){return 1;}function f(){return 42;}f()",
            Value::Number(42.0),
        ),
        (
            "function f(x){function x(){return 42;}return x();}f(0)",
            Value::Number(42.0),
        ),
        ("var x=1;{let x=2;}x", Value::Number(1.0)),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
    for source in [
        "let x;var x",
        "var x;const x=1",
        "let x;{var x;}",
        "{let x;{var x;}}",
        "function f(x){let x;}",
        "let f;function f(){}",
    ] {
        assert!(compile(source, Limits::default()).is_err(), "{source}");
    }
}

#[test]
fn strict_directives_validate_parameters_without_accepting_escaped_directives() {
    for source in [
        "function f(a,a){'use strict';}",
        "'use strict';function f(a,a){}",
        "'other';'use strict';function f(a,a){}",
        "function f(eval){'use strict';}",
        "function f(arguments){'use strict';}",
        "function f(){'use strict';function g(a,a){}}",
    ] {
        assert!(compile(source, Limits::default()).is_err(), "{source}");
    }
    for source in [
        "'use\\x20strict';function f(a,a){return a;}f(1,2)",
        "'use strict'+'';function f(a,a){return a;}f(1,2)",
        "('use strict');function f(a,a){return a;}f(1,2)",
    ] {
        assert_eq!(eval(source), Ok(Value::Number(2.0)), "{source}");
    }
}

#[test]
fn strict_assignments_and_loop_var_conflicts_are_early_errors() {
    for source in [
        "'use strict'; eval=1",
        "'use strict'; ++arguments",
        "'use strict'; eval++",
        "function eval(){'use strict';}",
        "for(let x=0;x<1;x++){var x;}",
    ] {
        assert!(
            matches!(
                compile(source, Limits::default()),
                Err(Error::Syntax { .. })
            ),
            "{source}"
        );
    }
}

#[test]
fn captured_writes_and_collected_frame_parameters_keep_identity() -> Result<(), Error> {
    for (source, expected) in [
        ("let x=1;let f=()=>x=42;f();x", Value::Number(42.0)),
        ("let x=1;let f=()=>x+=41;f();x", Value::Number(42.0)),
        ("let f=function(){return 1;}; f==f", Value::Boolean(true)),
        ("Number===Number", Value::Boolean(true)),
        ("Number===Boolean", Value::Boolean(false)),
        ("Number==(()=>0)", Value::Boolean(false)),
        ("let f=()=>1;f==Number", Value::Boolean(false)),
        (
            "function f(a,a){return ()=>a;}f(1,42)()",
            Value::Number(42.0),
        ),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
    assert!(matches!(
        eval("const x=1;(()=>x=2)()"),
        Err(Error::Type { .. })
    ));
    assert!(matches!(
        eval("let f=()=>x=2;f();let x;"),
        Err(Error::Reference { .. })
    ));
    let limits = Limits {
        // Intrinsic method identities now include the heap-owned toString method.
        heap_entries: 32,
        ..Limits::default()
    };
    let program = compile(
        "function f(x){return ()=>x;}let keep=f(42);for(let i=0;i<200;i++)f(i);keep()",
        limits,
    )?;
    assert_eq!(
        Runtime::new(limits).run(&program, &mut SilentHost)?,
        Value::Number(42.0)
    );
    let functions = compile("()=>42", limits)?;
    let mut runtime = Runtime::new(limits);
    let first = runtime.run(&functions, &mut SilentHost)?;
    let second = runtime.run(&functions, &mut SilentHost)?;
    assert_ne!(first, second);
    assert!(!first.to_string().is_empty());
    Ok(())
}

#[test]
fn total_frame_slots_are_bounded_before_allocation() -> Result<(), Error> {
    let limits = Limits {
        binding_slots: 3,
        ..Limits::default()
    };
    let program = compile("function f(a,b){return f(a,b);}f(1,2)", limits)?;
    assert!(matches!(
        Runtime::new(limits).run(&program, &mut SilentHost),
        Err(Error::Limit {
            resource: "binding slots"
        })
    ));
    let program = compile("let a;let b;let c;let d;", limits)?;
    assert!(matches!(
        Runtime::new(limits).run(&program, &mut SilentHost),
        Err(Error::Limit {
            resource: "binding slots"
        })
    ));
    Ok(())
}

#[test]
fn default_and_rest_parameters_have_tdz_and_separate_body_environment() {
    for (source, expected) in [
        ("function f(a=40,b=a+2){return b;}f()", Value::Number(42.0)),
        (
            "function f(a=42){return a;}f(undefined)",
            Value::Number(42.0),
        ),
        ("function f(a=42){return a;}f(null)", Value::Null),
        (
            "function f(a={}){return a;}f()===f()",
            Value::Boolean(false),
        ),
        (
            "function f(a=42,g=()=>a){var a=1;return g();}f()",
            Value::Number(42.0),
        ),
        (
            "let x=42;function f(g=()=>x){var x=1;return g();}f()",
            Value::Number(42.0),
        ),
        ("function f(a=42){var a;return a;}f()", Value::Number(42.0)),
        (
            "function f(a=42,g=()=>a){function a(){return 1;}return g();}f()",
            Value::Number(42.0),
        ),
        (
            "function f(a,...rest){return rest.join();}f(0,1,2,3)",
            Value::string("1,2,3"),
        ),
        (
            "function f(...rest){return rest.length;}f()",
            Value::Number(0.0),
        ),
        ("((a=40,b=a+2)=>b)()", Value::Number(42.0)),
        ("((...args)=>args.join())(1,2)", Value::string("1,2")),
        ("function f(a,b=2,c){return 1;}f.length", Value::Number(1.0)),
        ("function f(a,...r){}f.length", Value::Number(1.0)),
        (
            "let n=0;function f(a=++n){return a;}f(1);f();n",
            Value::Number(1.0),
        ),
        (
            "let o={x:42,f(a=this.x){return a;}};o.f()",
            Value::Number(42.0),
        ),
        ("function C(x=42){this.x=x;}new C().x", Value::Number(42.0)),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
    for source in [
        "function f(a=a){}f()",
        "function f(a=b,b=2){}f()",
        "function f(a=missing){var missing=1;}f()",
    ] {
        assert!(
            matches!(eval(source), Err(Error::Reference { .. })),
            "{source}"
        );
    }
    for source in [
        "function f(a=1,a){}",
        "function f(a=1){'use strict';}",
        "function f(...r=[]){}",
        "function f(...r,){}",
        "function f(...r,a){}",
        "(a=1)=>{'use strict';}",
        "function f(a=1){let a;}",
    ] {
        assert!(compile(source, Limits::default()).is_err(), "{source}");
    }
}

#[test]
fn parameter_objects_and_rest_arguments_survive_gc_before_body() -> Result<(), Error> {
    let limits = Limits {
        // Include the newly GC-owned Array methods in the stress-test budget.
        heap_entries: 72,
        ..Limits::default()
    };
    for source in [
        "function f(a={x:42},b=(()=>{for(let i=0;i<100;i++){let o={};o.o=o;}return 1;})()){return a.x;}f()",
        "function f(a=()=>b,b=42){return a;}f()()",
        "function f(a,...rest){for(let i=0;i<100;i++){let o={};o.o=o;}return rest[0].x;}f(0,{x:42})",
        "function f(a=(()=>{throw 42;})()){return 0;}try{f();}catch(e){e}",
    ] {
        assert_eq!(
            Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
            Value::Number(42.0),
            "{source}"
        );
    }
    Ok(())
}
