// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::{Error, Limits, Runtime, SilentHost, Value, compile, eval};

#[test]
fn weakmap_identity_updates_deletion_and_invalid_keys() {
    for source in [
        "let m=new WeakMap();let a={},b={};m.set(a,1).set(b,2);m.get(a)===1&&m.get(b)===2",
        "let m=new WeakMap();let a={};m.set(a,undefined);m.has(a)&&m.get(a)===undefined",
        "let m=new WeakMap();let a={};m.set(a,1);m.set(a,2);m.get(a)===2&&m.delete(a)&&!m.delete(a)&&!m.has(a)",
        "let m=new WeakMap();m.get({})===undefined&&!m.has({})&&!m.delete({})",
        "let m=new WeakMap();m.get(null)===undefined&&!m.has(1)&&!m.delete('x')",
        "let m=new WeakMap();let f=()=>1,b=f.bind(null);m.set(f,3).set(b,4).set(Array,5);m.get(f)===3&&m.get(b)===4&&m.get(Array)===5",
        "let m=new WeakMap();let p=Promise.withResolvers();m.set(p.resolve,1).set(p.reject,2);m.get(p.resolve)===1&&m.get(p.reject)===2",
        "let m=new WeakMap();Object.freeze(m);let k=Object.freeze({});m.set(k,1);m.get(k)===1",
        "new WeakMap() instanceof WeakMap && new WeakMap(null) instanceof WeakMap",
        "Object.getPrototypeOf(WeakMap.prototype)===Object.prototype&&WeakMap.prototype.constructor===WeakMap",
        "let d=Object.getOwnPropertyDescriptor(WeakMap,'prototype');!d.writable&&!d.enumerable&&!d.configurable",
        "let d=Object.getOwnPropertyDescriptor(WeakMap.prototype,'set');d.writable&&!d.enumerable&&d.configurable",
        "let d=Object.getOwnPropertyDescriptor(WeakMap.prototype.set,'length');d.value===2&&!d.writable&&!d.enumerable&&d.configurable",
        "WeakMap.length===0&&WeakMap.name==='WeakMap'&&WeakMap.prototype.get.name==='get'",
        "WeakMap.prototype.get.length===1&&WeakMap.prototype.has.length===1&&WeakMap.prototype.delete.length===1",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "WeakMap()",
        "new WeakMap().set(1,2)",
        "WeakMap.prototype.get({})",
        "WeakMap.prototype.set.call({}, {}, 1)",
        "WeakMap.prototype.has.call(null,{})",
        "WeakMap.prototype.delete.call(undefined,{})",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn weakmap_constructor_reads_adder_and_entries_in_order() {
    for (source, expected) in [
        (
            "let k={};new WeakMap([[k,1],[k,2]]).get(k)",
            Value::Number(2.0),
        ),
        (
            "let k={};let log='';let e={get 0(){log+='k';return k},get 1(){log+='v';return 3}};new WeakMap([e]);log",
            Value::string("kv"),
        ),
        (
            "let log='';let set=WeakMap.prototype.set;Object.defineProperty(WeakMap.prototype,'set',{get(){log+='g';return function(k,v){log+='s';return set.call(this,k,v)}}});new WeakMap([[{},1],[{},2]]);log",
            Value::string("gss"),
        ),
        (
            "let k={},a=[[k,1]];WeakMap.prototype.set=function(){a.push([{},2]);throw 7};try{new WeakMap(a)}catch(e){e}",
            Value::Number(7.0),
        ),
        (
            "Object.defineProperty(WeakMap.prototype,'set',{get(){throw 7}});new WeakMap(null) instanceof WeakMap",
            Value::Boolean(true),
        ),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
    for source in [
        "new WeakMap([1])",
        "new WeakMap([['key',1]])",
        "new WeakMap({})",
        "WeakMap.prototype.set=3;new WeakMap([])",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn weakmap_get_or_insert_preserves_undefined_and_rechecks_after_callback() {
    for source in [
        "let m=new WeakMap(),k={};m.getOrInsert(k,1)===1&&m.getOrInsert(k,2)===1",
        "let m=new WeakMap(),k={};m.set(k,undefined);m.getOrInsertComputed(k,()=>{throw 1})===undefined",
        "let m=new WeakMap(),k={};let v=m.getOrInsertComputed(k,x=>{m.set(x,2);return 3});v===3&&m.get(k)===3",
        "let m=new WeakMap(),k={};let v=m.getOrInsertComputed(k,function(x){'use strict';return this===undefined&&x===k});v===true",
        "let m=new WeakMap(),k={};try{m.getOrInsertComputed(k,()=>{throw 9})}catch(e){}!m.has(k)",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "new WeakMap().getOrInsert(null,1)",
        "new WeakMap().getOrInsertComputed(null,()=>1)",
        "let m=new WeakMap(),k={};m.set(k,1);m.getOrInsertComputed(k,3)",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn weakmap_gc_collects_unreachable_key_value_cycles_and_releases_quota() -> Result<(), Error> {
    let limits = Limits {
        heap_entries: 70,
        weak_entries: 2,
        ..Limits::default()
    };
    for source in [
        "let m=new WeakMap();for(let i=0;i<300;i++){let k={};m.set(k,{k});}true",
        "for(let i=0;i<300;i++){let m=new WeakMap(),k={};m.set(k,{m,k});}true",
        "let m=new WeakMap(),n=new WeakMap();for(let i=0;i<200;i++){let a={},b={};m.set(a,b);n.set(b,a);}true",
    ] {
        assert_eq!(
            Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
            Value::Boolean(true),
            "{source}"
        );
    }
    let source = "let m=new WeakMap(),k={},v={n:7};m.set(k,v);v=null;for(let i=0;i<200;i++){let t={}}m.get(k).n";
    assert_eq!(
        Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
        Value::Number(7.0)
    );
    let source =
        "let m=new WeakMap();m.set(Array,{n:7});for(let i=0;i<200;i++){let t={}}m.get(Array).n";
    assert_eq!(
        Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
        Value::Number(7.0)
    );
    let source = "let m=new WeakMap(),a={},b={},c={};m.set(a,1).set(b,2).set(c,3)";
    assert!(matches!(
        Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost),
        Err(Error::Limit { .. })
    ));
    Ok(())
}

#[test]
fn weakmap_callbacks_keep_pending_keys_values_and_maps_rooted() -> Result<(), Error> {
    let limits = Limits {
        heap_entries: 80,
        weak_entries: 2,
        ..Limits::default()
    };
    for (source, expected) in [
        (
            "let m=new WeakMap(),k={};m.getOrInsertComputed(k,()=>{for(let i=0;i<100;i++){let a={}}return {n:5}});m.get(k).n",
            5.0,
        ),
        (
            "let m=new WeakMap(),a={},b={};m.set(a,{next:b}).set(b,{n:8});b=null;for(let i=0;i<100;i++){let t={}}m.get(m.get(a).next).n",
            8.0,
        ),
        (
            "let k={};let e={get 0(){return k},get 1(){for(let i=0;i<100;i++){let t={}}return {n:9}}};new WeakMap([e]).get(k).n",
            9.0,
        ),
    ] {
        assert_eq!(
            Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
            Value::Number(expected),
            "{source}"
        );
    }
    Ok(())
}
