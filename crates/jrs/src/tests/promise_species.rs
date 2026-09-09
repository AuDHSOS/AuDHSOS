// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::{Error, Limits, Runtime, SilentHost, Value, compile, eval};

fn check(source: &str) {
    assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
}

#[test]
fn promise_species_constructs_the_selected_result_and_preserves_identity() {
    for source in [
        "class P extends Promise{}let p=P.resolve(7);p instanceof P&&P.resolve(p)===p&&Promise.resolve(p)!==p",
        "class P extends Promise{}let p=P.reject(7);p.catch(()=>{});p instanceof P",
        "class P extends Promise{}let r=P.withResolvers();r.promise instanceof P&&typeof r.resolve==='function'&&typeof r.reject==='function'",
        "class P extends Promise{}let q=P.resolve(1).then(x=>x);q instanceof P",
        "class P extends Promise{static get [Symbol.species](){return Promise}}let q=P.resolve(1).then(x=>x);q instanceof Promise&&!(q instanceof P)",
        "class P extends Promise{static get [Symbol.species](){return null}}!(P.resolve(1).then() instanceof P)",
        "class Q extends Promise{}class P extends Promise{static get [Symbol.species](){return Q}}P.resolve(1).then() instanceof Q",
        "let p=Promise.resolve(1);p.constructor=undefined;p.then() instanceof Promise",
        "let c={},p=Promise.resolve(1);p.constructor=c;Promise.resolve.call(c,p)===p",
        "Promise[Symbol.species]===Promise&&Object.getOwnPropertyDescriptor(Promise,Symbol.species).set===undefined",
        "class P extends Promise{}P[Symbol.species]===P",
        "Object.prototype.toString.call(Promise.resolve())==='[object Promise]'",
    ] {
        check(source);
    }
}

#[test]
fn generic_capabilities_call_resolvers_with_undefined_receiver() {
    for source in [
        "let v,t;function C(ex){ex(function(x){'use strict';v=x;t=this},()=>{});this.x=1}let p=Promise.resolve.call(C,7);v===7&&t===undefined&&p.x===1",
        "let v;function C(ex){ex(()=>{},x=>{v=x})}Promise.reject.call(C,9);v===9",
        "let a=[];function C(ex){ex(x=>a.push(x),x=>a.push(x));return {x:7}}let p=Promise.resolve.call(C,3);p.x===7&&a[0]===3",
        "let f;function C(ex){f=ex;ex(()=>{},()=>{})}Promise.withResolvers.call(C);let ok=false;try{f(()=>{},()=>{})}catch(e){ok=e instanceof TypeError}ok",
        "function C(ex){ex(undefined,undefined);ex(()=>{},()=>{})}Promise.withResolvers.call(C).promise instanceof C",
        "let ok=false;function C(ex){ok=ex.length===2&&ex.name===''&&!Object.hasOwn(ex,'prototype');try{new ex()}catch(e){ok=ok&&e instanceof TypeError}ex(()=>{},()=>{})}Promise.withResolvers.call(C);ok",
    ] {
        check(source);
    }
    for source in [
        "Promise.resolve.call(1,3)",
        "Promise.reject.call({},3)",
        "Promise.withResolvers.call(()=>{})",
        "function C(ex){}Promise.resolve.call(C,3)",
        "function C(ex){ex(1,()=>{})}Promise.resolve.call(C,3)",
        "function C(ex){ex(()=>{},1)}Promise.resolve.call(C,3)",
        "function C(ex){ex(undefined,()=>{});ex(()=>{},()=>{})}Promise.resolve.call(C,3)",
        "function C(ex){ex(()=>{},undefined);ex(()=>{},()=>{})}Promise.resolve.call(C,3)",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
    assert_eq!(
        eval("function C(ex){ex(()=>{throw 7},()=>{});}try{Promise.resolve.call(C,1)}catch(e){e}"),
        Ok(Value::Number(7.0))
    );
    assert_eq!(
        eval("function C(ex){ex(()=>{},()=>{throw 9});}try{Promise.reject.call(C,1)}catch(e){e}"),
        Ok(Value::Number(9.0))
    );
}

#[test]
fn reaction_jobs_use_captured_capability_functions_and_do_not_convert_their_errors() {
    for source in [
        "let log='';function C(ex){ex(v=>{log+='r'+v},v=>{log+='j'+v});}let p=Promise.resolve(2);p.constructor={[Symbol.species]:C};p.then(x=>x+3);Promise.resolve().then(()=>{if(log!=='r5')throw log});true",
        "let log='';function C(ex){ex(v=>{log+='r'+v},v=>{log+='j'+v});}let p=Promise.reject(2);p.constructor={[Symbol.species]:C};p.then();Promise.resolve().then(()=>{if(log!=='j2')throw log});true",
        "let log='';function C(ex){ex(v=>{log+='r'+v},v=>{log+='j'+v});}let p=Promise.resolve(2);p.constructor={[Symbol.species]:C};p.then(()=>{throw 7});Promise.resolve().then(()=>{if(log!=='j7')throw log});true",
        "let log='';function C(ex){ex(v=>{log+='r'+v},v=>{log+='j'+v});}let r=Promise.withResolvers();r.promise.constructor={[Symbol.species]:C};r.promise.then();C=null;r.resolve(4);Promise.resolve().then(()=>{if(log!=='r4')throw log});true",
    ] {
        check(source);
    }
    assert_eq!(
        eval(
            "let rejected=false;function C(ex){ex(()=>{throw 7},()=>{rejected=true})}let p=Promise.resolve(1);p.constructor={[Symbol.species]:C};p.then();"
        ),
        Err(Error::Thrown {
            value: Value::Number(7.0)
        })
    );
}

#[test]
fn species_errors_and_getter_order_are_observable() {
    for source in [
        "let log='';function C(ex){log+='C';ex(()=>{},()=>{})}let p=Promise.resolve(1);Object.defineProperty(p,'constructor',{get(){log+='c';return {get [Symbol.species](){log+='s';return C}}}});p.then();log==='csC'",
        "let touched=0;let p=Promise.resolve(1);p.constructor={get [Symbol.species](){touched++;throw 7}};try{p.finally(null)}catch(e){}touched===1",
        "let obj={get constructor(){throw 7},then(){throw 9}};let got=0;try{Promise.prototype.finally.call(obj,()=>{})}catch(e){got=e}got===7",
        "let p=Promise.resolve(1);Object.defineProperty(p,'constructor',{get(){throw 7}});let got=0;try{Promise.resolve(p)}catch(e){got=e}got===7",
    ] {
        check(source);
    }
    for source in [
        "let p=Promise.resolve(1);p.constructor=null;p.then()",
        "let p=Promise.resolve(1);p.constructor=1;p.then()",
        "let p=Promise.resolve(1);p.constructor={[Symbol.species]:()=>{}};p.then()",
        "let p=Promise.resolve(1);p.constructor={[Symbol.species]:1};p.finally(3)",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn combinators_are_generic_get_resolve_once_and_close_abrupt_iterations() {
    for source in [
        "class P extends Promise{}P.all([1,2]).then(a=>{if(a.join()!=='1,2')throw 7});P.race([3]).then(v=>{if(v!==3)throw 9});P.allSettled([1]).then(a=>{if(a[0].status!=='fulfilled')throw 5});true",
        "let log='';function C(ex){log+='C';ex(v=>{log+='R'+v.join()},v=>{log+='J'+v})}C.resolve=x=>({then(f){f(x);f(99)}});let p=Promise.all.call(C,[1,2]);p instanceof C&&log==='CR1,2'",
        "let log='';function C(ex){ex(v=>{log+='R'+v},v=>{log+='J'+v})}C.resolve=x=>({then(f){f(x)}});Promise.race.call(C,[1,2]);log==='R1R2'",
        "let log='';function C(ex){log+='C';ex(()=>{},v=>{log+='J'+v.name})}C.resolve=1;let input={get [Symbol.iterator](){log+='i';return ()=>({next(){return {done:true}}})}};Promise.all.call(C,input);log==='CJTypeError'",
        "let log='';function C(ex){ex(()=>{},e=>{log+='J'+e})}C.resolve=()=>{throw 7};let input={[Symbol.iterator](){return {next(){return {value:1}},return(){log+='r';throw 9}}}};Promise.all.call(C,input);log==='rJ7'",
        "let gets=0;class P extends Promise{static get resolve(){gets++;return Promise.resolve}}P.all([1,2]);gets===1",
    ] {
        check(source);
    }
}

#[test]
fn finally_uses_species_without_reading_public_resolve() {
    for source in [
        "class P extends Promise{static resolve(){throw 9}}let p=new P(r=>r(3));let q=p.finally(()=>7);if(!(q instanceof P))throw 1;q.then(v=>{if(v!==3)throw 2});true",
        "class Q extends Promise{}class P extends Promise{static get [Symbol.species](){return Q}}let q=P.resolve(1).finally(()=>2);q instanceof Q",
        "let p=Promise.reject(3);p.finally(()=>1).catch(e=>{if(e!==3)throw 9});true",
        "let p=Promise.resolve(3);p.finally(()=>{throw 7}).catch(e=>{if(e!==7)throw 9});true",
    ] {
        check(source);
    }
}

#[test]
fn capabilities_and_species_closures_survive_collection() -> Result<(), Error> {
    let limits = Limits {
        heap_entries: 150,
        ..Limits::default()
    };
    for source in [
        "let log='';function C(ex){for(let i=0;i<200;i++){let t={}}ex(v=>{log+=v.n},()=>{})}let p=Promise.resolve(1);p.constructor={[Symbol.species]:C};p.then(()=>({n:7}));for(let i=0;i<200;i++){let t={}}Promise.resolve().then(()=>{if(log!=='7')throw 9});true",
        "class P extends Promise{}let r=P.withResolvers();let p=r.promise.then(v=>v+1);for(let i=0;i<200;i++){let t={}}r.resolve(2);p.then(v=>{if(v!==3)throw 7});true",
        "let p=Promise.resolve(1);Object.defineProperty(p,'constructor',{get(){for(let i=0;i<200;i++){let t={}}return {get [Symbol.species](){return class P extends Promise{}}}}});p.then(v=>v);true",
    ] {
        assert_eq!(
            Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
            Value::Boolean(true),
            "{source}"
        );
    }
    Ok(())
}
