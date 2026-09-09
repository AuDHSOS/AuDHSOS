// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use super::{Error, Limits, Runtime, SilentHost, Value, compile, eval};

#[test]
fn flat_depth_conversion_holes_and_only_real_arrays_flatten() {
    for source in [
        "[1,[2,[3]]].flat().join()==='1,2,3'&&Array.isArray([1,[2,[3]]].flat()[2])",
        "[1,[2,[3,[4]]]].flat(2).length===4&&Array.isArray([1,[2,[3,[4]]]].flat(2)[3])",
        "[1,[2,[3,[4]]]].flat(Infinity).join()==='1,2,3,4'&&[1,[2,[3,[4]]]].flat(Infinity)[3]===4",
        "let a=[1,[2],,undefined];let b=a.flat(0);b!==a&&b.length===3&&b[1]===a[1]&&b[2]===undefined&&Object.hasOwn(b,2)&&!(2 in a)",
        "let x=[1];[x].flat(-1)[0]===x&&[x].flat(NaN)[0]===x&&[x].flat(-Infinity)[0]===x&&[x].flat(-0.5)[0]===x",
        "let x=[1];[x].flat(0.9)[0]===x&&[x].flat(1.9)[0]===1&&[x].flat(undefined)[0]===1",
        "[,,[,,1,,],undefined].flat().length===2&&[,,[,,1,,],undefined].flat()[0]===1",
        "let p={0:7};let a=[,1];Object.setPrototypeOf(a,p);Array.prototype.flat.call(a).join()==='7,1'",
        "let inner=[,2];Object.setPrototypeOf(inner,{0:7});[inner].flat().join()==='7,2'",
        "let o={0:7,length:1,[Symbol.isConcatSpreadable]:true};[o].flat()[0]===o",
        "let a=[1,2];a[Symbol.isConcatSpreadable]=false;a[Symbol.iterator]=()=>{throw 7};[a].flat().join()==='1,2'",
        "let a=Object.create(Array.prototype);a.length=1;a[0]=7;[a].flat()[0]===a",
        "let a={length:3,0:[1,2],2:3};Array.prototype.flat.call(a).join()==='1,2,3'",
        "Array.prototype.flat.call('a😀').length===3&&Array.prototype.flat.call(false).length===0&&Array.prototype.flat.call(Symbol()).length===0",
        "let o={length:2.9,0:[1],1:[2],2:[3]};Array.prototype.flat.call(o).join()==='1,2'",
        "let a=[];a[0]=a;a.flat(0)[0]===a&&a.flat(1)[0]===a&&a.flat(3)[0]===a",
        "let a=[1,[2]];Object.freeze(a);a.flat().join()==='1,2'&&a.length===2&&Array.isArray(a[1])",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "Array.prototype.flat.call(null)",
        "Array.prototype.flat.call(undefined)",
        "[1].flat(Symbol())",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn flatmap_calls_only_present_top_level_indices_and_flattens_one_level() {
    for source in [
        "[1,2,3].flatMap(v=>[v,v+1]).join()==='1,2,2,3,3,4'",
        "let n=0;let a=[1,,3].flatMap((v,k)=>{n++;return [,v,k,,]});n===2&&a.join()==='1,0,3,2'&&Object.keys(a).join()==='0,1,2,3'",
        "let x=[7],n=0;let a=[1,2].flatMap(v=>{n++;return [[x]]});n===2&&a[0][0]===x&&a[1][0]===x",
        "let source=[1,2],seen=0,context={};let result=source.flatMap(function(v,k,o){'use strict';if(this!==context||arguments.length!==3||o!==source)throw 7;seen+=k;return [v]},context);seen===1&&result.join()==='1,2'",
        "let seen=false;[1].flatMap(function(){'use strict';seen=this===undefined;return []});seen",
        "let o={length:1,0:7};Array.prototype.flatMap.call(o,(v,k,s)=>[v,k,s])[2]===o",
        "Array.prototype.flatMap.call('ab',(v,k,s)=>[v,k,typeof s]).join()==='a,0,object,b,1,object'",
        "let o={length:1,0:7,[Symbol.isConcatSpreadable]:true};[1].flatMap(()=>o)[0]===o",
        "let a=[1];a[Symbol.isConcatSpreadable]=false;[7].flatMap(()=>a)[0]===1",
        "let a=[1,2,3],log='';a.flatMap((v,k)=>{log+=v;if(k===0){delete a[1];a[2]=7;a.push(9)}return v});log==='17'",
        "let f=v=>[v+1];f.call=()=>{throw 7};[1].flatMap(f)[0]===2",
        "let a=[1].flatMap(async v=>[v]);a.length===1&&a[0] instanceof Promise",
        "[1,2].flatMap(Array.of).length===6",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "[].flatMap()",
        "[].flatMap(undefined)",
        "[].flatMap(null)",
        "[].flatMap({})",
        "Array.prototype.flatMap.call(null,()=>1)",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn flatten_species_order_custom_targets_and_partial_effects() {
    for source in [
        "let log='',a=[[1]];Object.defineProperty(a,'constructor',{get(){log+='c';return {get [Symbol.species](){log+='s';return function(n){log+='n'+n;return {}}}}}});let out=a.flat({valueOf(){log+='d';return 1}});log==='dcsn0'&&out[0]===1&&!Object.hasOwn(out,'length')",
        "let log='',o={get length(){log+='l';return 1},0:[7]};let out=Array.prototype.flat.call(o,{valueOf(){log+='d';return 1}});log==='ld'&&out[0]===7",
        "let n=0,a=[];Object.defineProperty(a,'constructor',{get(){n++;throw 7}});let ok=false;try{a.flatMap(null)}catch(e){ok=e instanceof TypeError}ok&&n===0",
        "let e={},n=0,ok=false;try{Array.prototype.flatMap.call({get length(){n++;throw e}},null)}catch(x){ok=x===e}ok&&n===1",
        "let a=[[1]],out={set length(v){throw 7}};a.constructor={[Symbol.species]:function(n){if(n!==0)throw 8;return out}};a.flat()===out&&out[0]===1",
        "let a=[1],out={set length(v){throw 7}};a.constructor={[Symbol.species]:function(n){if(n!==0)throw 8;return out}};a.flatMap(v=>[v])===out&&out[0]===1",
        "class A extends Array{}let a=new A([1],[2]);a.flat() instanceof A&&a.flatMap(v=>v) instanceof A",
        "let a=[[1],[2]],out={set 0(v){throw 7}};a.constructor={[Symbol.species]:function(){return out}};a.flat()===out&&Object.getOwnPropertyDescriptor(out,0).value===1&&out[1]===2",
        "let a=[[1],[2]],out={};Object.defineProperty(out,1,{value:9,writable:false});a.constructor={[Symbol.species]:function(){return out}};let ok=false;try{a.flat()}catch(e){ok=e instanceof TypeError}ok&&out[0]===1&&out[1]===9",
        "let a=[1,2],out={};Object.defineProperty(out,1,{value:9,writable:false});a.constructor={[Symbol.species]:function(){return out}};let n=0,ok=false;try{a.flatMap(v=>{n++;return [v]})}catch(e){ok=e instanceof TypeError}ok&&n===2&&out[0]===1&&out[1]===9",
        "let a=[1,[2]],out=a;a.constructor={[Symbol.species]:function(){return out}};a.flat()===a&&a.join()==='1,2'",
        "let a=[1,2];a.constructor={[Symbol.species]:function(){return a}};a.flatMap(v=>[v,v]).join()==='1,1,1,1'",
        "let nested=[1];Object.defineProperty(nested,'constructor',{get(){throw 7}});[nested].flat()[0]===1",
        "let o={length:1,0:[7],get constructor(){throw 7}};Array.prototype.flat.call(o)[0]===7",
        "let e={},a=[],ok=false;Object.defineProperty(a,'constructor',{get(){throw e}});try{a.flat()}catch(x){ok=x===e}ok",
        "let e={},a=[],ok=false;a.constructor={[Symbol.species]:function(){throw e}};try{a.flatMap(()=>1)}catch(x){ok=x===e}ok",
        "let e={},n=0,a=[];Object.defineProperty(a,'constructor',{get(){n++}});let ok=false;try{a.flat({valueOf(){throw e}})}catch(x){ok=x===e}ok&&n===0",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn flatten_source_snapshots_and_depth_first_mutations() {
    for source in [
        "let log='',inner=[1,2];Object.defineProperty(inner,0,{get(){log+='a';delete inner[1];return 7}});let outer=[inner,3];Object.defineProperty(outer,1,{get(){log+='b';return 3}});outer.flat().join()==='7,3'&&log==='ab'",
        "let inner=[1,2],outer=[inner,3];Object.defineProperty(inner,0,{get(){outer[1]=7;outer.push(8);inner.push(9);return 1}});outer.flat().join()==='1,2,7'",
        "let next=[1],first=[2],outer=[first,next];Object.defineProperty(first,0,{get(){next.push(7);return 2}});outer.flat().join()==='2,1,7'",
        "let a=[[1],[2]];a.constructor={[Symbol.species]:function(){a.length=1;return {}}};let out=a.flat();out[0]===1&&!Object.hasOwn(out,1)",
        "let a=[[1],[2]];let out=a.flat({valueOf(){a.push([3]);return 1}});out.join()==='1,2'",
        "let a=[1,2],log='',inner=[7];Object.defineProperty(inner,0,{get(){log+='i';return 7}});a.flatMap(v=>{log+='m';return inner});log==='mimi'",
        "let n=0,e={},a=[1,2],ok=false;try{a.flatMap(v=>{n++;throw e})}catch(x){ok=x===e}ok&&n===1",
        "let e={},inner=[];Object.defineProperty(inner,0,{get(){throw e}});let ok=false;try{[inner].flat()}catch(x){ok=x===e}ok",
        "let o={length:Infinity,get 0(){throw 7}};let ok=false;try{Array.prototype.flat.call(o)}catch(e){ok=e===7}ok",
        "let o={length:Infinity,get 0(){throw 7}};let ok=false;try{Array.prototype.flatMap.call(o,()=>1)}catch(e){ok=e===7}ok",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn flatten_gc_roots_release_siblings_and_keep_live_pending_arrays() -> Result<(), Error> {
    let limits = Limits {
        heap_entries: 180,
        ..Limits::default()
    };
    for source in [
        "let inner=[{n:7},2],outer=[inner];Object.defineProperty(inner,0,{get(){outer.length=0;inner=null;for(let i=0;i<300;i++){let g={}}return {n:7}}});let b=outer.flat();b[0].n===7&&b[1]===2",
        "let a=[{n:7},{n:8}];a.constructor={[Symbol.species]:function(){for(let i=0;i<300;i++){let g={}}return {}}};let b=a.flatMap(v=>{a.length=0;for(let i=0;i<300;i++){let g={}}return [v]});b[0].n===7&&!Object.hasOwn(b,1)",
        "let o={length:300},getter=function(){return []};for(let i=0;i<300;i++){Object.defineProperty(o,i,{get:getter})};Array.prototype.flat.call(o).length===0",
        "let items={length:300};let result=Array.prototype.flatMap.call(items,()=>[]);result.length===0",
        "let a=Array(300).fill(1);a.flatMap(()=>[]).length===0",
        "let source=[1];let out=source.flatMap(()=>{let a=[{n:7}];Object.defineProperty(a,1,{get(){for(let i=0;i<300;i++){let g={}}return 8}});return a});out[0].n===7&&out[1]===8",
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
        realm.evaluate("try{[1].flatMap(()=>{throw 7})}catch(e){}[1,[2]].flat().join()")?,
        Value::string("1,2")
    );
    Ok(())
}

#[test]
fn flatten_depth_and_cycles_obey_cumulative_frame_and_fuel_limits() -> Result<(), Error> {
    let limits = Limits {
        nesting: 24,
        ..Limits::default()
    };
    for source in [
        "let a=[];a[0]=a;try{a.flat(Infinity)}catch(e){throw 'caught'}",
        "let a=[1];for(let i=0;i<40;i++)a=[a];a.flat(100)",
    ] {
        assert!(
            matches!(
                Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost),
                Err(Error::Limit {
                    resource: "array flatten frames"
                })
            ),
            "{source}"
        );
    }
    let source = "let a=[1];for(let i=0;i<22;i++)a=[a];a.flat(Infinity)[0]===1";
    assert_eq!(
        Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
        Value::Boolean(true)
    );
    let source = "let inner=[1];for(let i=0;i<10;i++)inner=[inner];let outer=[1];Object.defineProperty(outer,0,{get(){return inner.flat(Infinity)}});for(let i=0;i<15;i++)outer=[outer];outer.flat(Infinity)";
    assert!(matches!(
        Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost),
        Err(Error::Limit {
            resource: "array flatten frames"
        })
    ));
    let limits = Limits {
        fuel: 1000,
        ..Limits::default()
    };
    for source in [
        "Array.prototype.flat.call({length:Infinity})",
        "Array.prototype.flatMap.call({length:Infinity},()=>1)",
        "[1].flatMap(function f(){return [1].flatMap(f)})",
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

#[test]
fn flatten_method_metadata_and_nonconstructibility() {
    for (name, length) in [("flat", 0), ("flatMap", 1)] {
        let source = format!(
            "let f=Array.prototype.{name},d=Object.getOwnPropertyDescriptor(Array.prototype,'{name}');f.name==='{name}'&&f.length==={length}&&f.prototype===undefined&&d.writable&&!d.enumerable&&d.configurable&&Array.prototype[Symbol.unscopables].{name}"
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
