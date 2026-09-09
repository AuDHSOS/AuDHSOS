// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use super::{Error, Limits, Runtime, SilentHost, Value, compile, eval};

#[test]
fn array_from_iterables_array_likes_and_holes() {
    for source in [
        "Array.from([1,2,3]).join()==='1,2,3'&&Array.from('a😀b').join('|')==='a|😀|b'",
        "let a=Array.from([,undefined,,]);a.length===3&&Object.keys(a).join()==='0,1,2'",
        "let a=Array.from({length:3,1:7});a.length===3&&Object.keys(a).join()==='0,1,2'&&a[0]===undefined&&a[1]===7",
        "Array.from({length:-1}).length===0&&Array.from(42).length===0&&Array.from(Symbol()).length===0",
        "let a=[1,2];a[Symbol.iterator]=null;Array.from(a).join()==='1,2'",
        "let a=[1,2,3];a[Symbol.iterator]=function(){let n=0;return {next(){return n++===0?{value:7,done:false}:{done:true}}}};Array.from(a).join()==='7'",
        "let n=0,a={length:2,0:1,get 1(){n++;return 2}};Array.from(a,v=>v*2).join()==='2,4'&&n===1",
        "let a=[1,2];Array.from(a,(v,k)=>{if(k===0)a.push(3);return v}).join()==='1,2,3'",
        "let a={length:2,0:1,1:2};Array.from(a,(v,k)=>{a.length=1;if(k===0){a[1]=7;a[2]=9}return v}).join()==='1,7'",
        "let n=0;let a=Array.from({length:2},function(v,k){'use strict';if(this!==7||arguments.length!==2||v!==undefined)throw 1;n+=k;return k},7);a.join()==='0,1'&&n===1",
        "let f=v=>v+1;f.call=()=>{throw 7};Array.from([1],f)[0]===2",
        "let result=Array.from([1],async v=>v+1);result[0] instanceof Promise",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "Array.from(null)",
        "Array.from(undefined)",
        "Array.from([],null)",
        "Array.from({},7)",
        "Array.from({[Symbol.iterator]:7})",
        "Array.from({[Symbol.iterator](){return 7}})",
        "Array.from({[Symbol.iterator](){return {next:7}}})",
        "Array.from({[Symbol.iterator](){return {next(){return 7}}}})",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn array_from_constructor_and_iterator_getter_order() {
    for source in [
        "let log='',iterator={get next(){log+='n';return function(){log+='s';return {done:true,get value(){throw 7}}}}};let items={get [Symbol.iterator](){log+='i';return function(){log+='m';return iterator}}};function C(){log+='c';if(arguments.length!==0)throw 7;return {set length(v){log+='l'+v}}}Array.from.call(C,items);log==='icmnsl0'",
        "let log='',items={get [Symbol.iterator](){log+='i'},get length(){log+='l';return {valueOf(){log+='v';return 1}}},get 0(){log+='g';return 7}},C=function(n){log+='c'+n;return {set length(v){log+='s'+v}}};Array.from.call(C,items,v=>{log+='m';return v});log==='ilvc1gms1'",
        "let n=0,items={get [Symbol.iterator](){n++;throw 7}};let ok=false;try{Array.from(items,null)}catch(e){ok=e instanceof TypeError}ok&&n===0",
        "let n=0,items={get [Symbol.iterator](){n++;return function(){return {next(){return {done:true}}}}}};function C(){delete items[Symbol.iterator];return {}}Array.from.call(C,items).length===0&&n===1",
        "let n=0,it={get next(){n++;return function(){return {done:true}}}},items={[Symbol.iterator](){return it}};Array.from(items).length===0&&n===1",
        "let iterator={next(){this.next=()=>{throw 7};return {done:!this.first,value:this.first=false}}};iterator.first=true;Array.from({[Symbol.iterator](){return iterator}}).length===1",
        "class A extends Array{}let a=A.from([1,2]),b=A.from({length:1,0:7});a instanceof A&&b instanceof A&&a.join()==='1,2'&&b[0]===7",
        "let C=Array.bind(null);Array.from.call(C,[7])[0]===7&&Array.from.call(C,{length:1,0:8})[0]===8",
        "Object.defineProperty(Array,Symbol.species,{get(){throw 7}});Array.from([1])[0]===1&&Array.from({length:1,0:2})[0]===2",
        "let fn=()=>{throw 7};fn.prototype={};Array.from.call(fn,[1])[0]===1&&Array.from.call(fn,{length:1,0:2})[0]===2",
        "let items={length:1,0:7,get constructor(){throw 7}};Array.from(items)[0]===7",
        "let seen;function C(n){seen=arguments.length;this.n=n}let a=Array.from.call(C,[1]);a instanceof C&&a[0]===1&&a.length===1&&seen===0",
        "let seen;function C(n){seen=arguments.length;this.n=n}let a=Array.from.call(C,{length:1,0:7});a instanceof C&&a[0]===7&&a.n===1&&seen===1",
        "let e={},n=0,items={[Symbol.iterator](){n++;return {}}},ok=false;function C(){throw e}try{Array.from.call(C,items)}catch(x){ok=x===e}ok&&n===0",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn array_from_closes_only_mapper_or_definition_failures() {
    for source in [
        "let n=0,e={},it={next(){return {value:7,done:false}},return(){n++;return {}}},ok=false;try{Array.from({[Symbol.iterator](){return it}},()=>{throw e})}catch(x){ok=x===e}ok&&n===1",
        "let n=0,e={},it={next(){return {value:7,done:false}},get return(){n++;throw 8}},ok=false;try{Array.from({[Symbol.iterator](){return it}},()=>{throw e})}catch(x){ok=x===e}ok&&n===1",
        "let n=0,it={next(){return {value:7,done:false}},return(){n++;return 7}},ok=false;try{Array.from({[Symbol.iterator](){return it}},()=>{throw 8})}catch(x){ok=x===8}ok&&n===1",
        "let n=0,e={},it={next(){throw e},return(){n++;return {}}},ok=false;try{Array.from({[Symbol.iterator](){return it}})}catch(x){ok=x===e}ok&&n===0",
        "let n=0,e={},it={next(){return {get done(){throw e}}},return(){n++;return {}}},ok=false;try{Array.from({[Symbol.iterator](){return it}})}catch(x){ok=x===e}ok&&n===0",
        "let n=0,e={},it={next(){return {done:false,get value(){throw e}}},return(){n++;return {}}},ok=false;try{Array.from({[Symbol.iterator](){return it}})}catch(x){ok=x===e}ok&&n===0",
        "let n=0,it={next(){return {done:true,get value(){throw 7}}},return(){n++;return {}}};Array.from({[Symbol.iterator](){return it}}).length===0&&n===0",
        "let n=0,it={next(){return {value:7,done:false}},return(){n++;return {}}};function C(){return Object.preventExtensions({})}let ok=false;try{Array.from.call(C,{[Symbol.iterator](){return it}})}catch(e){ok=e instanceof TypeError}ok&&n===1",
        "let n=0,it={next(){return {done:true}},return(){n++;return {}}};function C(){return Object.freeze({length:0})}let ok=false;try{Array.from.call(C,{[Symbol.iterator](){return it}})}catch(e){ok=e instanceof TypeError}ok&&n===0",
        "let n=0,it={get next(){throw 7},return(){n++;return {}}};try{Array.from({[Symbol.iterator](){return it}})}catch(e){}n===0",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn array_from_ordinary_writes_and_wide_lengths() {
    for source in [
        "let log='',out={set 0(v){throw 7},set length(v){log+=v}};function C(){return out}Array.from.call(C,[7])===out&&out[0]===7&&log==='1'",
        "let out={};Object.defineProperty(out,1,{value:9});function C(){return out}let ok=false;try{Array.from.call(C,{length:2,0:7,1:8})}catch(e){ok=e instanceof TypeError}ok&&out[0]===7&&out[1]===9&&!Object.hasOwn(out,'length')",
        "let n=0;function C(length){n=length;throw 7}try{Array.from.call(C,{length:Infinity})}catch(e){}n===Number.MAX_SAFE_INTEGER",
        "let n=0,items={length:Infinity,get 0(){n++;return 7}};let ok=false;try{Array.from(items)}catch(e){ok=e instanceof RangeError}ok&&n===0",
        "let e={},n=0;function C(){return {}}let items={length:Infinity,get 0(){n++;throw e}},ok=false;try{Array.from.call(C,items)}catch(x){ok=x===e}ok&&n===1",
        "let items={length:1,0:7};function C(){items.length=5;items[0]=8;return {}}let a=Array.from.call(C,items);a[0]===8&&a.length===1&&!(1 in a)",
        "let f=Array.from,d=Object.getOwnPropertyDescriptor(Array,'from');f.name==='from'&&f.length===1&&f.prototype===undefined&&d.writable&&!d.enumerable&&d.configurable",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    assert!(matches!(
        eval("new Array.from([])"),
        Err(Error::Type { .. })
    ));
}

#[test]
fn array_from_gc_and_fatal_fuel_remain_bounded() -> Result<(), Error> {
    let limits = Limits {
        heap_entries: 180,
        ..Limits::default()
    };
    for source in [
        "let a=Array.from([{n:7},{n:8}],v=>{for(let i=0;i<300;i++){let g={}}return v});a[0].n===7&&a[1].n===8",
        "let it={next(){return {value:{n:7},done:false}},return(){for(let i=0;i<300;i++){let g={}}return {}}},e;try{Array.from({[Symbol.iterator](){return it}},()=>{throw {n:9}})}catch(x){e=x}e.n===9",
        "let items={get [Symbol.iterator](){return function(){return {next(){return {done:true}}}}}};function C(){for(let i=0;i<300;i++){let g={}}return {}}Array.from.call(C,items).length===0",
        "let out;function C(){out={set length(v){for(let i=0;i<300;i++){let g={}}}};return out}Array.from.call(C,{length:2,0:{n:7},1:{n:8}});out[0].n===7&&out[1].n===8",
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
    for source in [
        "Array.from({[Symbol.iterator](){return {next(){return {value:1,done:false}},return(){throw 'closed'}}}})",
        "function C(){return {}}Array.from.call(C,{length:Infinity})",
        "Array.from([1],()=>{while(true){}})",
    ] {
        assert!(
            matches!(
                Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost),
                Err(Error::Limit { .. })
            ),
            "{source}"
        );
    }
    Ok(())
}
