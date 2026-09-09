// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use super::{Error, Limits, Runtime, SilentHost, Value, compile, eval};

#[test]
fn map_filter_slice_species_constructor_length_and_output_properties() {
    for source in [
        "class A extends Array{}let a=new A(1,2,3);let m=a.map(v=>v*2),f=a.filter(v=>v>1),s=a.slice(1);m instanceof A&&f instanceof A&&s instanceof A&&m.join()==='2,4,6'&&f.join()==='2,3'&&s.join()==='2,3'",
        "let n,out={set length(v){throw 7}},a=[1,,3];a.constructor={[Symbol.species]:function(k){n=k;return out}};a.map(v=>v*2)===out&&n===3&&out[0]===2&&!(1 in out)&&out[2]===6",
        "let n,out={set length(v){throw 7}},a=[1,,3];a.constructor={[Symbol.species]:function(k){n=k;return out}};a.filter(v=>v>1)===out&&n===0&&out[0]===3&&!(1 in out)",
        "let n=0,out={set 0(v){n++}},a=[7];a.constructor={[Symbol.species]:function(){return out}};a.map(v=>v);let d=Object.getOwnPropertyDescriptor(out,0);n===0&&d.value===7&&d.writable&&d.enumerable&&d.configurable",
        "let n=0,out={set 0(v){n++}},a=[7];a.constructor={[Symbol.species]:function(){return out}};a.filter(v=>true);let d=Object.getOwnPropertyDescriptor(out,0);n===0&&d.value===7&&d.writable&&d.enumerable&&d.configurable",
        "let size=-1,n=-1,out={set length(v){size=v}},a=[1,,3];a.constructor={[Symbol.species]:function(k){n=k;return out}};a.slice()===out&&n===3&&size===3&&out[0]===1&&!(1 in out)&&out[2]===3",
        "let n=1,out={set length(v){n=v}},a=[1];a.constructor={[Symbol.species]:function(){return out}};a.slice(1,0)===out&&n===0",
        "let a=[1,2];a.constructor={[Symbol.species]:null};Array.isArray(a.map(v=>v))&&Array.isArray(a.filter(v=>true))&&Array.isArray(a.slice())",
        "let o={0:7,length:1,get constructor(){throw 9}};Array.prototype.map.call(o,v=>v)[0]===7&&Array.prototype.filter.call(o,v=>true)[0]===7&&Array.prototype.slice.call(o)[0]===7",
        "let a=[1,2];a.constructor={[Symbol.species]:function(){return a}};a.map((v,k)=>v+k)===a&&a.join()==='1,3'",
        "let a=[1,2,3];a.constructor={[Symbol.species]:function(){return a}};a.filter(v=>v>1)===a&&a.join()==='2,3,3'",
        "let a=[1,2,3];a.constructor={[Symbol.species]:function(){return a}};a.slice(1)===a&&a.join()==='2,3'",
        "let a=[,2,,];Array.prototype[2]=7;let m=a.map(v=>v*2),f=a.filter(()=>true),s=a.slice();m.length===3&&!(0 in m)&&m[1]===4&&Object.hasOwn(m,2)&&m[2]===14&&f.join()==='2,7'&&s.length===3&&!(0 in s)&&Object.hasOwn(s,2)",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn species_callback_and_conversion_order_survive_mutation_and_exceptions() {
    for method in ["map", "filter"] {
        for text in [
            "let log='',a=[1,2];Object.defineProperty(a,'constructor',{get(){log+='c';return {get [Symbol.species](){log+='s';return function(n){log+='n'+n;return {}}}}}});a.METHOD(v=>{log+=v;return true});log==='csnCOUNT12'",
            "let n=0,a=[1];Object.defineProperty(a,'constructor',{get(){n++;throw 7}});let ok=false;try{a.METHOD(null)}catch(e){ok=e instanceof TypeError}ok&&n===0",
            "let a=[1,2,3];a.constructor={[Symbol.species]:function(){a.length=1;return {}}};let n=0,out=a.METHOD(v=>{n++;return v});n===1&&out[0]===1&&!(1 in out)",
            "let a=[1,2];a.constructor={[Symbol.species]:function(){a[1]=7;return {}}};let log='';a.METHOD(v=>{log+=v;return true});log==='17'",
            "let a=[1,2],out={};Object.defineProperty(out,1,{value:9,writable:false});a.constructor={[Symbol.species]:function(){return out}};let n=0,ok=false;try{a.METHOD(v=>{n++;return v})}catch(e){ok=e instanceof TypeError}ok&&out[0]===1&&out[1]===9&&n===2",
            "let n=0,a=[1];a.constructor={[Symbol.species]:()=>({})};let ok=false;try{a.METHOD(()=>{n++})}catch(e){ok=e instanceof TypeError}ok&&n===0",
            "let e={},a=[1];a.constructor={get [Symbol.species](){throw e}};let ok=false;try{a.METHOD(()=>1)}catch(x){ok=x===e}ok",
            "let a=[1,2,3],log='';a.METHOD((v,k)=>{log+=v;if(k===0){delete a[1];a[2]=7;a.push(9)}return true});log==='17'",
        ] {
            let source = text
                .replace("METHOD", method)
                .replace("COUNT", if method == "map" { "2" } else { "0" });
            assert_eq!(eval(&source), Ok(Value::Boolean(true)), "{source}");
        }
    }
    for source in [
        "let log='',a=[1,2];Object.defineProperty(a,'constructor',{get(){log+='c';return {get [Symbol.species](){log+='s';return function(n){log+='n'+n;return []}}}}});a.slice({valueOf(){log+='a';return 0}},{valueOf(){log+='e';return 1}});log==='aecsn1'",
        "let a=[1,2,3];let out=a.slice({valueOf(){a.length=1;return 0}},3);out.length===3&&out[0]===1&&!(1 in out)&&!(2 in out)",
        "let a=[1,2],out={};Object.defineProperty(out,'length',{value:0,writable:false});a.constructor={[Symbol.species]:function(){return out}};let ok=false;try{a.slice()}catch(e){ok=e instanceof TypeError}ok&&out[0]===1&&out[1]===2&&a.join()==='1,2'",
        "let a=[1,2],out={};Object.defineProperty(a,0,{get(){delete a[1];return 7}});a.constructor={[Symbol.species]:function(){return out}};a.slice()===out&&out[0]===7&&!(1 in out)&&out.length===2",
        "let a=[1,2],x={};a.filter((v,k)=>{a[k]=x;return true}).join()==='1,2'",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn wide_generic_search_slice_callbacks_and_length_snapshot() {
    for source in [
        "let o={length:4294967298,4294967296:7};Array.prototype.indexOf.call(o,7,4294967295)===4294967296&&Array.prototype.includes.call(o,7,4294967295)",
        "let n=Number.MAX_SAFE_INTEGER,o={length:Infinity};o[n-2]=7;Array.prototype.indexOf.call(o,7,-2)===n-2&&Array.prototype.includes.call(o,undefined,-1)",
        "let n=Number.MAX_SAFE_INTEGER,o={length:n};o[n-3]=7;o[n-1]=9;let a=Array.prototype.slice.call(o,-3);a.length===3&&a[0]===7&&!(1 in a)&&a[2]===9",
        "let o={length:Infinity,0:7};Array.prototype.some.call(o,v=>v===7)&&!Array.prototype.every.call(o,v=>v!==7)",
        "let n=0;let o={length:Infinity,get 0(){n++;throw 7}};try{Array.prototype.forEach.call(o,()=>{})}catch(e){}n===1",
        "let n=0;let o={length:Infinity,get 0(){n++;throw 7}};try{Array.prototype.filter.call(o,()=>{})}catch(e){}n===1",
        "let n=0;let o={length:Infinity,get 0(){n++;throw 7}};try{Array.prototype.reduce.call(o,()=>{})}catch(e){}n===1",
        "let n=0;let o={length:Infinity,get 0(){n++;throw 7}};try{Array.prototype.join.call(o,'')}catch(e){}n===1",
        "let n=0;let o={length:Infinity,[Symbol.isConcatSpreadable]:true,get 0(){n++;throw 7}};try{[].concat(o)}catch(e){}n===1",
        "let n=0;let o={length:Infinity,get 0(){n++;return 7}};let ok=false;try{Array.prototype.map.call(o,v=>v)}catch(e){ok=e instanceof RangeError}ok&&n===0",
        "let n=0;let o={length:Infinity,get 0(){n++;return 7}};let ok=false;try{Array.prototype.slice.call(o)}catch(e){ok=e instanceof RangeError}ok&&n===0",
        "let n=0;let o={length:0};Array.prototype.indexOf.call(o,7,{valueOf(){n++}})===-1&&Array.prototype.includes.call(o,7,{valueOf(){n++}})===false&&n===0",
        "let o={length:2.9,0:1,1:2,2:3};Array.prototype.map.call(o,v=>v).join()==='1,2'",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn wide_push_pop_validate_before_writes_and_keep_partial_mutations() {
    for source in [
        "let o={length:4294967295};Array.prototype.push.call(o,7,8)===4294967297&&o[4294967295]===7&&o[4294967296]===8&&Array.prototype.pop.call(o)===8&&o.length===4294967296",
        "let n=Number.MAX_SAFE_INTEGER,o={length:n-1};Array.prototype.push.call(o,7)===n&&o[n-1]===7&&Array.prototype.pop.call(o)===7&&o.length===n-1&&!(n-1 in o)",
        "let n=0,o={length:Number.MAX_SAFE_INTEGER-1,set 9007199254740990(v){n++}};let ok=false;try{Array.prototype.push.call(o,1,2)}catch(e){ok=e instanceof TypeError}ok&&n===0",
        "let n=Number.MAX_SAFE_INTEGER,o={length:Infinity};Array.prototype.push.call(o)===n&&o.length===n",
        "let n=Number.MAX_SAFE_INTEGER,log='',o={get length(){log+='l';return Infinity},set length(v){log+='s';if(v!==n-1)throw 7},get 9007199254740990(){log+='g';return 7}};Array.prototype.pop.call(o)===7&&log==='lgs'&&!(n-1 in o)",
        "let o={length:2,0:1,1:2};Object.defineProperty(o,'length',{writable:false});try{Array.prototype.pop.call(o)}catch(e){}o.length===2&&!(1 in o)",
        "let a=[];a.length=4294967295;let ok=false;try{a.push(7,8)}catch(e){ok=e instanceof RangeError}ok&&a[4294967295]===7&&a[4294967296]===8&&a.length===4294967295",
        "let o={length:-1};Array.prototype.pop.call(o)===undefined&&o.length===0",
        "let o={length:2};Object.defineProperty(o,1,{value:7,configurable:false});try{Array.prototype.pop.call(o)}catch(e){}o.length===2&&o[1]===7",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn callback_outputs_gc_and_wide_scans_obey_limits() -> Result<(), Error> {
    let limits = Limits {
        heap_entries: 180,
        ..Limits::default()
    };
    for source in [
        "let a=[1,2,3];a.constructor={[Symbol.species]:function(){return {}}};a.map(v=>{for(let i=0;i<300;i++){let g={}}return {v}})[2].v===3",
        "let a=[{n:7},2];a.constructor={[Symbol.species]:function(){return {}}};a.filter((v,k)=>{a[k]=0;for(let i=0;i<300;i++){let g={}}return k===0})[0].n===7",
        "let a=[{n:7},2];a.constructor={[Symbol.species]:function(){return {set length(v){for(let i=0;i<300;i++){let g={}}}}}};a.slice()[0].n===7",
        "let o={get length(){return 1},get 0(){return {n:7}},set length(v){for(let i=0;i<300;i++){let g={}}}};Array.prototype.pop.call(o).n===7",
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
        "Array.prototype.forEach.call({length:Infinity},()=>{})",
        "Array.prototype.reduce.call({length:Infinity},()=>{})",
        "Array.prototype.filter.call({length:Infinity},()=>true)",
        "Array.prototype.some.call({length:Infinity},()=>false)",
        "Array.prototype.every.call({length:Infinity},()=>true)",
        "Array.prototype.join.call({length:Infinity},'')",
        "Array.prototype.indexOf.call({length:Infinity},7)",
        "Array.prototype.includes.call({length:Infinity},7)",
        "[].concat({length:Infinity,[Symbol.isConcatSpreadable]:true})",
        "let a=[];a.length=4294967295;a.constructor={[Symbol.species]:function(){return {}}};a.map(v=>v)",
    ] {
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
    Ok(())
}

#[test]
fn species_method_metadata_and_nonconstructibility() {
    for (name, length) in [
        ("map", 1),
        ("filter", 1),
        ("slice", 2),
        ("concat", 1),
        ("push", 1),
        ("pop", 0),
        ("join", 1),
        ("forEach", 1),
        ("some", 1),
        ("every", 1),
        ("reduce", 1),
        ("indexOf", 1),
        ("includes", 1),
    ] {
        let source = format!(
            "let f=Array.prototype.{name},d=Object.getOwnPropertyDescriptor(Array.prototype,'{name}');f.name==='{name}'&&f.length==={length}&&d.writable&&!d.enumerable&&d.configurable&&f.prototype===undefined"
        );
        assert_eq!(eval(&source), Ok(Value::Boolean(true)), "{source}");
        let source = format!(
            "let f=Array.prototype.{name};delete f.name;f.extra=7;delete Array.prototype.{name};f.extra===7&&!Object.hasOwn(f,'name')&&!Object.hasOwn(Array.prototype,'{name}')"
        );
        assert_eq!(eval(&source), Ok(Value::Boolean(true)), "{source}");
        assert!(matches!(
            eval(&format!("new Array.prototype.{name}()")),
            Err(Error::Type { .. })
        ));
    }
}
