// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::eval;
use crate::{Error, Host, Limits, Runtime, SilentHost, Value, compile};

#[test]
fn getters_setters_literals_and_descriptors_use_original_receiver() {
    for (source, expected) in [
        ("let o={get x(){return 42;}};o.x", Value::Number(42.0)),
        (
            "let o={n:1,get x(){return this.n;},set x(v){this.n=v;}};o.x=42;o.x",
            Value::Number(42.0),
        ),
        (
            "let n=0;let a=[];Object.defineProperty(a,0,{get:function(){n++;return 7;}});a.length+n",
            Value::Number(1.0),
        ),
        (
            "let n=0;let a=[];Object.defineProperty(a,0,{get:function(){n++;return 7;}});a[0]+n",
            Value::Number(8.0),
        ),
        (
            "let p={get x(){return this.n;},set x(v){this.n=v;}};let o=Object.create(p);o.x=42;o.x",
            Value::Number(42.0),
        ),
        (
            "let o={get x(){return 42;}};Object.freeze(o);o.x",
            Value::Number(42.0),
        ),
        (
            "let o={n:0,set x(v){this.n=v;}};o.x=42;o.n",
            Value::Number(42.0),
        ),
        ("({set x(v){}}).x", Value::Undefined),
        ("let o={get x(){return 1;}};o.x=2;o.x", Value::Number(1.0)),
        (
            "Object.getOwnPropertyDescriptor({get x(){return 1;}},'x').get()",
            Value::Number(1.0),
        ),
        (
            "'value' in Object.getOwnPropertyDescriptor({get x(){}},'x')",
            Value::Boolean(false),
        ),
        (
            "let o={get x(){return 1;}};Object.defineProperty(o,'x',{value:42});o.x",
            Value::Number(42.0),
        ),
        (
            "let o={x:1};Object.defineProperty(o,'x',{get(){return 42;}});o.x",
            Value::Number(42.0),
        ),
        ("let o={get ['x'](){return 42;}};o.x", Value::Number(42.0)),
        ("let o={get(){return 42;}};o.get()", Value::Number(42.0)),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
}

#[test]
fn getter_exceptions_and_finally_cross_native_boundary_once() {
    for source in [
        "let o={get x(){throw 42;}};try{o.x;}catch(e){e}",
        "let o={get x(){try{return 1;}finally{throw 42;}}};try{o.x;}catch(e){e}",
        "let o={set x(v){throw v;}};try{o.x=42;}catch(e){e}",
        "let o={get x(){return [1].map(()=>42)[0];}};o.x",
        "let o={get x(){try{throw 42;}catch(e){return e;}}};o.x",
        "function f(){try{let o={get x(){throw 1;}};return o.x;}finally{return 42;}}f()",
    ] {
        assert_eq!(eval(source), Ok(Value::Number(42.0)), "{source}");
    }
}

#[test]
fn descriptor_getter_order_and_validation() {
    assert_eq!(
        eval(
            "let order='';let d={get enumerable(){order+='e';return true;},get configurable(){order+='c';return true;},get value(){order+='v';return 42;},get writable(){order+='w';return true;}};let o={};Object.defineProperty(o,'x',d);order+o.x"
        ),
        Ok(Value::string("ecvw42"))
    );
    for source in ["({get x(v){}})", "({set x(){}})", "({set x(a,b){}})"] {
        assert!(compile(source, Limits::default()).is_err(), "{source}");
    }
    for source in [
        "Object.defineProperty({},'x',{get:1})",
        "Object.defineProperty({},'x',{get(){},value:1})",
        "Object.defineProperty([], 'length',{get(){return 1;}})",
        "'use strict';let o={get x(){return 1;}};o.x=2",
        "let o={};Object.defineProperty(o,'x',{get(){return 1;}});Object.defineProperty(o,'x',{get(){return 2;}})",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn getter_host_calls_reach_real_host_and_failures_propagate() -> Result<(), Error> {
    struct Recorder(Vec<Value>);
    struct Failure;
    impl Host for Failure {
        fn print(&mut self, _: &[Value]) -> Result<(), Error> {
            Err(Error::Host)
        }
    }
    impl Host for Recorder {
        fn print(&mut self, args: &[Value]) -> Result<(), Error> {
            self.0.extend_from_slice(args);
            Ok(())
        }
    }
    let limits = Limits::default();
    let program = compile("({get x(){print(42);return 1;}}).x", limits)?;
    let mut host = Recorder(Vec::new());
    assert_eq!(
        Runtime::new(limits).run(&program, &mut host)?,
        Value::Number(1.0)
    );
    assert_eq!(host.0, vec![Value::Number(42.0)]);
    let program = compile("try{({get x(){print(1);}}).x;}catch(e){42;}", limits)?;
    assert_eq!(
        Runtime::new(limits).run(&program, &mut Failure),
        Err(Error::Host)
    );
    Ok(())
}

#[test]
fn recursive_getters_and_pending_values_are_bounded_and_rooted() -> Result<(), Error> {
    let limits = Limits {
        heap_entries: 48,
        ..Limits::default()
    };
    let program = compile("let o={get x(){return this.x;}};o.x", limits)?;
    assert!(matches!(
        Runtime::new(limits).run(&program, &mut SilentHost),
        Err(Error::Limit {
            resource: "native reentry"
        })
    ));
    let program = compile(
        "let o={get x(){for(let i=0;i<100;i++){let a={};a.a=a;}return this.n;},n:42};o.x",
        limits,
    )?;
    assert_eq!(
        Runtime::new(limits).run(&program, &mut SilentHost)?,
        Value::Number(42.0)
    );
    Ok(())
}

#[test]
fn ordinary_coercion_order_and_primitive_results() {
    for (source, expected) in [
        ("let o={valueOf(){return 42;}};+o", Value::Number(42.0)),
        ("let o={valueOf(){return 40;}};o+2", Value::Number(42.0)),
        (
            "let o={valueOf(){return {};},toString(){return '42';}};Number(o)",
            Value::Number(42.0),
        ),
        (
            "String({toString(){return 'yes';},valueOf(){throw 1;}})",
            Value::string("yes"),
        ),
        (
            "let o={};let k={toString(){return 'x';}};o[k]=42;o.x",
            Value::Number(42.0),
        ),
        ("({})[{}]", Value::Undefined),
        ("({valueOf(){return 42;}})==42", Value::Boolean(true)),
        ("({valueOf(){return 1;}})==true", Value::Boolean(true)),
        ("({valueOf(){throw 1;}})===42", Value::Boolean(false)),
        ("({valueOf(){throw 1;}})==null", Value::Boolean(false)),
        ("'x'+{toString(){return 'y';}}", Value::string("xy")),
        ("[1,[2,3],null].join('-')", Value::string("1-2,3-")),
        ("String([1,2,3])", Value::string("1,2,3")),
        (
            "let n=0;let a=new Array;Object.defineProperty(a,0,{get:function(){n++;return 7;}});let length=a.length;let read=a[0];let text=String(a);length+':'+read+':'+text+':'+n",
            Value::string("1:7:7:2"),
        ),
        (
            "let a=[];let n=0;a.length={valueOf(){n++;return 3;}};n+':'+a.length",
            Value::string("2:3"),
        ),
        (
            "let a=[];let n=0;Object.defineProperty(a,'length',{value:{valueOf(){n++;return 3;}}});n+':'+a.length",
            Value::string("2:3"),
        ),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
    assert_eq!(
        eval(
            "let s='';let a={valueOf(){s+='a';return 1;}};let b={valueOf(){s+='b';return 2;}};a>b;s"
        ),
        Ok(Value::string("ab"))
    );
    assert_eq!(
        eval("let s='';let o={get valueOf(){s+='g';return function(){s+='c';return 42;};}};+o;s"),
        Ok(Value::string("gc"))
    );
    for source in [
        "Number({valueOf(){return {};},toString(){return {};}})",
        "String(Object.create(null))",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn accessor_descriptor_conversion_preserves_flags_and_frozen_setters() {
    for (source, expected) in [
        (
            "let n=0;let o={set x(v){n=v;}};Object.freeze(o);o.x=42;n",
            Value::Number(42.0),
        ),
        (
            "let f=()=>42;let o={};Object.defineProperty(o,'x',{get:f});Object.defineProperty(o,'x',{get:f});o.x",
            Value::Number(42.0),
        ),
        (
            "let o={get x(){return 1;},set x(v){}};Object.defineProperty(o,'x',{enumerable:false});typeof Object.getOwnPropertyDescriptor(o,'x').set",
            Value::string("function"),
        ),
        (
            "let o={get x(){return 1;}};Object.defineProperty(o,'x',{value:42});Object.getOwnPropertyDescriptor(o,'x').writable",
            Value::Boolean(false),
        ),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
    for source in [
        "let o={};Object.defineProperty(o,'x',{get(){return 1;}});Object.defineProperty(o,'x',{value:2})",
        "Object.defineProperty({},'x',{set:1})",
        "Object.defineProperty({},'x',{set(){},writable:true})",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn native_intermediate_values_survive_getter_gc_and_mutation() -> Result<(), Error> {
    let limits = Limits {
        // Array constructor storage and mutable intrinsic methods are roots;
        // the transient allocations below still force repeated collection.
        heap_entries: 64,
        ..Limits::default()
    };
    for source in [
        "let a=[{x:42},2];Object.defineProperty(a,1,{get(){delete a[0];for(let i=0;i<100;i++){let o={};o.o=o;}return 1;}});a.slice()[0].x",
        "let o={0:{x:42},get length(){return 1;},set length(v){for(let i=0;i<100;i++){let o={};o.o=o;}}};Array.prototype.pop.call(o).x",
        "let o={};let d={get value(){return {x:42};},get writable(){for(let i=0;i<100;i++){let o={};o.o=o;}return true;}};Object.defineProperty(o,'x',d);o.x.x",
        "let o={get x(){return {valueOf(){for(let i=0;i<100;i++){let o={};o.o=o;}return 41;}}},set x(v){this.n=v;}};++o.x;o.n",
    ] {
        assert_eq!(
            Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
            Value::Number(42.0),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn math_extrema_convert_all_arguments_left_to_right_and_preserve_zero_signs() {
    for name in ["max", "min"] {
        for prefix in ["NaN", "Infinity", "-Infinity"] {
            let source = format!(
                "let error={{}};try{{Math.{name}({prefix},{{valueOf(){{throw error;}}}});}}catch(e){{e===error}}"
            );
            assert_eq!(eval(&source), Ok(Value::Boolean(true)), "{source}");
        }
        let source = format!(
            "let order='';Math.{name}({{valueOf(){{order+='a';return NaN;}}}},{{valueOf(){{order+='b';return 1;}}}});order"
        );
        assert_eq!(eval(&source), Ok(Value::string("ab")));
    }
    for (source, expected) in [
        ("Object.is(Math.max(0,-0),0)", true),
        ("Object.is(Math.min(0,-0),-0)", true),
        ("Object.is(Math.max(-0,-0),-0)", true),
        ("Object.is(Math.min(0,0),0)", true),
        ("Math.max()===-Infinity", true),
        ("Math.min()===Infinity", true),
        ("isNaN(Math.max(NaN,1))", true),
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(expected)), "{source}");
    }
    assert_eq!(eval("Math.max('1',42,-3)"), Ok(Value::Number(42.0)));
}
