// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use super::{Error, Limits, Runtime, SilentHost, Value, compile, eval};

#[test]
fn at_relative_indices_holes_boxing_and_conversion_order() {
    for source in [
        "[1,2,3].at()===1&&[1,2,3].at(-1)===3&&[1,2,3].at(-3)===1",
        "[1,2,3].at(3)===undefined&&[1,2,3].at(-4)===undefined",
        "[1,2,3].at(1.9)===2&&[1,2,3].at(-1.9)===3",
        "[1].at(NaN)===1&&[1].at(-0.5)===1&&[1].at(-0)===1",
        "[1].at(Infinity)===undefined&&[1].at(-Infinity)===undefined",
        "[,].at(0)===undefined&&[].at(0)===undefined",
        "Array.prototype[0]=7;[,].at(0)===7",
        "Array.prototype.at.call('abc',-1)==='c'&&Array.prototype.at.call(true,0)===undefined",
        "Array.prototype.at.call('😀',-1)==='\\ude00'",
        "let log='';let o={get length(){log+='l';return {valueOf(){log+='n';return 2}}},get 1(){log+='g';return 7}};let v=Array.prototype.at.call(o,{valueOf(){log+='i';return -1}});v===7&&log==='lnig'",
        "let n=0;[].at({valueOf(){n++;return 0}});n===1",
        "let o={length:0,get 0(){throw 7},get '-1'(){throw 7}};Array.prototype.at.call(o,-1)===undefined&&Array.prototype.at.call(o,0)===undefined",
        "let a=[1,2,3];a.at({valueOf(){a.length=1;return -1}})===undefined&&a.length===1",
        "let o={length:2.9,1:7};Array.prototype.at.call(o,-1)===7",
        "let o={length:-1,0:7};Array.prototype.at.call(o,0)===undefined",
        "let e={},ok=false;try{Array.prototype.at.call({get length(){throw e}},0)}catch(x){ok=x===e}ok",
        "let e={},ok=false;try{[].at({valueOf(){throw e}})}catch(x){ok=x===e}ok",
        "let e={},ok=false;try{Array.prototype.at.call({length:1,get 0(){throw e}},0)}catch(x){ok=x===e}ok",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "Array.prototype.at.call(null,0)",
        "Array.prototype.at.call(undefined,0)",
        "[].at(Symbol())",
        "Array.prototype.at.call({length:Symbol()},0)",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn predicate_searches_visit_holes_in_order_and_return_original_values() {
    for source in [
        "[1,2,3].find(v=>v>1)===2&&[1,2,3].findIndex(v=>v>1)===1",
        "[1,2,3].findLast(v=>v>1)===3&&[1,2,3].findLastIndex(v=>v>1)===2",
        "[].find(()=>true)===undefined&&[].findIndex(()=>true)===-1",
        "[].findLast(()=>true)===undefined&&[].findLastIndex(()=>true)===-1",
        "[1].find(()=>false)===undefined&&[1].findLast(()=>false)===undefined",
        "[1].findIndex(()=>false)===-1&&[1].findLastIndex(()=>false)===-1",
        "let log='';[1,,3].find((v,k)=>{log+=k+':'+v+';';return false});log==='0:1;1:undefined;2:3;'",
        "let log='';[1,,3].findLast((v,k)=>{log+=k+':'+v+';';return false});log==='2:3;1:undefined;0:1;'",
        "[,].findIndex(v=>v===undefined)===0&&[,].findLastIndex(v=>v===undefined)===0",
        "Array.prototype[1]=7;[1,,3].find(v=>v===7)===7&&[1,,3].findLastIndex(v=>v===7)===1",
        "let x={},a=[x];a.find((v,k,o)=>{o[k]=7;return true})===x&&a[0]===7",
        "let x={},a=[x];a.findLast((v,k,o)=>{delete o[k];return true})===x&&!(0 in a)",
        "let n=0;[1,2,3].find(v=>{n++;return v===2});n===2",
        "let n=0;[1,2,3].findLastIndex(v=>{n++;return v===2});n===2",
        "[1].find(()=>({valueOf(){throw 7}}))===1&&[1].findLast(()=>Symbol())===1",
        "let a=[1,2,3],log='';a.find((v,k)=>{log+=k+':'+v+';';if(k===0){delete a[1];a[2]=7;a.push(9)}return false});log==='0:1;1:undefined;2:7;'",
        "let a=[1,2,3],log='';a.findLast((v,k)=>{log+=k+':'+v+';';if(k===2){a.length=0;a.push(9)}return false});log==='2:3;1:undefined;0:9;'",
        "let n=0;let o={length:2,get 0(){n++;this[1]=7;return 1}};Array.prototype.find.call(o,v=>v===7)===7&&n===1",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn callbacks_are_generic_and_do_not_use_overridable_call_hooks() {
    for method in ["find", "findIndex", "findLast", "findLastIndex"] {
        for body in [
            "let a=[7],t={};let ok=false;a.METHOD(function(v,k,o){'use strict';ok=this===t&&v===7&&k===0&&o===a&&arguments.length===3;return true},t);ok",
            "let ok=false;[7].METHOD(function(){'use strict';ok=this===undefined;return true});ok",
            "let ok=false;[7].METHOD(function(){'use strict';ok=this===null;return true},null);ok",
            "let ok=false;[7].METHOD(function(){'use strict';ok=this===42;return true},42);ok",
            "let ok=false;[7].METHOD(function(){ok=this===globalThis;return true});ok",
            "let ok=false;[7].METHOD(function(){ok=typeof this==='object'&&this.valueOf()===42;return true},42);ok",
            "let ok=false;Array.prototype.METHOD.call('a',function(v,k,o){ok=v==='a'&&k===0&&typeof o==='object'&&o.valueOf()==='a';return true});ok",
            "let n=0,f=()=>{n++;return true};f.call=()=>{throw 7};f.apply=f.call;[1].METHOD(f);n===1",
            "let n=0;Array.prototype.METHOD.call(false,()=>n++);n===0",
            "let n=0;Array.prototype.METHOD.call({get length(){n++;return 1},0:7},()=>true);n===1",
            "let n=0;try{Array.prototype.METHOD.call({get length(){n++;return 0}},null)}catch(e){n+=e instanceof TypeError?1:10}n===2",
            "let e={},ok=false;try{Array.prototype.METHOD.call({get length(){throw e}},null)}catch(x){ok=x===e}ok",
            "let e={},ok=false;try{[1].METHOD(()=>{throw e})}catch(x){ok=x===e}ok",
            "let n=0,e={},ok=false;try{Array.prototype.METHOD.call({length:1,get 0(){throw e}},()=>{n++;return true})}catch(x){ok=x===e}ok&&n===0",
            "let n=0;[1].METHOD(()=>[2].METHOD(v=>{n++;return true}));n===1",
            "let n=0;let f=Array.prototype.METHOD.bind([1],()=>{n++;return true});f();Reflect.apply(Array.prototype.METHOD,[1],[()=>{n++;return true}]);n===2",
        ] {
            let source = body.replace("METHOD", method);
            assert_eq!(eval(&source), Ok(Value::Boolean(true)), "{source}");
        }
        for body in [
            "Array.prototype.METHOD.call(null,()=>true)",
            "Array.prototype.METHOD.call(undefined,()=>true)",
            "Array.prototype.METHOD.call({length:Symbol()},()=>true)",
            "[].METHOD()",
            "[].METHOD({})",
            "[].METHOD(Symbol())",
        ] {
            let source = body.replace("METHOD", method);
            assert!(matches!(eval(&source), Err(Error::Type { .. })), "{source}");
        }
    }
}

#[test]
fn searches_use_full_53_bit_indices_without_allocating_index_lists() {
    for source in [
        "let o={length:4294967298,4294967297:7};Array.prototype.at.call(o,-1)===7&&Array.prototype.at.call(o,4294967297)===7",
        "let n=Number.MAX_SAFE_INTEGER,o={length:Infinity};o[n-1]=7;Array.prototype.at.call(o,-1)===7&&Array.prototype.at.call(o,n)===undefined&&Array.prototype.at.call(o,-n-1)===undefined",
        "let n=Number.MAX_SAFE_INTEGER,o={length:n};o[0]=7;Array.prototype.at.call(o,-n)===7",
        "let o={length:Infinity,0:7};Array.prototype.find.call(o,()=>true)===7&&Array.prototype.findIndex.call(o,()=>true)===0",
        "let n=Number.MAX_SAFE_INTEGER,o={length:Infinity};o[n-1]=7;let k;Array.prototype.findLast.call(o,(v,i)=>{k=i;return true})===7&&k===n-1",
        "let n=Number.MAX_SAFE_INTEGER,o={length:n};Array.prototype.findLastIndex.call(o,(v,i)=>i===n-2)===n-2",
        "let o={length:4294967298,4294967296:7};Array.prototype.findLast.call(o,v=>v===7)===7&&Array.prototype.findLastIndex.call(o,v=>v===7)===4294967296",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn search_values_and_callback_state_survive_gc_and_unwinding() -> Result<(), Error> {
    let limits = Limits {
        heap_entries: 180,
        ..Limits::default()
    };
    for source in [
        "[{},{}].find((v,k,o)=>{o.length=0;for(let i=0;i<300;i++){let x={}}v.n=7;return true}).n===7",
        "[{},{}].findLast((v,k,o)=>{o.length=0;for(let i=0;i<300;i++){let x={}}v.n=7;return true}).n===7",
        "Array.prototype.find.call({length:1,get 0(){return {n:7}}},function(v){for(let i=0;i<300;i++){let x={}}return this.n===v.n},{n:7}).n===7",
        "Array.prototype.at.call({length:1,get 0(){for(let i=0;i<300;i++){let x={}}return {n:7}}},0).n===7",
        "let n=0;try{[1].find(()=>{try{throw 7}finally{n++}})}catch(e){n+=e}[1].find(()=>true)===1&&n===8",
        "let n=0;try{[1].findLast(()=>{try{throw 7}finally{n++}})}catch(e){n+=e}[1].findLast(()=>true)===1&&n===8",
    ] {
        assert_eq!(
            Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
            Value::Boolean(true),
            "{source}"
        );
    }
    let mut host = SilentHost;
    let mut realm = crate::Realm::new(limits, &mut host)?;
    assert_eq!(
        realm.evaluate(
            "let log='';[1].find(async v=>{log+='a';await 0;log+='b';return false});log+='c';log",
        )?,
        Value::string("ac")
    );
    assert_eq!(realm.evaluate("log")?, Value::string("acb"));
    Ok(())
}

#[test]
fn searches_share_fuel_frames_and_fatal_realm_limits() -> Result<(), Error> {
    for method in ["find", "findIndex", "findLast", "findLastIndex"] {
        let source = format!(
            "try{{Array.prototype.{method}.call({{length:Infinity}},()=>false)}}catch(e){{throw 'caught'}}"
        );
        let limits = Limits {
            fuel: 3000,
            ..Limits::default()
        };
        let mut host = SilentHost;
        let mut realm = crate::Realm::new(limits, &mut host)?;
        assert!(matches!(
            realm.evaluate(&source),
            Err(Error::Limit {
                resource: "execution fuel"
            })
        ));
        assert!(realm.evaluate("42").is_err());
        let limits = Limits {
            call_frames: 12,
            ..Limits::default()
        };
        let source = format!("function f(){{[1].{method}(f)}}f()");
        assert!(matches!(
            Runtime::new(limits).run(&compile(&source, limits)?, &mut SilentHost),
            Err(Error::Limit {
                resource: "call frames"
            })
        ));
    }
    Ok(())
}

#[test]
fn search_method_properties_are_mutable_and_methods_are_not_constructors() {
    for method in ["at", "find", "findIndex", "findLast", "findLastIndex"] {
        let source = format!(
            "let f=Array.prototype.{method};let d=Object.getOwnPropertyDescriptor(Array.prototype,'{method}');let n=Object.getOwnPropertyDescriptor(f,'name'),l=Object.getOwnPropertyDescriptor(f,'length');let ok=f.name==='{method}'&&f.length===1&&d.writable&&!d.enumerable&&d.configurable&&!n.writable&&!n.enumerable&&n.configurable&&!l.writable&&!l.enumerable&&l.configurable&&f.prototype===undefined;f.extra=7;delete f.name;delete Array.prototype.{method};ok&&f.extra===7&&!Object.hasOwn(f,'name')&&!Object.hasOwn(Array.prototype,'{method}')"
        );
        assert_eq!(eval(&source), Ok(Value::Boolean(true)), "{source}");
        let source = format!("new Array.prototype.{method}()");
        assert!(matches!(eval(&source), Err(Error::Type { .. })), "{source}");
    }
}
