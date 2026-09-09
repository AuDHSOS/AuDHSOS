// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use super::{Error, Limits, Runtime, SilentHost, Value, compile, eval};

#[test]
fn string_split_hooks_preserve_inputs_and_conversion_order() {
    for source in [
        "let raw={toString(){throw 7}},limit={valueOf(){throw 8}},o={[Symbol.split](s,l){return this===o&&s===raw&&l===limit&&arguments.length===2}};String.prototype.split.call(raw,o,limit)",
        "let n=0,p={get [Symbol.split](){n++;throw 7}};try{String.prototype.split.call(null,p)}catch(e){}n===0",
        "let log='',o={get [Symbol.split](){log+='h'},toString(){log+='p';return ','}};String.prototype.split.call({toString(){log+='s';return 'a,b'}},o,{valueOf(){log+='l';return 2}}).join()==='a,b'&&log==='hslp'",
        "let n=0;let o={[Symbol.split]:null,toString(){n++;return ','}};'a,b'.split(o,0).length===0&&n===1",
        "let marker={};'a'.split({[Symbol.split](){return marker}})===marker",
        "String.prototype.split.call(Symbol(),{[Symbol.split](){return 7}})===7",
        "Number.prototype[Symbol.split]=()=>{throw 7};'a1b'.split(1).join()==='a,b'",
        "let r=/,/;r[Symbol.split]=null;r.toString=()=>'-';'a-b'.split(r).join()==='a,b'",
        "let e={},ok=false;try{'a'.split({get [Symbol.split](){throw e}})}catch(x){ok=x===e}ok",
        "let n=0,e={},ok=false;try{'a'.split({toString(){n++;return ','}},{valueOf(){throw e}})}catch(x){ok=x===e}ok&&n===0",
        "let p={[Symbol.split](s,l){return l===undefined}};'a'.split(p)",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "'a'.split({[Symbol.split]:7})",
        "String.prototype.split.call(undefined,',')",
        "'a'.split(Symbol())",
        "'a'.split(',',Symbol())",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn split_species_order_flags_limit_and_abrupt_paths() {
    for source in [
        "let log='',seen,splitter={exec(){return null}},o={get constructor(){log+='c';return {get [Symbol.species](){log+='s';return function(r,f){seen=r===o&&f==='gy';log+='n';return splitter}}}},get flags(){log+='f';return {toString(){log+='t';return 'g'}}}};let a=RegExp.prototype[Symbol.split].call(o,{toString(){log+='x';return 'ab'}},{valueOf(){log+='l';return 0}});seen&&a.length===0&&log==='xcsftnl'",
        "let flags;let o={flags:'yy',constructor:{[Symbol.species]:function(r,f){flags=f;return {exec(){return null}}}}};RegExp.prototype[Symbol.split].call(o,'a');flags==='yy'",
        "let r=/,/;r.lastIndex=7;Object.freeze(r);'a,b'.split(r).join()==='a,b'&&r.lastIndex===7",
        "class R extends RegExp{}let n=0;class S extends RegExp{constructor(r,f){super(r,f);n++;if(f!=='y')throw 7}}let r=new R(',');Object.defineProperty(R,Symbol.species,{value:S});'a,b'.split(r).join()==='a,b'&&n===1",
        "let r=/,/;r.constructor=undefined;'a,b'.split(r).join()==='a,b'",
        "let r=/,/;r.constructor={[Symbol.species]:null};'a,b'.split(r).join()==='a,b'",
        "let r=/,/;r.constructor={};'a,b'.split(r).join()==='a,b'",
        "let n=0,o={get constructor(){n++;return null},get flags(){throw 7}};let ok=false;try{RegExp.prototype[Symbol.split].call(o,'',0)}catch(e){ok=e instanceof TypeError}ok&&n===1",
        "let e={},ok=false;try{RegExp.prototype[Symbol.split].call({get constructor(){throw e}},'',0)}catch(x){ok=x===e}ok",
        "let e={},ok=false;try{RegExp.prototype[Symbol.split].call({constructor:{get [Symbol.species](){throw e}}},'',0)}catch(x){ok=x===e}ok",
        "let e={},n=0,o={flags:'',constructor:{[Symbol.species]:function(){throw e}}},ok=false;try{RegExp.prototype[Symbol.split].call(o,'',{valueOf(){n++}})}catch(x){ok=x===e}ok&&n===0",
        "let e={},ok=false;try{RegExp.prototype[Symbol.split].call({get flags(){throw e}},'',0)}catch(x){ok=x===e}ok",
        "let e={},ok=false;try{RegExp.prototype[Symbol.split].call(/,/,'a',{valueOf(){throw e}})}catch(x){ok=x===e}ok",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "RegExp.prototype[Symbol.split].call(null,'')",
        "let r=/,/;r.constructor=7;r[Symbol.split]('')",
        "let r=/,/;r.constructor={[Symbol.species]:()=>({})};r[Symbol.split]('')",
        "RegExp.prototype[Symbol.split].call({flags:Symbol()},'')",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn custom_split_exec_results_capture_identity_and_limit_short_circuit() {
    for source in [
        "let log='',x={},splitter={exec(s){log+=this.lastIndex;if(this.lastIndex===1){this.lastIndex=2;return {length:3,1:x}}return null}},o={flags:'',constructor:{[Symbol.species]:function(){return splitter}}};let a=RegExp.prototype[Symbol.split].call(o,'abc');a.length===4&&a[0]==='a'&&a[1]===x&&a[2]===undefined&&Object.hasOwn(a,2)&&a[3]==='c'&&log==='012'",
        "let n=0,splitter={exec(){n++;this.lastIndex=1;return {get length(){throw 7}}}},o={flags:'',constructor:{[Symbol.species]:function(){return splitter}}};RegExp.prototype[Symbol.split].call(o,'a',1).join()===''&&n===1",
        "let splitter={exec(){this.lastIndex=1;return {length:3,1:7,get 2(){throw 9}}}},o={flags:'',constructor:{[Symbol.species]:function(){return splitter}}};RegExp.prototype[Symbol.split].call(o,'a',2).join()===',7'",
        "let log='',splitter={exec(){log+='a';this.exec=()=>{log+='b';return null};this.lastIndex=1;return {length:1}}},o={flags:'',constructor:{[Symbol.species]:function(){return splitter}}};RegExp.prototype[Symbol.split].call(o,'ab').join()===',b'&&log==='ab'",
        "let n=0,splitter={get lastIndex(){throw 7},exec(){n++;return null}},o={flags:'',constructor:{[Symbol.species]:function(){return splitter}}};RegExp.prototype[Symbol.split].call(o,'').join()===''&&n===1",
        "let n=0,splitter={get lastIndex(){throw 7},exec(){n++;return {get length(){throw 8}}}},o={flags:'',constructor:{[Symbol.species]:function(){return splitter}}};RegExp.prototype[Symbol.split].call(o,'').length===0&&n===1",
        "let splitter={exec(){this.lastIndex=Infinity;return {length:1}}},o={flags:'',constructor:{[Symbol.species]:function(){return splitter}}};RegExp.prototype[Symbol.split].call(o,'abc').join()===','",
        "let splitter={exec(){this.lastIndex={valueOf(){return 1.9}};return {length:1}}},o={flags:'',constructor:{[Symbol.species]:function(){return splitter}}};RegExp.prototype[Symbol.split].call(o,'a').join()===','",
        "let splitter={exec(){this.lastIndex=0;return {get length(){throw 7}}}},o={flags:'',constructor:{[Symbol.species]:function(){return splitter}}};RegExp.prototype[Symbol.split].call(o,'ab').join()==='ab'",
        "let e={},splitter={exec(){this.lastIndex=1;return {get length(){throw e}}}},o={flags:'',constructor:{[Symbol.species]:function(){return splitter}}},ok=false;try{RegExp.prototype[Symbol.split].call(o,'a')}catch(x){ok=x===e}ok",
        "let e={},splitter={exec(){this.lastIndex=1;return {length:2,get 1(){throw e}}}},o={flags:'',constructor:{[Symbol.species]:function(){return splitter}}},ok=false;try{RegExp.prototype[Symbol.split].call(o,'a')}catch(x){ok=x===e}ok",
        "let e={},splitter={get lastIndex(){throw e},set lastIndex(v){},exec(){return {}}},o={flags:'',constructor:{[Symbol.species]:function(){return splitter}}},ok=false;try{RegExp.prototype[Symbol.split].call(o,'a')}catch(x){ok=x===e}ok",
        "let e={},splitter={exec(){this.lastIndex={valueOf(){throw e}};return {}}},o={flags:'',constructor:{[Symbol.species]:function(){return splitter}}},ok=false;try{RegExp.prototype[Symbol.split].call(o,'a')}catch(x){ok=x===e}ok",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "let r=/,/;r.constructor={[Symbol.species]:function(){return Object.freeze(/,/y)}};r[Symbol.split]('a,b')",
        "let r=/,/;r.constructor={[Symbol.species]:function(){return {exec(){return 7}}}};r[Symbol.split]('a,b')",
        "let r=/,/;r.constructor={[Symbol.species]:function(){return {exec(){this.lastIndex=Symbol();return {}}}}};r[Symbol.split]('a,b')",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn empty_unicode_custom_splits_and_native_guard_mutations() {
    for source in [
        "let seen=[],splitter={exec(){seen.push(this.lastIndex);return {length:1}}},o={flags:'u',constructor:{[Symbol.species]:function(){return splitter}}};let a=RegExp.prototype[Symbol.split].call(o,'😀x');a.join('|')==='😀|x'&&seen.join()==='0,2,2'",
        "let seen=[],splitter={exec(){seen.push(this.lastIndex);return null}},o={flags:'v',constructor:{[Symbol.species]:function(){return splitter}}};RegExp.prototype[Symbol.split].call(o,'😀x').join()==='😀x'&&seen.join()==='0,2'",
        "let seen=[],splitter={exec(){seen.push(this.lastIndex);return null}},o={flags:'u',constructor:{[Symbol.species]:function(){return splitter}}};RegExp.prototype[Symbol.split].call(o,'\\ud800x\\udc00');seen.join()==='0,1,2'",
        "let native=RegExp.prototype.exec,seen=[];RegExp.prototype.exec=function(s){seen.push(this.lastIndex);return native.call(this,s)};'abc'.split(/,/).join()==='abc'&&seen.join()==='0,1,2'",
        "let seen=[],r=/,/;r.constructor={[Symbol.species]:function(){let x=/,/y;x.exec=function(s){seen.push(this.lastIndex);return null};return x}};'abc'.split(r).join()==='abc'&&seen.join()==='0,1,2'",
        "let native=RegExp.prototype.exec,seen=[],r=/,/;let a='abc'.split(r,{valueOf(){RegExp.prototype.exec=function(s){seen.push(this.lastIndex);return native.call(this,s)};return 10}});a.join()==='abc'&&seen.join()==='0,1,2'",
        "let native=RegExp.prototype.exec,n=0;Object.defineProperty(RegExp.prototype,'exec',{get(){n++;return native},configurable:true});'abc'.split(/,/).join()==='abc'&&n===3",
        "let native=RegExp.prototype.exec,r=/,/;r.constructor={[Symbol.species]:function(){let x=/,/y;Object.setPrototypeOf(x,{exec:native});return x}};'abc'.split(r).join()==='abc'",
        "let r=/,/;r.constructor={[Symbol.species]:function(){return /,/g}};'a,b,c'.split(r).join('|')==='||c'",
        "let r=/,/;r.constructor={[Symbol.species]:function(){return r}};'a,b'.split(r).join('|')==='a|,|b'",
        "let n=0,r=/,/;r.constructor={[Symbol.species]:function(){let x=/,/y;x.exec=0;return x}};delete RegExp.prototype.exec;'a,b'.split(r).join()==='a,b'",
        "let n=0,splitter={exec(){if(n++===0){this.lastIndex=1;return {length:2,get 1(){RegExp.prototype.exec=()=>null;return 7}}}return null}},r={flags:'',constructor:{[Symbol.species]:function(){return splitter}}};RegExp.prototype[Symbol.split].call(r,'ab').join()===',7,b'",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn split_gc_limits_and_function_metadata() -> Result<(), Error> {
    let limits = Limits {
        heap_entries: 180,
        ..Limits::default()
    };
    for source in [
        "let r=/,/;r.constructor={[Symbol.species]:function(){for(let i=0;i<300;i++){let g={}}return /,/y}};'a,b'.split(r).join()==='a,b'",
        "let splitter={exec(){this.lastIndex=1;return {length:3,get 1(){return {n:7}},get 2(){for(let i=0;i<300;i++){let g={}}return 8}}}},r={flags:'',constructor:{[Symbol.species]:function(){return splitter}}};let a=RegExp.prototype[Symbol.split].call(r,'a');a[1].n===7&&a[2]===8",
        "let splitter={exec(){return {get length(){for(let i=0;i<300;i++){let g={}}return 2},1:{n:7}}}},r={flags:'',constructor:{[Symbol.species]:function(){return splitter}}};RegExp.prototype[Symbol.split].call(r,'ab')[1].n===7",
    ] {
        assert_eq!(
            Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
            Value::Boolean(true),
            "{source}"
        );
    }
    for source in [
        "let f=RegExp.prototype[Symbol.split],d=Object.getOwnPropertyDescriptor(RegExp.prototype,Symbol.split);f.name==='[Symbol.split]'&&f.length===2&&d.writable&&!d.enumerable&&d.configurable&&f.prototype===undefined",
        "String.prototype.split.name==='split'&&String.prototype.split.length===2&&String.prototype.split.prototype===undefined",
        "let f=RegExp.prototype[Symbol.split];f.extra=7;delete f.name;delete RegExp.prototype[Symbol.split];f.call(/,/,'a,b').join()==='a,b'&&f.extra===7&&!Object.hasOwn(f,'name')",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "new String.prototype.split()",
        "new RegExp.prototype[Symbol.split]()",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })));
    }
    let limits = Limits {
        fuel: 500,
        ..Limits::default()
    };
    let source = "let r={flags:'',constructor:{[Symbol.species]:function(){return {exec(){this.lastIndex=1;return {length:Infinity}}}}}};RegExp.prototype[Symbol.split].call(r,'a')";
    assert!(matches!(
        Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost),
        Err(Error::Limit { .. })
    ));
    Ok(())
}

#[test]
fn split_native_search_is_bounded_and_custom_paths_cannot_skip_observable_calls()
-> Result<(), Error> {
    let limits = Limits {
        fuel: 50_000,
        ..Limits::default()
    };
    let source = "'a'.repeat(4096).split(/a+b/).length===1";
    assert_eq!(
        Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
        Value::Boolean(true)
    );
    for source in [
        "let count=0;let r=/,/;r.constructor={[Symbol.species]:function(){let x=/,/y;x.exec=undefined;return x}};'a,b'.split(r).join()==='a,b'",
        "let n=0;let r=/,/;r.constructor={[Symbol.species]:function(){let x=/,/y;Object.setPrototypeOf(x,{get exec(){n++;return RegExp.prototype.exec}});return x}};'abc'.split(r).join()==='abc'&&n===3",
        "let saved,old=RegExp.prototype.exec;let r=/,/;r.constructor={[Symbol.species]:function(){saved=/,/y;return saved}};'abc'.split(r);saved.lastIndex===0",
        "let saved;let r=/,/;r.constructor={[Symbol.species]:function(){saved=/$/y;return saved}};'abc'.split(r).join()==='abc'&&saved.lastIndex===0",
        "let n=0;let r=/,/;r.constructor={[Symbol.species]:function(){let x=/,/y;x.exec=function(s){n++;delete this.exec;return RegExp.prototype.exec.call(this,s)};return x}};'abc'.split(r).join()==='abc'&&n===1",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    let limits = Limits {
        properties: 64,
        ..Limits::default()
    };
    let source = "let r={flags:'',constructor:{[Symbol.species]:function(){return {exec(){this.lastIndex=1;return {length:1000}}}}}};RegExp.prototype[Symbol.split].call(r,'a')";
    assert!(matches!(
        Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost),
        Err(Error::Limit {
            resource: "split results"
        })
    ));
    Ok(())
}
