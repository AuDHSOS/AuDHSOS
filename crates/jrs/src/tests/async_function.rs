// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use crate::{Error, Limits, Realm, SilentHost, Value};
fn check(source: &str) -> Result<(), Error> {
    let mut host = SilentHost;
    let mut r = Realm::new(Limits::default(), &mut host)?;
    assert_eq!(r.evaluate(source)?, Value::Boolean(true), "{source}");
    Ok(())
}

#[test]
fn async_intrinsics_have_distinct_identity_brands_and_descriptors() -> Result<(), Error> {
    for source in [
        "let A=(async function(){}).constructor,P=A.prototype;A!==Function&&Object.getPrototypeOf(A)===Function&&Object.getPrototypeOf(P)===Function.prototype&&typeof P==='object'",
        "let A=(async()=>{}).constructor;A===(async function(){}).constructor&&A===({async m(){}}).m.constructor",
        "class C{async m(){}static async s(){}}let A=(async()=>{}).constructor;C.prototype.m.constructor===A&&C.s.constructor===A",
        "let A=(async()=>{}).constructor;A.name==='AsyncFunction'&&A.length===1&&Object.isExtensible(A)&&Object.isExtensible(A.prototype)",
        "let A=(async()=>{}).constructor;Object.prototype.toString.call(A.prototype)==='[object AsyncFunction]'&&Object.prototype.toString.call(async()=>{})==='[object AsyncFunction]'",
        "let A=(async()=>{}).constructor,d=Object.getOwnPropertyDescriptor(A,'prototype');!d.writable&&!d.enumerable&&!d.configurable&&d.value===A.prototype",
        "let A=(async()=>{}).constructor,d=Object.getOwnPropertyDescriptor(A.prototype,'constructor');!d.writable&&!d.enumerable&&d.configurable&&d.value===A",
        "let A=(async()=>{}).constructor,d=Object.getOwnPropertyDescriptor(A.prototype,Symbol.toStringTag);!d.writable&&!d.enumerable&&d.configurable&&d.value==='AsyncFunction'",
        "let A=(async()=>{}).constructor;Object.getOwnPropertyNames(A).join()==='length,name,prototype'&&Object.getOwnPropertyNames(A.prototype).join()==='constructor'",
        "let A=(async()=>{}).constructor;typeof AsyncFunction==='undefined'&&!Object.hasOwn(globalThis,'AsyncFunction')&&A.toString()==='function AsyncFunction() { [native code] }'",
        "let A=(async()=>{}).constructor;A.x=7;delete A.length;A.x===7&&!Object.hasOwn(A,'length')",
        "let A=(async()=>{}).constructor;let n=0;for(let k in A){n++}n===0&&'call' in A&&'bind' in A",
    ] {
        check(source)?;
    }
    Ok(())
}

#[test]
fn dynamic_async_bodies_use_promise_jobs_global_environment_and_source() -> Result<(), Error> {
    for source in [
        "let A=(async()=>{}).constructor,f=A('a','b','return await a+b');f.name==='anonymous'&&f.length===2&&Object.getPrototypeOf(f)===A.prototype&&f.prototype===undefined&&!Object.hasOwn(f,'prototype')",
        "let A=(async()=>{}).constructor,f=new A('return 7');f() instanceof Promise",
        "let A=(async()=>{}).constructor;let f=A('a=1','...r','return a+r.length');f.length===0&&f instanceof A&&f instanceof Function",
    ] {
        check(source)?;
    }
    let mut host = SilentHost;
    let mut r = Realm::new(Limits::default(), &mut host)?;
    r.evaluate(
        "let A=(async()=>{}).constructor;let result;A('x','return await x+1')(6).then(x=>result=x)",
    )?;
    assert_eq!(r.evaluate("result")?, Value::Number(7.0));
    assert_eq!(
        r.evaluate("A('x','return await x+1').toString()")?,
        Value::string("async function anonymous(x\n) {\nreturn await x+1\n}")
    );
    r.evaluate("let globalValue=7;function make(){let globalValue=9;return A('return globalValue')}make()().then(x=>result=x)")?;
    assert_eq!(r.evaluate("result")?, Value::Number(7.0));
    r.evaluate("let log='';A(\"log+='a';await 0;log+='b'\")();log+='c';if(log!=='ac')throw 7")?;
    assert_eq!(r.evaluate("log")?, Value::string("acb"));
    r.evaluate("let reason={n:7};A('throw reason')().then(()=>{throw 9},e=>result=e===reason)")?;
    assert_eq!(r.evaluate("result")?, Value::Boolean(true));
    r.evaluate("A('try{await Promise.reject(reason)}finally{log+=1}')().then(()=>{throw 9},e=>result=e===reason)")?;
    assert_eq!(r.evaluate("result&&log==='acb1'")?, Value::Boolean(true));
    Ok(())
}

#[test]
fn async_construction_subclasses_bound_calls_and_new_target() -> Result<(), Error> {
    for source in [
        "let A=(async()=>{}).constructor;class Sub extends A{}let f=new Sub('return 7');Object.getPrototypeOf(f)===Sub.prototype&&f instanceof Sub&&f instanceof A&&!Object.hasOwn(f,'prototype')",
        "let A=(async()=>{}).constructor;function T(){}let f=Reflect.construct(A,['return 7'],T);Object.getPrototypeOf(f)===T.prototype&&f() instanceof Promise",
        "let A=(async()=>{}).constructor;function T(){}T.prototype=null;Object.getPrototypeOf(Reflect.construct(A,[],T))===A.prototype",
        "let A=(async()=>{}).constructor,B=A.bind(null,'x','return await x');let f=new B();f.length===1&&Object.getPrototypeOf(f)===A.prototype",
        "let A=(async()=>{}).constructor;let n=0;try{new (A('n++'))}catch(e){if(!(e instanceof TypeError))throw e}n===0",
        "let A=(async()=>{}).constructor;let yes=false;try{Reflect.construct(A(''),[])}catch(e){yes=e instanceof TypeError}yes",
        "let A=(async()=>{}).constructor;let yes=false;try{A.prototype()}catch(e){yes=e instanceof TypeError}yes",
    ] {
        check(source)?;
    }
    let mut host = SilentHost;
    let mut r = Realm::new(Limits::default(), &mut host)?;
    r.evaluate(
        "let A=(async()=>{}).constructor,result;A('return new.target')().then(v=>result=v)",
    )?;
    assert_eq!(r.evaluate("result")?, Value::Undefined);
    r.evaluate("A('return this').call({n:7}).then(v=>result=v.n)")?;
    assert_eq!(r.evaluate("result")?, Value::Number(7.0));
    Ok(())
}

#[test]
fn async_dynamic_early_errors_and_gc_are_bounded() -> Result<(), Error> {
    for (params, body) in [
        ("a,a", ""),
        ("await", "return 1"),
        ("a=await 1", ""),
        ("a=1", "'use strict'"),
        ("/*", "*/) {}"),
        ("a", "let a"),
        ("", "return super.x"),
        ("a)", "return 7"),
    ] {
        check(&format!(
            "let A=(async()=>{{}}).constructor,ok=false;try{{A({params:?},{body:?})}}catch(e){{ok=e instanceof SyntaxError}}ok"
        ))?;
    }
    let mut host = SilentHost;
    let mut r = Realm::new(
        Limits {
            heap_entries: 190,
            ..Limits::default()
        },
        &mut host,
    )?;
    let gc = r.gc_function()?;
    r.set_global("gc", &gc)?;
    r.evaluate("let A=(async()=>{}).constructor,result;let f=A({toString(){gc();return 'a'}},{toString(){gc();return 'let o={a};await 0;gc();return o'}});f(7).then(x=>result=x.a);for(let i=0;i<300;i++){let g={}}")?;
    assert_eq!(r.evaluate("result")?, Value::Number(7.0));
    assert_eq!(
        r.evaluate("(async()=>{}).constructor===A")?,
        Value::Boolean(true)
    );
    Ok(())
}

#[test]
fn dynamic_async_parameter_failures_reject_without_running_body() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut r = Realm::new(Limits::default(), &mut host)?;
    r.evaluate("let A=(async()=>{}).constructor,n=0,caught=false,result;let f=A('x=missing','n++;return 7');let p;try{p=f()}catch(e){caught=true}p.catch(e=>result=e instanceof ReferenceError)")?;
    assert_eq!(r.evaluate("!caught&&n===0&&result")?, Value::Boolean(true));
    r.evaluate("A('x=y','y=1','return 7')().catch(e=>result=e instanceof ReferenceError)")?;
    assert_eq!(r.evaluate("result")?, Value::Boolean(true));
    r.evaluate("A('a=1','arguments[0]=7;return a')().then(v=>result=v)")?;
    assert_eq!(r.evaluate("result")?, Value::Number(1.0));
    r.evaluate("A('x','arguments[0]=7;return x')(1).then(v=>result=v)")?;
    assert_eq!(r.evaluate("result")?, Value::Number(7.0));
    r.evaluate("A('return undefined')().then(v=>result=v)")?;
    assert_eq!(r.evaluate("result")?, Value::Undefined);
    Ok(())
}

#[test]
fn async_intrinsic_mutation_does_not_change_internal_prototype_selection() -> Result<(), Error> {
    for source in [
        "let A=(async()=>{}).constructor;Object.defineProperty(A.prototype,'constructor',{value:7});let f=async()=>{};Object.getPrototypeOf(f)===A.prototype&&f.constructor===7",
        "let A=(async()=>{}).constructor;delete A.prototype[Symbol.toStringTag];Object.prototype.toString.call(async()=>{})==='[object Function]'",
        "let A=(async()=>{}).constructor;Object.defineProperty(A,'name',{value:'changed'});A.toString()==='function AsyncFunction() { [native code] }'",
        "let A=(async()=>{}).constructor;class S extends A{constructor(){super('return 1');this.n=7}}new S().n===7",
        "let A=(async()=>{}).constructor,log='';A({toString(){log+='p';return 'a'}},{toString(){log+='b';return 'return await a'}});log==='pb'",
        "let A=(async()=>{}).constructor,ok=false;try{A(Symbol(),'')}catch(e){ok=e instanceof TypeError}ok",
    ] {
        check(source)?;
    }
    let mut host = SilentHost;
    let mut r = Realm::new(
        Limits {
            fuel: 3000,
            ..Limits::default()
        },
        &mut host,
    )?;
    assert!(matches!(
        r.evaluate("let A=(async()=>{}).constructor;while(true){A('return 7')}"),
        Err(Error::Limit { .. })
    ));
    Ok(())
}

#[test]
fn async_constructor_prototype_order_policy_and_native_limits() -> Result<(), Error> {
    use crate::Host;
    struct Deny;
    impl Host for Deny {
        fn print(&mut self, _: &[Value]) -> Result<(), Error> {
            Ok(())
        }
        fn ensure_can_compile_strings(&mut self) -> Result<(), Error> {
            Err(Error::Host)
        }
    }
    for source in [
        "let A=(async()=>{}).constructor,B=(function(){}).bind(null),n=0;Object.defineProperty(B,'prototype',{get(){n++;throw 7}});let ok=false;try{Reflect.construct(A,['bad)'],B)}catch(e){ok=e instanceof SyntaxError}ok&&n===0",
        "let A=(async()=>{}).constructor,B=(function(){}).bind(null),log='';let p={};Object.defineProperty(B,'prototype',{get(){log+='g';return p}});let f=Reflect.construct(A,[{toString(){log+='b';return 'return 1'}}],B);log==='bg'&&Object.getPrototypeOf(f)===p",
        "let A=(async()=>{}).constructor,B=(function(){}).bind(null),n=0;Object.defineProperty(B,'prototype',{get(){n++;throw 7}});let ok=false;try{Reflect.construct(A,['return 1'],B)}catch(e){ok=e===7}ok&&n===1",
        "let A=(async()=>{}).constructor;let b=(async()=>{}).bind(null),ok=false;try{Reflect.construct(A,[],b)}catch(e){ok=e instanceof TypeError}ok",
        "let A=(async()=>{}).constructor;Object.setPrototypeOf(A,null);let f=A('return 7');Object.getPrototypeOf(f)===A.prototype",
    ] {
        check(source)?;
    }
    let mut host = Deny;
    let mut r = Realm::new(Limits::default(), &mut host)?;
    assert_eq!(
        r.evaluate("(async()=>{}).constructor('invalid!!!')"),
        Err(Error::Host)
    );
    let mut host = SilentHost;
    let mut r = Realm::new(Limits::default(), &mut host)?;
    assert!(matches!(
        r.evaluate("let A=(async()=>{}).constructor,o={toString(){return A(o)}};A(o)"),
        Err(Error::Limit { .. })
    ));
    Ok(())
}
