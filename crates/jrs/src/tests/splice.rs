// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use super::{Error, Limits, Runtime, SilentHost, Value, compile, eval};

#[test]
fn splice_insertion_deletion_replacement_and_relative_indices() {
    for source in [
        "let a=[1,2,3];let d=a.splice();d.length===0&&a.join()==='1,2,3'",
        "let a=[1,2,3];let d=a.splice(undefined);d.join()==='1,2,3'&&a.length===0",
        "let a=[1,2,3];let d=a.splice(1,undefined,7);d.length===0&&a.join()==='1,7,2,3'",
        "let a=[1,2,3];let d=a.splice(1);d.join()==='2,3'&&a.join()==='1'",
        "let a=[1,2,3];let d=a.splice(1,1,7,8);d.join()==='2'&&a.join()==='1,7,8,3'",
        "let a=[1,2,3];let d=a.splice(0,2,7);d.join()==='1,2'&&a.join()==='7,3'",
        "let a=[1,2,3];let d=a.splice(-1,1,7);d.join()==='3'&&a.join()==='1,2,7'",
        "let a=[1,2,3];let d=a.splice(1.9,1.9);d.join()==='2'&&a.join()==='1,3'",
        "let a=[1,2];a.splice(-0.5,-1,7);a.join()==='7,1,2'",
        "let a=[1,2];a.splice(NaN,NaN,7);a.join()==='7,1,2'",
        "let a=[1,2];a.splice(-Infinity,Infinity,7);a.join()==='7'",
        "let a=[1,2];a.splice(Infinity,-Infinity,7);a.join()==='1,2,7'",
        "let a=[1,2];a.splice(-10,1);a.join()==='2'",
        "let a=[1,2];a.splice(10,20,7);a.join()==='1,2,7'",
        "let a=[1,2],x={valueOf(){throw 7}};a.splice(1,1,x)[0]===2&&a[1]===x",
        "let o={0:'a',2:'c',length:3};let d=Array.prototype.splice.call(o,0,1,'x','y');d[0]==='a'&&o[0]==='x'&&o[1]==='y'&&!(2 in o)&&o[3]==='c'&&o.length===4",
        "let o={length:-7};let d=Array.prototype.splice.call(o,0,0,7);d.length===0&&o.length===1&&o[0]===7",
        "let o={0:1,1:2,length:'2.9'};let d=Array.prototype.splice.call(o,0,1);d[0]===1&&o[0]===2&&o.length===1",
        "let p;Object.defineProperty(Boolean.prototype,'length',{set(v){p=this}});let d=Array.prototype.splice.call(true);d.length===0&&p.valueOf()===true",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn splice_preserves_holes_and_rechecks_inherited_properties() {
    for source in [
        "let a=[1,,3,,5];let d=a.splice(1,3);d.length===3&&!(0 in d)&&d[1]===3&&!(2 in d)&&a.join()==='1,5'",
        "let a=[1,,3,,];a.splice(0,1);a.length===3&&!(0 in a)&&a[1]===3&&!(2 in a)",
        "let a=[1,,3];a.splice(0,0,7);a.length===4&&a[0]===7&&a[1]===1&&!(2 in a)&&a[3]===3",
        "Array.prototype[1]=7;let a=[1,,3];let d=a.splice(1,1);Object.hasOwn(d,0)&&d[0]===7&&a.join()==='1,3'",
        "let p={1:7},o=Object.create(p);o[0]=1;o.length=3;Array.prototype.splice.call(o,0,1);o[0]===7&&!Object.hasOwn(o,1)&&o[1]===7&&o.length===2",
        "let o={length:3,1:2,2:3,get 0(){delete this[1];return 7},set 0(v){}};let d=Array.prototype.splice.call(o,0,2,8,9);d[0]===7&&!(1 in d)",
        "let a=[1,,];Object.preventExtensions(a);let d=a.splice(0,1);d[0]===1&&a.length===1&&!(0 in a)",
        "let a=[1,,];Object.seal(a);let d=a.splice(1,1);d.length===1&&!(0 in d)&&a.length===1",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn species_length_metadata_fallback_and_aliasing() {
    for source in [
        "let seen;let a=[1,,3];a.constructor={[Symbol.species]:function(n){seen=n;return {}}};let d=a.splice(0,2);seen===2&&d.length===2&&d[0]===1&&!(1 in d)&&!Array.isArray(d)&&a.join()==='3'",
        "class A extends Array{}let a=new A(1,2,3);let d=a.splice(0,1);d instanceof A&&d.length===1&&d[0]===1&&a.join()==='2,3'",
        "let a=[1,2];a.constructor={[Symbol.species]:null};Array.isArray(a.splice(0,1))",
        "let a=[1,2];a.constructor={};Array.isArray(a.splice(0,1))",
        "let a=[1,2];a.constructor=undefined;Array.isArray(a.splice(0,1))",
        "let o={length:0,get constructor(){throw 7}};Array.isArray(Array.prototype.splice.call(o))",
        "let a=[1],n;function C(k){n=1/k}a.constructor={[Symbol.species]:C};a.splice();n===Infinity",
        "let log='',out={set length(n){log+='l'+n}};let a=[1,2];a.constructor={[Symbol.species]:function(n){log+='c'+n;return out}};let d=a.splice(0,1);d===out&&out[0]===1&&log==='c1l1'",
        "let calls=0,out={set 0(v){calls++}};let a=[1];a.constructor={[Symbol.species]:function(){return out}};a.splice(0,1);let d=Object.getOwnPropertyDescriptor(out,0);calls===0&&d.value===1&&d.writable&&d.enumerable&&d.configurable",
        "let a=[1,2,3];a.constructor={[Symbol.species]:function(){return a}};let d=a.splice(1,1);d===a&&a.length===2&&a[0]===2&&!(1 in a)",
        "let f=Array.prototype.splice;let d=Object.getOwnPropertyDescriptor(Array.prototype,'splice');f.name==='splice'&&f.length===2&&f.prototype===undefined&&d.writable&&!d.enumerable&&d.configurable",
        "let f=Array.prototype.splice;delete f.name;f.extra=7;delete Array.prototype.splice;f.call([1,2],0,1)[0]===1&&f.extra===7&&!Object.hasOwn(f,'name')&&!Object.hasOwn(Array.prototype,'splice')",
        "let d=Object.getOwnPropertyDescriptor(Array.prototype.splice,'length');!d.writable&&!d.enumerable&&d.configurable&&!Object.hasOwn(Array.prototype[Symbol.unscopables],'splice')",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "let a=[];a.constructor=null;a.splice()",
        "let a=[];a.constructor=7;a.splice()",
        "let a=[];a.constructor={[Symbol.species]:{}};a.splice()",
        "let a=[];a.constructor={[Symbol.species]:()=>({})};a.splice()",
        "new Array.prototype.splice()",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn splice_observable_order_and_partial_mutations_on_exceptions() {
    for source in [
        "let log='',a=[1,2];Object.defineProperty(a,'constructor',{get(){log+='c';return {get [Symbol.species](){log+='s';return function(n){log+='n'+n;return []}}}}});a.splice({valueOf(){log+='a';return 0}},{valueOf(){log+='d';return 1}});log==='adcsn1'",
        "let log='',o={get length(){log+='l';return {valueOf(){log+='n';return 0}}},set length(v){log+='w'}};Array.prototype.splice.call(o,{valueOf(){log+='a';return 0}},{valueOf(){log+='d';return 0}});log==='lnadw'",
        "let log='',o={length:3,get 1(){log+='b';return 2},get 2(){log+='c';return 3},set 0(v){log+='A'+v},set 1(v){log+='B'+v}};Array.prototype.splice.call(o,0,1);log==='bA2cB3'",
        "let log='',o={length:2,get 0(){log+='a';return 1},set 0(v){log+='A'+v},get 1(){log+='b';return 2},set 1(v){log+='B'+v},set 2(v){log+='C'+v}};Array.prototype.splice.call(o,0,0,7);log==='bC2aB1A7'",
        "let a=[1,2,3];Object.defineProperty(a,1,{writable:false});let ok=false;try{a.splice(0,1)}catch(e){ok=e instanceof TypeError}ok&&a[0]===2&&a[1]===2&&a[2]===3&&a.length===3",
        "let a=[1,2,3];Object.defineProperty(a,1,{configurable:false});try{a.splice(0,3)}catch(e){}a.length===3&&a[0]===1&&a[1]===2&&!(2 in a)",
        "let a=[1,2];Object.defineProperty(a,1,{writable:false});try{a.splice(0,0,7)}catch(e){}a.length===3&&a.join()==='1,2,2'",
        "let a=[1,2];Object.defineProperty(a,'length',{writable:false});try{a.splice(0,1,7)}catch(e){}a.join()==='7,2'",
        "let o={length:2,1:7};Object.defineProperty(o,0,{configurable:false,value:1});try{Array.prototype.splice.call(o,0,0,9)}catch(e){}o[2]===7&&o[1]===1&&o[0]===1&&o.length===2",
        "let a=[1,2],out={};Object.defineProperty(out,'length',{value:0,writable:false});a.constructor={[Symbol.species]:function(){return out}};try{a.splice(0,1)}catch(e){}a.join()==='1,2'&&out[0]===1",
        "let a=[1,2];a.constructor={[Symbol.species]:function(){return Object.preventExtensions({})}};try{a.splice(0,1)}catch(e){}a.join()==='1,2'",
        "let a=[1,2,3];let d=a.splice({valueOf(){a.length=1;return 1}},1);d.length===1&&!(0 in d)&&a.length===2&&a[0]===1&&!(1 in a)",
        "let a=[1,2,3];a.constructor={[Symbol.species]:function(n){a[1]=7;return []}};a.splice(1,1)[0]===7",
        "let n=0;try{Array.prototype.splice.call(null,{valueOf(){n++}})}catch(e){}n===0",
        "let n=0;try{[].splice({valueOf(){throw 7}},{valueOf(){n++}})}catch(e){}n===0",
        "let e={},ok=false;try{Array.prototype.splice.call({get length(){throw e}},0)}catch(x){ok=x===e}ok",
        "let e={},a=[],ok=false;Object.defineProperty(a,'constructor',{get(){throw e}});try{a.splice()}catch(x){ok=x===e}ok",
        "let e={},a=[],ok=false;a.constructor={get [Symbol.species](){throw e}};try{a.splice()}catch(x){ok=x===e}ok",
        "let e={},a=[],ok=false;a.constructor={[Symbol.species]:function(){throw e}};try{a.splice()}catch(x){ok=x===e}ok",
        "let e={},ok=false;try{Array.prototype.splice.call({length:1,get 0(){throw e}},0,1)}catch(x){ok=x===e}ok",
        "let e={},ok=false;try{[].splice(0,{valueOf(){throw e}})}catch(x){ok=x===e}ok",
        "let e={},ok=false;try{Array.prototype.splice.call({length:{valueOf(){throw e}}},0)}catch(x){ok=x===e}ok",
        "let e={},n=0,o={length:3,1:7,get 2(){throw e},set 0(v){n=v}},ok=false;try{Array.prototype.splice.call(o,0,1)}catch(x){ok=x===e}ok&&n===7",
        "let a=[1,2,3];Object.defineProperty(a,1,{configurable:false});delete a[2];try{a.splice(0,1)}catch(e){}a[0]===2&&a[1]===2&&a.length===3",
        "let n=0,o={length:0,set 0(v){n=v},set 1(v){throw 7}};try{Array.prototype.splice.call(o,0,0,8,9)}catch(e){}n===8&&o.length===0",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "Array.prototype.splice.call(null)",
        "Array.prototype.splice.call(undefined)",
        "Array.prototype.splice.call('')",
        "Array.prototype.splice.call('abc',1,1)",
        "[].splice(Symbol())",
        "[].splice(0,Symbol())",
        "Array.prototype.splice.call({length:Symbol()},0)",
        "Object.freeze([]).splice()",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn splice_wide_lengths_bounds_and_result_creation_errors() {
    for source in [
        "let n=Number.MAX_SAFE_INTEGER,o={length:n-1};o[n-2]=7;let d=Array.prototype.splice.call(o,n-2,0,9);o.length===n&&o[n-2]===9&&o[n-1]===7&&d.length===0",
        "let n=Number.MAX_SAFE_INTEGER,o={length:Infinity};o[n-3]=1;o[n-2]=2;o[n-1]=3;let d=Array.prototype.splice.call(o,-3,2);d.join()==='1,2'&&o[n-3]===3&&!(n-2 in o)&&!(n-1 in o)&&o.length===n-2",
        "let o={length:4294967298,4294967296:7};let d=Array.prototype.splice.call(o,4294967296,1,9);d[0]===7&&o[4294967296]===9&&o.length===4294967298&&!(0 in o)",
        "let n=0,o={get length(){return Infinity},set length(v){n++},get constructor(){throw 7}};let ok=false;try{Array.prototype.splice.call(o,0)}catch(e){ok=e instanceof RangeError}ok&&n===0",
        "let n=0,o={length:Infinity,get 0(){n++;return 7}};let ok=false;try{Array.prototype.splice.call(o,0,0,1)}catch(e){ok=e instanceof TypeError}ok&&n===0",
        "let o={length:Infinity};let d=Array.prototype.splice.call(o,Infinity,0);d.length===0&&o.length===Number.MAX_SAFE_INTEGER",
        "let a=[];a.length=4294967295;let ok=false;try{a.splice(4294967295,0,7)}catch(e){ok=e instanceof RangeError}ok&&a[4294967295]===7&&a.length===4294967295",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn splice_values_survive_gc_and_huge_loops_are_fuel_bounded() -> Result<(), Error> {
    let limits = Limits {
        heap_entries: 180,
        ..Limits::default()
    };
    for source in [
        "let a=[{n:7},2];a.constructor={[Symbol.species]:function(n){for(let i=0;i<300;i++){let g={}}return []}};a.splice(0,1)[0].n===7",
        "let out,seen;let a=[{n:7},2];a.constructor={[Symbol.species]:function(){out={set length(v){for(let i=0;i<300;i++){let g={}}seen=this[0].n}};return out}};a.splice(0,1);seen===7&&out[0].n===7",
        "let o={length:2,get 1(){return {n:7}},set 0(v){for(let i=0;i<300;i++){let g={}}this.saved=v}};Array.prototype.splice.call(o,0,1);o.saved.n===7",
        "let o={length:0,set 0(v){for(let i=0;i<300;i++){let g={}}this.saved=v}};Array.prototype.splice.call(o,0,0,{n:7});o.saved.n===7",
    ] {
        assert_eq!(
            Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
            Value::Boolean(true),
            "{source}"
        );
    }
    for source in [
        "Array.prototype.splice.call({length:9007199254740990},0,0,1)",
        "Array.prototype.splice.call({length:Infinity},0,1)",
        "Array(4294967295).splice(0)",
        "Array.prototype.splice.call({length:Infinity},0,4294967295)",
    ] {
        let limits = Limits {
            fuel: 3000,
            ..Limits::default()
        };
        assert!(
            matches!(
                Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost),
                Err(Error::Limit {
                    resource: "execution fuel"
                })
            ),
            "{source}"
        );
    }
    let source = "let a=[1];a.constructor={[Symbol.species]:function(){return a.splice()}};try{a.splice()}catch(e){throw 'caught'}";
    assert!(matches!(
        Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost),
        Err(Error::Limit { .. })
    ));
    Ok(())
}
