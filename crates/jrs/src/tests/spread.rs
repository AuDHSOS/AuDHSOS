// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::{Error, Limits, Runtime, SilentHost, Value, compile, eval};

#[test]
fn spread_calls_preserve_values_receivers_and_mixed_argument_order() {
    for (source, expected) in [
        (
            "function f(...a){return a.join('|')}f(0,...[1,2],3,...[],...[4],5)",
            Value::string("0|1|2|3|4|5"),
        ),
        (
            "function f(...a){return a.join('|')}f(...'😀a')",
            Value::string("😀|a"),
        ),
        (
            "function f(){return arguments.length}f(...[,,])",
            Value::Number(2.0),
        ),
        (
            "let o={n:7,m(...a){return this.n+a[0]+a[1]}};o.m(...[1,2])",
            Value::Number(10.0),
        ),
        (
            "let o={n:7,m(...a){return this.n+a[0]}};(o.m)(...[1])",
            Value::Number(8.0),
        ),
        (
            "function f(a,b){return this.n+a+b}f.call(...[{n:7},1,2])",
            Value::Number(10.0),
        ),
        (
            "function f(...a){return a.join()}f.bind(null,1)(...[2,3])",
            Value::string("1,2,3"),
        ),
        (
            "function f(...a){return a.join()}f(...[f(...[1,2]),f(3,...[4])])",
            Value::string("1,2,3,4"),
        ),
        ("Math.max(...[1,7,3])", Value::Number(7.0)),
        (
            "function f(){return [...arguments].join()}f(...[1,2])",
            Value::string("1,2"),
        ),
        (
            "function f(...a){return a.length}f(...[],)",
            Value::Number(0.0),
        ),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
}

#[test]
fn array_spread_preserves_holes_but_materializes_iterated_holes() {
    for (source, expected) in [
        ("[0,...[1,2],3,...[],4].join()", "0,1,2,3,4"),
        (
            "let a=[,...[,,],,5,,];a.length+'|'+Object.keys(a).join()",
            "6|1,2,4",
        ),
        ("[...'😀a'].join('|')", "😀|a"),
        (
            "let a=[1,2];a[Symbol.iterator]=function(){let n=0;return {next(){return n++?{done:true}:{value:7}}}};[...a].join()",
            "7",
        ),
        (
            "let a=[1];a[Symbol.isConcatSpreadable]=false;[...a].join()",
            "1",
        ),
        (
            "Array.prototype.push=function(){throw 9};Array.prototype[0]=99;[...[1,2]].join()",
            "1,2",
        ),
        ("let a=[...[[]],[],...[['x']]];a.length+'|'+a[2][0]", "3|x"),
    ] {
        assert_eq!(eval(source), Ok(Value::string(expected)), "{source}");
    }
    assert_eq!(
        eval("[...'😀a\\ud800'][2]"),
        Ok(Value::String(alloc::rc::Rc::from([0xd800])))
    );
    assert_eq!(
        eval(
            "Object.defineProperty(Array.prototype,0,{set(){throw 7},configurable:true});[...[3]][0]"
        ),
        Ok(Value::Number(3.0))
    );
}

#[test]
fn iterator_evaluation_completes_before_next_argument_or_element() {
    let setup = "let log='';let iterable={get [Symbol.iterator](){log+='i';return function(){log+='c';let n=0;return {get next(){log+='n';return function(){return {get done(){log+='d';return n===2},get value(){log+='v';return ++n}}}}}}}};function mark(){log+='e';return 3}function f(...a){log+='f';return a.join()}";
    for (body, expected) in [
        ("f(...iterable,mark());log", "icndvdvdef"),
        ("[...iterable,mark()];log", "icndvdvde"),
        (
            "let o={get m(){log+='g';return f}};o.m(...iterable,mark());log",
            "gicndvdvdef",
        ),
    ] {
        assert_eq!(
            eval(&format!("{setup}{body}")),
            Ok(Value::string(expected)),
            "{body}"
        );
    }
    for source in [
        "let n=0;let a={ [Symbol.iterator](){n++;return {next(){return {done:true}}}}};try{(0)(...a)}catch(e){}n===1",
        "let n=0;let a={ [Symbol.iterator](){n++;return {next(){return {done:true}}}}};try{new (0)(...a)}catch(e){}n===1",
        "let n=0;let a=[1];Object.defineProperty(a,0,{get(){if(n++===0)a.push(2);return 1}});[...a].join()==='1,2'",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn failed_spread_uses_iterator_step_errors_without_closing() {
    for body in [
        "throw 7",
        "return 1",
        "return {get done(){throw 7}}",
        "return {get value(){throw 7}}",
    ] {
        for expression in ["f(...i,after())", "[...i,after()]", "new F(...i,after())"] {
            let source = format!(
                "let log='';let i={{[Symbol.iterator](){{return {{next(){{{body}}},return(){{log+='r';return {{}}}}}}}}}};function after(){{log+='a'}}function f(){{log+='f'}}function F(){{log+='c'}}try{{{expression}}}catch(e){{}}log"
            );
            assert_eq!(
                eval(&source),
                Ok(Value::string("")),
                "{body} / {expression}"
            );
        }
    }
    for source in [
        "[...null]",
        "[...undefined]",
        "[...{length:2}]",
        "(function(){})(...1)",
        "new Array(...true)",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
    for source in [
        "...x",
        "[... ]",
        "f(...)",
        "f(, ...[])",
        "let x=...y",
        "function f(...a,){}",
        "function f(...a,b){}",
    ] {
        assert!(compile(source, Limits::default()).is_err(), "{source}");
    }
}

#[test]
fn spread_construction_super_calls_and_new_target() {
    for source in [
        "function C(a,b){this.n=a+b;this.t=new.target}let c=new C(...[2,3]);c.n===5&&c.t===C",
        "class A{constructor(...a){this.values=a;this.target=new.target}}class B extends A{constructor(...a){super(0,...a,3)}}let b=new B(...[1,2]);b.values.join()==='0,1,2,3'&&b.target===B",
        "class A{m(...a){return this.x+a[0]}}class B extends A{m(...a){return super.m(...a)}}let b=new B();b.x=3;b.m(...[4])===7",
        "class A{}class B extends A{constructor(...a){let f=()=>super(...a);f();this.n=7}}new B(...[]).n===7",
        "let a=new Array(...[1,2]);a.length===2&&a[1]===2",
        "class A{constructor(){this.x='A'}}class C{constructor(){this.x='C'}}class B extends A{constructor(){super(...{[Symbol.iterator](){Object.setPrototypeOf(B,C);return {next(){return {done:true}}}}})}}new B().x==='A'",
        "class A{constructor(){this.x='A'}}class C{constructor(){this.x='C'}}class B extends A{constructor(){super(Object.setPrototypeOf(B,C))}}new B().x==='A'",
        "class A{constructor(){this.x=1}}class B extends A{constructor(...a){super(...a)}}let i=Array.prototype[Symbol.iterator],n=0;Array.prototype[Symbol.iterator]=function(){n++;return i.call(this)};new B();n===1",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn spread_intermediates_survive_getter_gc_and_await() -> Result<(), Error> {
    let limits = Limits {
        heap_entries: 150,
        ..Limits::default()
    };
    for source in [
        "function f(...a){return a[0].n+a[1].n}let i={[Symbol.iterator](){let n=0;return {next(){for(let k=0;k<200;k++){let t={}}return n++?{done:true}:{value:{n:7}}}}}};f(...i,{n:3})===10",
        "let i={[Symbol.iterator](){let n=0;return {next(){for(let k=0;k<200;k++){let t={}}return n++?{done:true}:{value:{n:7}}}}}};let a=[{n:3},...i];a[0].n+a[1].n===10",
        "async function f(){function sum(...a){return a.join()}let x=sum(1,...[2],await 3,...[4]);let y=[,...[5],await 6,,];if(x!=='1,2,3,4'||y.length!==4||y[2]!==6)throw 7}f();true",
        "async function f(){let e={};try{(function(){})(...[{x:1}],await Promise.reject(e));}catch(x){if(x!==e)throw 9}}f();true",
        "let cb=(...args)=>args.length;for(let i=0;i<100;i++){try{cb(...null)}catch(e){}}cb(...[1,2])===2",
        "let o={x:7,m(...a){return this.x+a[0].x+a[1].x}};async function f(){let v=o.m(...[{x:1}],await {x:2});if(v!==10)throw 9}f();true",
    ] {
        assert_eq!(
            Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
            Value::Boolean(true),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn spread_limits_bound_infinite_iterators_and_argument_storage() -> Result<(), Error> {
    let limits = Limits {
        fuel: 1000,
        ..Limits::default()
    };
    for source in [
        "let i={[Symbol.iterator](){return {next(){return {value:1}}}}};[...i]",
        "let i={[Symbol.iterator](){return {next(){return {value:1}}}}};(function(){})(...i)",
    ] {
        assert!(
            matches!(
                Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost),
                Err(Error::Limit { .. })
            ),
            "{source}"
        );
    }
    let limits = Limits {
        stack: 16,
        ..Limits::default()
    };
    let source = "(function(){})(...[1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16])";
    assert!(matches!(
        Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost),
        Err(Error::Limit { .. })
    ));
    Ok(())
}

#[test]
fn expansion_uses_iterator_and_create_data_semantics_not_array_helpers() {
    for source in [
        "let a=[1,2];Object.defineProperty(a,'constructor',{get(){throw 9}});[...a].join()==='1,2'",
        "let a=[1,2];a[Symbol.iterator]=function(){return 'ab'[Symbol.iterator]()};(function(...x){return x.join()})(...a)==='a,b'",
        "Object.defineProperty(Array.prototype,0,{value:9,writable:false,configurable:true});[...[1]][0]===1",
        "let s=Symbol('x');let a=[s];[...a][0]===s&&(function(x){return x})(...a)===s",
        "let closed=false;let i={[Symbol.iterator](){return {next(){return {done:true,get value(){throw 9}}},return(){closed=true;return {}}}}};[...i].length===0&&!closed",
        "let a=[1];let i=a[Symbol.iterator]();i.next();i.next();a.push(2);[...i].length===0",
        "let n=0;try{(function(){})(...(function(){throw 7})(),n++)}catch(e){}n===0",
        "let n=0;try{new (function(){})(...[1],...(function(){throw 7})(),n++)}catch(e){}n===0",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn expanded_arguments_use_the_same_stack_budget_as_suspended_operands() -> Result<(), Error> {
    let source = "function f(...a){return a.length}f(...'abcdefghijkl')";
    let limits = Limits {
        stack: 16,
        ..Limits::default()
    };
    assert_eq!(
        Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
        Value::Number(12.0)
    );
    let limits = Limits {
        stack: 16,
        ..Limits::default()
    };
    let source = "function f(...a){return a.length}f(...'abcdefghijklmn')";
    assert!(matches!(
        Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost),
        Err(Error::Limit { .. })
    ));
    let limits = Limits {
        properties: 32,
        ..Limits::default()
    };
    let source = "[...'abcdefghijklmnopqrstuvwxyz123456789']";
    assert!(matches!(
        Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost),
        Err(Error::Limit { .. })
    ));
    Ok(())
}
