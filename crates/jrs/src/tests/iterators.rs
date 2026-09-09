// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::{Error, Limits, Runtime, SilentHost, Value, compile, eval};

#[test]
fn builtin_iterators_are_live_branded_and_self_iterable() {
    for source in [
        "Array.prototype[Symbol.iterator]===Array.prototype.values",
        "let i=[1].values();i[Symbol.iterator]()===i&&i.next().value===1&&i.next().done&&i.next().done",
        "let a=[1],i=a.values();i.next();a.push(2);i.next().value===2&&i.next().done",
        "let a=[1],i=a.values();i.next();i.next();a.push(2);i.next().done",
        "let i=[,3].entries();i.next().value.join()==='0,'&&i.next().value.join()==='1,3'",
        "let a={get 0(){throw 1},length:1};Array.prototype.keys.call(a).next().value===0",
        "let i='😀a\\ud800'[Symbol.iterator]();i.next().value==='😀'&&i.next().value==='a'&&i.next().value==='\\ud800'&&i.next().done",
        "Object.prototype.toString.call([].values())==='[object Array Iterator]'",
        "Object.prototype.toString.call(''[Symbol.iterator]())==='[object String Iterator]'",
        "let out='';for(let x of [2,3].keys())out+=x;out==='01'",
        "function f(){let out=0;for(let x of arguments)out+=x;return out}f(2,3)===5",
        "let i=Array.prototype.values.call('ab');i.next().value==='a'&&i.next().value==='b'",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "[].values().next.call({})",
        "[].values().next.call(''[Symbol.iterator]())",
        "''[Symbol.iterator]().next.call([].values())",
        "Array.prototype.values.call(null)",
        "String.prototype[Symbol.iterator].call(undefined)",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn iterator_next_state_order_reentry_and_metadata() {
    for (source, expected) in [
        (
            "let a={get length(){throw 7}},i=Array.prototype.values.call(a);try{i.next()}catch(e){}Object.defineProperty(a,'length',{value:1});a[0]=9;i.next().value",
            Value::Number(9.0),
        ),
        (
            "let a={length:2,get 0(){throw 7},1:9},i=Array.prototype.values.call(a);try{i.next()}catch(e){}i.next().value",
            Value::Number(9.0),
        ),
        (
            "let n=0,i;let a={get length(){if(n++===0)i.next();return 2},0:'a',1:'b'};i=Array.prototype.values.call(a);i.next().value+i.next().value",
            Value::string("ab"),
        ),
        (
            "let i;let a={length:2,get 0(){return i.next().value},1:9};i=Array.prototype.values.call(a);i.next().value",
            Value::Number(9.0),
        ),
        (
            "let i=[1].values();Object.freeze(i);i.next().value",
            Value::Number(1.0),
        ),
        (
            "let p=Object.getPrototypeOf([].values());Object.getPrototypeOf(p)===Object.getPrototypeOf(Object.getPrototypeOf(''[Symbol.iterator]()))",
            Value::Boolean(true),
        ),
        (
            "[].values.name+'|'+[].values.length+'|'+[].values().next.name+'|'+[].values().next.length",
            Value::string("values|0|next|0"),
        ),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
}

#[test]
fn weakmap_closes_entry_errors_but_not_next_errors() {
    for (entry, expected) in [
        ("1", "rTypeError"),
        ("{get 0(){throw 3}}", "r3"),
        ("{0:{},get 1(){throw 4}}", "r4"),
    ] {
        let source = format!(
            "let log='';let input={{[Symbol.iterator](){{return {{next(){{return {{value:{entry}}}}},return(){{log+='r';return {{}}}}}}}}}};try{{new WeakMap(input)}}catch(e){{log+=e.name||e}}log"
        );
        assert_eq!(eval(&source), Ok(Value::string(expected)), "{entry}");
    }
    let source = "let log='';let input={[Symbol.iterator](){return {next(){throw 3},return(){log+='r';return {}}}}};try{new WeakMap(input)}catch(e){log+=e}log";
    assert_eq!(eval(source), Ok(Value::string("3")));
}

#[test]
fn iterator_closing_preserves_outer_completion_and_embedding_failures() -> Result<(), Error> {
    struct Failing;
    impl crate::Host for Failing {
        fn print(&mut self, _: &[Value]) -> Result<(), Error> {
            Err(Error::Host)
        }
    }
    for (body, expected) in [
        ("i.return=null;for(let x of i){break}log", ""),
        (
            "i.return=7;try{for(let x of i){throw 3}}catch(e){log+=e}log",
            "3",
        ),
        (
            "i.return=7;try{for(let x of i){break}}catch(e){log+=e.name}log",
            "TypeError",
        ),
        (
            "function f(){try{for(let x of i){return 3}}finally{log+='f'}}let value=f();log+value",
            "rf3",
        ),
        (
            "function f(){for(let x of i){try{return 3}finally{break}}return 4}let value=f();log+value",
            "r4",
        ),
        ("for(let x of i){switch(x){case 1:break}break}log", "r"),
    ] {
        assert_eq!(
            eval(&format!("{INPUT}{body}")),
            Ok(Value::string(expected)),
            "{body}"
        );
    }
    let source = format!(
        "{INPUT}i.return=function(){{print(1);return {{}}}};try{{for(let x of i){{throw 3}}}}catch(e){{}}"
    );
    let limits = Limits::default();
    assert_eq!(
        Runtime::new(limits).run(&compile(&source, limits)?, &mut Failing),
        Err(Error::Host)
    );
    Ok(())
}

#[test]
fn user_iterators_cache_next_and_observe_done_before_value() {
    for (source, expected) in [
        (
            "let log='';let a={get [Symbol.iterator](){log+='i';return function(){log+='c';let n=0;return {get next(){log+='n';return function(){return {get done(){log+='d';return n===2},get value(){log+='v';return ++n}}}}}}}};let sum=0;for(let x of a)sum+=x;log+sum",
            "icndvdvd3",
        ),
        (
            "let i={n:0,next(){this.next=()=>{throw 9};return {value:++this.n,done:this.n>2}},[Symbol.iterator](){return this}};let s=0;for(let v of i)s+=v;String(s)",
            "3",
        ),
        (
            "let a=[1,2];a[Symbol.iterator]=function(){return {next(){return {done:true}}}};let n=0;for(let x of a)n++;String(n)",
            "0",
        ),
        (
            "let a=[1];let out='';for(let x of a){out+=x;if(x===1)a.push(2)}out",
            "12",
        ),
    ] {
        assert_eq!(eval(source), Ok(Value::string(expected)), "{source}");
    }
    for source in [
        "for(let x of {}){}",
        "for(let x of {[Symbol.iterator]:3}){}",
        "for(let x of {[Symbol.iterator](){return 1}}){}",
        "for(let x of {[Symbol.iterator](){return {next:3}}}){}",
        "for(let x of {[Symbol.iterator](){return {next(){return 1}}}}){}",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

const INPUT: &str = "let log='';let i={next(){return {value:1,done:false}},return(){log+='r';return {}},[Symbol.iterator](){return this}};";

#[test]
fn iterator_close_runs_at_abrupt_exits_in_finally_order() {
    for (body, expected) in [
        ("for(let x of i){break}log", "r"),
        ("function f(){for(let x of i){return 3}}f();log", "r"),
        ("try{for(let x of i){throw 3}}catch(e){log+=e}log", "r3"),
        ("for(let x of i){try{break}finally{log+='f'}}log", "fr"),
        ("try{for(let x of i){break}}finally{log+='f'}log", "rf"),
        (
            "for(let x of i){try{throw 2}catch(e){log+='c'}break}log",
            "cr",
        ),
        ("let n=0;for(let x of i){if(n++===0)continue;break}log", "r"),
        (
            "i.return=function(){log+='r';throw 9};try{for(let x of i){throw 3}}catch(e){log+=e}log",
            "r3",
        ),
        (
            "i.return=function(){log+='r';throw 9};try{for(let x of i){break}}catch(e){log+=e}log",
            "r9",
        ),
        (
            "i.return=function(){return 1};try{for(let x of i){break}}catch(e){log=e.name}log",
            "TypeError",
        ),
        (
            "Object.defineProperty(i,'return',{get(){log+='g';throw 9}});try{for(let x of i){throw 3}}catch(e){log+=e}log",
            "g3",
        ),
        ("for(let x of i){for(let y of i){break}break}log", "rr"),
    ] {
        let source = format!("{INPUT}{body}");
        assert_eq!(eval(&source), Ok(Value::string(expected)), "{body}");
    }
}

#[test]
fn next_result_failures_do_not_close_the_failed_iterator() {
    for next in [
        "throw 7",
        "return {get done(){throw 7}}",
        "return {get value(){throw 7}}",
    ] {
        let source = format!(
            "{INPUT}i.next=function(){{{next}}};try{{for(let x of i){{}}}}catch(e){{log+=e}}log"
        );
        assert_eq!(eval(&source), Ok(Value::string("7")), "{next}");
    }
}

#[test]
fn array_binding_patterns_close_inner_iterators_and_skip_elision_values() {
    for (body, expected) in [
        ("for(let [a] of [i]){log+=a}log", "r1"),
        ("for(let [] of [i]){}log", "r"),
        (
            "i.next=function(){return {get value(){throw 7}}};for(let [,] of [i]){}log",
            "r",
        ),
        (
            "i.next=function(){return {done:true,get value(){throw 7}}};for(let [a,b] of [i]){log+=String(a)+String(b)}log",
            "undefinedundefined",
        ),
        (
            "i.return=function(){log+='r';throw 7};try{for(let [a] of [i]){log+='b'}}catch(e){log+=e}log",
            "r7",
        ),
    ] {
        let source = format!("{INPUT}{body}");
        assert_eq!(eval(&source), Ok(Value::string(expected)), "{body}");
    }
}

#[test]
fn weakmap_and_promise_combinators_use_the_actual_iterator() {
    for source in [
        "let key={},n=0;let input={[Symbol.iterator](){return {next(){return n++?{done:true}:{value:[key,7]}}}}};new WeakMap(input).get(key)===7",
        "let key={},a=[];a[Symbol.iterator]=function(){let n=0;return {next(){return n++?{done:true}:{value:[key,8]}}}};new WeakMap(a).get(key)===8",
        "let log='';let input={[Symbol.iterator](){return {next(){return {value:1}},return(){log+='r';throw 9}}}};try{new WeakMap(input)}catch(e){log+=e.name}log==='rTypeError'",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    assert_eq!(
        eval(
            "let log='';let n=0;let input={[Symbol.iterator](){return {next(){return n++<2?{value:n}:{done:true}}}}};Promise.all(input).then(a=>{if(a.join()!=='1,2')throw 7});0"
        ),
        Ok(Value::Number(0.0))
    );
}

#[test]
fn iterator_roots_and_async_suspensions_survive_collection() -> Result<(), Error> {
    let limits = Limits {
        heap_entries: 110,
        ..Limits::default()
    };
    for source in [
        "let it=[{n:7}].values();for(let i=0;i<200;i++){let t={}}it.next().value.n===7",
        "let out=0;for(let x of [{n:1},{n:2}]){for(let i=0;i<200;i++){let t={}}out+=x.n}out===3",
        "let log='';let it={next(){return {value:1}},return(){for(let i=0;i<200;i++){let t={}}log+='r';return {}},[Symbol.iterator](){return this}};function f(){for(let x of it)return {n:7}}f().n===7&&log==='r'",
        "async function f(){let out=0;for(let x of [{n:1},{n:2}]){await 1;for(let i=0;i<200;i++){let t={}}out+=x.n}if(out!==3)throw 7}f();true",
    ] {
        assert_eq!(
            Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
            Value::Boolean(true),
            "{source}"
        );
    }
    let limits = Limits {
        fuel: 1000,
        ..Limits::default()
    };
    assert!(matches!(
        Runtime::new(limits).run(
            &compile(
                "for(let x of {[Symbol.iterator](){return {next(){return {value:1}}}}}){}",
                limits
            )?,
            &mut SilentHost
        ),
        Err(Error::Limit { .. })
    ));
    Ok(())
}
