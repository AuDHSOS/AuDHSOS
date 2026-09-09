// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use super::{Error, Limits, Runtime, SilentHost, Value, compile, eval};
#[test]
fn reflect_apply_and_construct_preserve_targets_and_argument_order() {
    for source in [
        "function f(a,b){return this.x+a+b}Reflect.apply(f,{x:1},[2,3])===6",
        "function C(a){this.x=a;this.target=new.target}function D(){}let c=Reflect.construct(C,[7],D);c.x===7&&c.target===D&&Object.getPrototypeOf(c)===D.prototype",
        "let f=Reflect.construct(Function,['a','return a+1']);f(2)===3",
        "let n=0;function f(a,b){return a===1&&b===2}let a={get length(){n++;return 2},get 0(){n++;return 1},get 1(){n++;return 2},[Symbol.iterator](){throw 7}};Reflect.apply(f,null,a)&&n===3",
        "let n=0;try{Reflect.apply(1,null,{get length(){n++;throw 7}})}catch(e){}n===0",
        "let n=0;try{Reflect.construct(Function,{get length(){n++;throw 7}},undefined)}catch(e){}n===0",
        "let log='';let args={get length(){log+='l';return 2},get 0(){log+='a';return 'a'},get 1(){log+='b';return 'return a'}};function T(){}T.prototype={};let f=Reflect.construct(Function,args,T);log==='lab'&&Object.getPrototypeOf(f)===T.prototype&&f(7)===7",
        "Object.getPrototypeOf(Reflect)===Object.prototype&&Object.prototype.toString.call(Reflect)==='[object Reflect]'&&Reflect.apply.length===3&&Reflect.construct.length===2",
        "Reflect.apply((...a)=>a.length,null,{length:2.9})===2&&Reflect.apply((...a)=>a.length,null,{length:NaN})===0",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "Reflect.apply(()=>{},null,null)",
        "Reflect.apply(()=>{},null,'abc')",
        "Reflect.construct(()=>{},[])",
        "Reflect.construct(Function,[],-1)",
        "new Reflect.apply()",
        "Reflect()",
        "new Reflect()",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}
#[test]
fn dynamic_constructor_prototype_getters_run_after_syntax_validation() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut realm = crate::Realm::new(Limits::default(), &mut host)?;
    let target = realm.host_function(1, "notConstructor", 0)?;
    realm.set_global("hostFn", &target)?;
    assert!(matches!(
        realm.evaluate("Reflect.construct(Function,[],hostFn)"),
        Err(Error::Type { .. })
    ));
    for source in [
        "function T(){}T.prototype=null;let f=Reflect.construct(Function,['return 7'],T);Object.getPrototypeOf(f)===Function.prototype&&f()===7",
        "let read=0;let args={get length(){read++;return 1},get 0(){throw 7}};let yes=false;try{Reflect.construct(Function,args)}catch(e){yes=e===7}yes&&read===1",
        "let e={};let yes=false;try{Reflect.apply(()=>{throw e},null,[])}catch(x){yes=x===e}yes",
    ] {
        assert_eq!(
            realm.evaluate(&format!("{{{source}}}"))?,
            Value::Boolean(true),
            "{source}"
        );
    }
    let limits = Limits {
        heap_entries: 170,
        ..Limits::default()
    };
    let source = "function C(x,y){this.n=x.n+y.n}let a={length:2,get 0(){return {n:3}},get 1(){for(let i=0;i<300;i++){let x={}}return {n:4}}};Reflect.construct(C,a).n===7";
    assert_eq!(
        Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
        Value::Boolean(true)
    );
    assert!(matches!(
        eval("Reflect.apply(()=>{},null,{length:Infinity})"),
        Err(Error::Limit { .. })
    ));
    Ok(())
}
