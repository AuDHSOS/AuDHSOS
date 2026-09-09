// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use super::{Error, Limits, Runtime, SilentHost, Value, compile, eval};

#[test]
fn array_constructor_metadata_storage_and_integrity_levels() {
    for source in [
        "Array.name==='Array'&&Array.length===1&&Object.getPrototypeOf(Array)===Function.prototype&&Array.prototype.constructor===Array",
        "let d=Object.getOwnPropertyDescriptor(Array,'prototype');d.value===Array.prototype&&!d.writable&&!d.enumerable&&!d.configurable",
        "let l=Object.getOwnPropertyDescriptor(Array,'length'),n=Object.getOwnPropertyDescriptor(Array,'name');!l.writable&&!l.enumerable&&l.configurable&&!n.writable&&!n.enumerable&&n.configurable",
        "Object.getOwnPropertyNames(Array).slice(0,3).join()==='length,name,prototype'&&Object.keys(Array).length===0",
        "Array.print=print;Array.saved={n:7};Array.print===print&&Array.saved.n===7&&Object.keys(Array).join()==='print,saved'",
        "let old=Array.of;Array.of=7;let ok=Array.of===7;delete Array.of;ok&&Array.of===undefined&&old(7)[0]===7",
        "let n=0;Object.defineProperty(Array,'of',{get(){n++;return 7}});Array.of===7&&n===1",
        "let s=Symbol();Array[s]={n:7};Object.getOwnPropertySymbols(Array).includes(s)&&Array[s].n===7&&delete Array[s]&&Array[s]===undefined",
        "let old=Array.prototype;Array.prototype={};Array.prototype===old&&Object.getPrototypeOf([])===old",
        "delete Array.name;delete Array.length;!Object.hasOwn(Array,'name')&&!Object.hasOwn(Array,'length')&&Array(2).length===2",
        "Object.defineProperty(Array,'name',{value:'Custom'});Array.name==='Custom'&&Array(1,2).join()==='1,2'",
        "let p={x:7};Object.setPrototypeOf(Array,p);Object.getPrototypeOf(Array)===p&&Array.x===7&&Array.of(1)[0]===1&&Object.getPrototypeOf([])===Array.prototype",
        "Object.freeze(Array);let f=Array.of;Array.of=7;Array.extra=1;Object.isFrozen(Array)&&Array.of===f&&!Object.hasOwn(Array,'extra')&&new Array(2).length===2",
        "Object.seal(Array);let f=Array.of;Array.of=7;Object.isSealed(Array)&&!Object.isFrozen(Array)&&Array.of===7&&f(1)[0]===1",
        "Object.preventExtensions(Array);Array.extra=1;!Object.isExtensible(Array)&&!Object.hasOwn(Array,'extra')&&Array.isArray([])",
        "Object.prototype.toString.call(Array)==='[object Function]'&&typeof Array==='function'&&Array instanceof Function",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "Object.defineProperty(Array,'prototype',{value:{}})",
        "Object.defineProperty(Array,'prototype',{writable:true})",
        "Object.freeze(Array);Object.defineProperty(Array,'of',{value:7})",
        "Object.preventExtensions(Array);Object.defineProperty(Array,'extra',{value:1})",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn array_species_is_a_real_mutable_accessor_with_inherited_receiver() {
    for source in [
        "let d=Object.getOwnPropertyDescriptor(Array,Symbol.species);d.get.name==='get [Symbol.species]'&&d.get.length===0&&d.set===undefined&&!d.enumerable&&d.configurable&&d.get.call(7)===7&&d.get.call(null)===null",
        "Array[Symbol.species]===Array&&Object.getOwnPropertySymbols(Array).length===1",
        "class A extends Array{}A[Symbol.species]===A&&new A(1,2).map(v=>v) instanceof A",
        "let n=0;Object.defineProperty(Array,Symbol.species,{get(){n++;return Array}});let a=[1].map(v=>v);n===1&&Array.isArray(a)&&a[0]===1",
        "let seen;Object.defineProperty(Array,Symbol.species,{get(){seen=this;return Array}});class A extends Array{}let a=new A(1,2).slice();seen===A&&!(a instanceof A)&&a.join()==='1,2'",
        "delete Array[Symbol.species];Array[Symbol.species]===undefined&&!Object.hasOwn(Array,Symbol.species)&&Array.isArray([1].map(v=>v))",
        "delete Array[Symbol.species];class A extends Array{}let a=new A(1,2).filter(()=>true);!(a instanceof A)&&Array.isArray(a)&&a.join()==='1,2'",
        "let d=Object.getOwnPropertyDescriptor(Array,Symbol.species),g=d.get;g.extra=7;delete g.name;g.call(Array)===Array&&g.extra===7&&!Object.hasOwn(g,'name')",
        "Object.defineProperty(Array,Symbol.species,{value:null});let a=[1];Array.isArray(a.map(v=>v))&&Array.isArray(a.filter(()=>true))&&Array.isArray(a.slice())&&Array.isArray(a.concat())&&Array.isArray(a.splice())",
        "Object.defineProperty(Array,Symbol.species,{get(){throw 7}});Array.of(1)[0]===1&&Array(2).length===2&&Array.isArray([])",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for method in [
        "map(v=>v)",
        "filter(()=>true)",
        "slice()",
        "concat()",
        "splice()",
    ] {
        let source = format!(
            "let error={{}},ok=false;Object.defineProperty(Array,Symbol.species,{{get(){{throw error}}}});try{{[1].{method}}}catch(e){{ok=e===error}}ok"
        );
        assert_eq!(eval(&source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "new (Object.getOwnPropertyDescriptor(Array,Symbol.species).get)()",
        "Object.defineProperty(Array,Symbol.species,{value:7});[1].slice()",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn array_of_is_generic_constructs_once_and_never_uses_species() {
    for source in [
        "Array.of().length===0&&Array.of(7).length===1&&Array.of(7)[0]===7&&Array.of(undefined).length===1&&Object.hasOwn(Array.of(undefined),0)",
        "let f=Array.of;f(1,2).join()==='1,2'&&Array.isArray(f.call(null,7))&&Array.isArray(f.call({},7))&&Array.isArray(f.call(Symbol(),7))",
        "class A extends Array{}let a=A.of(1,2);a instanceof A&&a.length===2&&a.join()==='1,2'",
        "let called=0,target,args;function C(n){called++;target=new.target;args=arguments;this.size=n}let a=Array.of.call(C,1,2);a instanceof C&&!Array.isArray(a)&&called===1&&target===C&&args.length===1&&args[0]===2&&a.size===2&&a[0]===1&&a[1]===2&&a.length===2",
        "function C(n){this.n=n}let bound=C.bind(null);let a=Array.of.call(bound,7);a instanceof C&&a.n===1&&a[0]===7&&a.length===1",
        "let o={get prototype(){throw 7}};Array.isArray(Array.of.call(o,7))&&Array.isArray(Array.of.call(()=>{throw 8},7))",
        "let f=()=>{throw 7};f.prototype={};Array.of.call(f,7)[0]===7&&Array.of.call(f.bind(null),8)[0]===8",
        "function C(n){this.n=n}Object.defineProperty(C,Symbol.species,{get(){throw 7}});Array.of.call(C,1).n===1",
        "let seen=-1;function C(n){seen=n;return {}}let a=Array.of.call(C);a.length===0&&seen===0",
        "let x={valueOf(){throw 7},toString(){throw 8}};Array.of.call(null,x)[0]===x",
        "function C(){return [9,8,7]}let a=Array.of.call(C,1);a.length===1&&a[0]===1&&!(1 in a)",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn array_of_property_order_and_partial_changes_on_errors() {
    for source in [
        "let log='',out={set 0(v){throw 7},set length(v){log+='l'+v}},C=function(n){log+='c'+n;return out};Array.of.call(C,7,8)===out&&out[0]===7&&out[1]===8&&log==='c2l2'",
        "let log='',out={set length(n){log+=this[0]+':'+this[1]+':'+n}},C=function(){return out};Array.of.call(C,7,8);log==='7:8:2'",
        "let e={},ok=false;function C(){throw e}try{Array.of.call(C,1)}catch(x){ok=x===e}ok",
        "let out={};Object.defineProperty(out,1,{value:9,writable:false});function C(){return out}let ok=false;try{Array.of.call(C,7,8)}catch(e){ok=e instanceof TypeError}ok&&out[0]===7&&out[1]===9&&!Object.hasOwn(out,'length')",
        "let out={};Object.defineProperty(out,'length',{value:0,writable:false});function C(){return out}let ok=false;try{Array.of.call(C,7,8)}catch(e){ok=e instanceof TypeError}ok&&out[0]===7&&out[1]===8&&out.length===0",
        "let out=function(){};function C(){return out}let ok=false;try{Array.of.call(C,7)}catch(e){ok=e instanceof TypeError}ok&&out[0]===7",
        "let out=Object.preventExtensions({});function C(){return out}let ok=false;try{Array.of.call(C,7)}catch(e){ok=e instanceof TypeError}ok&&!Object.hasOwn(out,0)",
        "let e={},out={set length(v){throw e}};function C(){return out}let ok=false;try{Array.of.call(C,7)}catch(x){ok=x===e}ok&&out[0]===7",
        "let log='',p={};function C(n){log+='c';this.n=n}let bound=C.bind(null);Object.defineProperty(bound,'prototype',{get(){throw 7}});let a=Array.of.call(bound,8);a.n===1&&a[0]===8&&log==='c'",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn array_static_methods_metadata_brands_and_mutation() {
    for (method, length) in [("of", 0), ("isArray", 1)] {
        let source = format!(
            "let f=Array.{method},d=Object.getOwnPropertyDescriptor(Array,'{method}'),l=Object.getOwnPropertyDescriptor(f,'length'),n=Object.getOwnPropertyDescriptor(f,'name');f.name==='{method}'&&f.length==={length}&&f.prototype===undefined&&d.writable&&!d.enumerable&&d.configurable&&!l.writable&&!l.enumerable&&l.configurable&&!n.writable&&!n.enumerable&&n.configurable"
        );
        assert_eq!(eval(&source), Ok(Value::Boolean(true)), "{source}");
        let source = format!(
            "let f=Array.{method};delete f.name;f.extra=7;delete Array.{method};!Object.hasOwn(f,'name')&&f.extra===7&&!Object.hasOwn(Array,'{method}')"
        );
        assert_eq!(eval(&source), Ok(Value::Boolean(true)), "{source}");
        assert!(matches!(
            eval(&format!("new Array.{method}()")),
            Err(Error::Type { .. })
        ));
    }
    for source in [
        "Array.isArray([])&&Array.isArray(Array.prototype)&&!Array.isArray({})&&!Array.isArray(Array)&&!Array.isArray(Object.create(Array.prototype))",
        "class A extends Array{}Array.isArray(new A())&&Array.isArray(new A(1,2))",
        "let o={get [Symbol.toStringTag](){throw 7},get length(){throw 8}};!Array.isArray(o)&&!Array.isArray(Symbol())&&!Array.isArray(null)&&!Array.isArray()",
        "let a=[];Object.setPrototypeOf(a,null);Array.isArray(a)",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn array_constructor_and_factory_roots_survive_collection() -> Result<(), Error> {
    let limits = Limits {
        heap_entries: 180,
        ..Limits::default()
    };
    for source in [
        "let key=Symbol();Array[key]={n:7};Array.saved={n:8};Array.of.saved={n:9};for(let i=0;i<300;i++){let g={}}Array[key].n===7&&Array.saved.n===8&&Array.of.saved.n===9",
        "function C(n){for(let i=0;i<300;i++){let g={}}return {}}let a=Array.of.call(C,{n:7},{n:8});a[0].n===7&&a[1].n===8",
        "let seen;function C(){return {set length(n){for(let i=0;i<300;i++){let g={}}seen=this[0].n}}}Array.of.call(C,{n:7});seen===7",
        "let n=0;Object.defineProperty(Array,Symbol.species,{get(){for(let i=0;i<300;i++){let g={}}return function(){n++;return {}}}});let a=[1].map(v=>v+1);n===1&&a[0]===2",
    ] {
        assert_eq!(
            Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
            Value::Boolean(true),
            "{source}"
        );
    }
    let mut host = SilentHost;
    let mut realm = crate::Realm::new(limits, &mut host)?;
    realm.evaluate(
        "let saved=Array.of;delete Array.of;Array.extra={n:7};delete Array[Symbol.species]",
    )?;
    assert_eq!(realm.evaluate("for(let i=0;i<300;i++){let g={}}!Object.hasOwn(Array,'of')&&!Object.hasOwn(Array,Symbol.species)&&Array.extra.n===7&&saved(1)[0]===1")?,Value::Boolean(true));
    Ok(())
}
