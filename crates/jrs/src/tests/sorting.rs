// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use super::{Error, Limits, Runtime, SilentHost, Value, compile, eval};

#[test]
fn copying_sort_materializes_holes_and_never_uses_species_or_setters() {
    for source in [
        "let a=[3,1,2],b=a.toSorted();b!==a&&Array.isArray(b)&&b.join()==='1,2,3'&&a.join()==='3,1,2'",
        "let a=[,3,undefined,,1],b=a.toSorted();b.length===5&&b[0]===1&&b[1]===3&&b[2]===undefined&&Object.keys(b).join()==='0,1,2,3,4'&&!(0 in a)&&!(3 in a)",
        "let a=[1,,];Array.prototype[1]=7;let b=a.toSorted();b.join()==='1,7'&&Object.hasOwn(b,1)",
        "class A extends Array{}let a=new A(3,1,2);Object.defineProperty(a,'constructor',{get(){throw 7}});let b=a.toSorted();Array.isArray(b)&&!(b instanceof A)&&Object.getPrototypeOf(b)===Array.prototype&&b.join()==='1,2,3'",
        "Object.defineProperty(Array,Symbol.species,{get(){throw 7}});[2,1].toSorted().join()==='1,2'",
        "Object.defineProperty(Array.prototype,0,{set(v){throw 7},configurable:true});let b=Array.prototype.toSorted.call({length:1,0:7});Object.getOwnPropertyDescriptor(b,0).value===7",
        "Object.freeze([2,1]).toSorted().join()==='1,2'",
        "Array.prototype.toSorted.call('ba').join()==='a,b'&&Array.prototype.toSorted.call(false).length===0",
        "let o={length:'3.9',0:3,2:1};let b=Array.prototype.toSorted.call(o);b.join()==='1,3,'&&b.length===3&&Object.hasOwn(b,2)&&!(1 in o)",
        "let o={get length(){return 2},set length(v){throw 7},0:2,1:1};Array.prototype.toSorted.call(o).join()==='1,2'",
        "let a=[];let b=a.toSorted();b!==a&&b.length===0",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn sort_getter_snapshot_order_errors_and_partial_writes() {
    for method in ["sort", "toSorted"] {
        for body in [
            "let log='',o={get length(){log+='l';return 3},get 0(){log+='a';return 3},get 1(){log+='b';return 2},get 2(){log+='c';return 1},set 0(v){log+='x'},set 1(v){log+='y'},set 2(v){log+='z'}};let calls=0;Array.prototype.METHOD.call(o,(a,b)=>{if(calls++===0&&log!=='labc')throw 7;return a-b});calls>0&&log.startsWith('labc')",
            "let n=0,o={get length(){n++;throw 7}};let ok=false;try{Array.prototype.METHOD.call(o,null)}catch(e){ok=e instanceof TypeError}ok&&n===0",
            "let a=[3,2,1],n=0;try{a.METHOD(()=>{n++;throw 7})}catch(e){}n===1&&a.join()==='3,2,1'",
            "let a=[3,2,1],n=0;try{a.METHOD(()=>({valueOf(){n++;throw 7}}))}catch(e){}n===1&&a.join()==='3,2,1'",
            "let n=0,o={length:3,get 0(){return 3},get 1(){throw 7},get 2(){n++;return 1}};try{Array.prototype.METHOD.call(o,()=>{n++;return 0})}catch(e){}n===0",
            "let a=[3,2,1];let b=a.METHOD((x,y)=>{a.length=0;return x-y});b.join()==='1,2,3'",
            "let a=[2,1],n=0;let b=a.METHOD((x,y)=>{n++;return NaN});b.join()==='2,1'&&n>0",
            "let n=0;let a=[undefined,,2,1].METHOD((a,b)=>{if(a===undefined||b===undefined)throw 7;n++;return a-b});a[0]===1&&a[1]===2&&n>0",
            "let ok=false;[2,1].METHOD(function(a,b){'use strict';ok=this===undefined&&arguments.length===2;return a-b});ok",
            "let e={},ok=false;try{[{toString(){throw e}},1].METHOD()}catch(x){ok=x===e}ok",
        ] {
            let source = body.replace("METHOD", method);
            assert_eq!(eval(&source), Ok(Value::Boolean(true)), "{source}");
        }
        for value in ["null", "Symbol()", "{}", "1"] {
            let source = format!("[2,1].{method}({value})");
            assert!(matches!(eval(&source), Err(Error::Type { .. })), "{source}");
        }
    }
    for source in [
        "let a=[3,2,1];Object.defineProperty(a,1,{writable:false});try{a.sort()}catch(e){}a.join()==='1,2,1'",
        "let a=[,,1,2];Object.defineProperty(a,3,{configurable:false});try{a.sort()}catch(e){}a.length===4&&a[0]===1&&a[1]===2&&!(2 in a)&&a[3]===2",
        "let a=[2,1];Object.defineProperty(a,'length',{writable:false});a.sort()===a&&a.join()==='1,2'",
        "let a=[3,2,1];let b=a.toSorted((x,y)=>{a[0]=9;return x-y});a[0]===9&&b.join()==='1,2,3'",
        "let o={length:3,get 0(){delete this[1];return 3},1:2,2:1};let b=Array.prototype.toSorted.call(o);b.join()==='1,3,'&&Object.hasOwn(b,2)",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn sorting_wide_lengths_limits_and_gc() -> Result<(), Error> {
    for source in [
        "let e={},n=0,o={length:Infinity,get 0(){n++;throw e}},ok=false;try{Array.prototype.sort.call(o)}catch(x){ok=x===e}ok&&n===1",
        "let e={},n=0,o={length:4294967296,get 0(){n++;throw e}},ok=false;try{Array.prototype.sort.call(o)}catch(x){ok=x===e}ok&&n===1",
        "let n=0,o={length:4294967296,get 0(){n++;return 1}},ok=false;try{Array.prototype.toSorted.call(o)}catch(e){ok=e instanceof RangeError}ok&&n===0",
        "let o={length:Infinity},ok=false;try{Array.prototype.toSorted.call(o)}catch(e){ok=e instanceof RangeError}ok",
        "let n=0;let a=[{n:2},{n:1}];let b=a.toSorted((x,y)=>{a.length=0;for(let i=0;i<300;i++){let g={}}n++;return x.n-y.n});b[0].n===1&&b[1].n===2&&a.length===0&&n>0",
        "let a=[{n:2},{n:1}];a.sort((x,y)=>{a.length=0;for(let i=0;i<300;i++){let g={}}return {valueOf(){return x.n-y.n}}});a[0].n===1&&a[1].n===2",
    ] {
        let limits = Limits {
            heap_entries: 180,
            ..Limits::default()
        };
        assert_eq!(
            Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
            Value::Boolean(true),
            "{source}"
        );
    }
    for source in [
        "Array.prototype.sort.call({length:Infinity})",
        "Array(4294967295).toSorted()",
    ] {
        let limits = Limits {
            fuel: 1000,
            ..Limits::default()
        };
        assert!(
            matches!(
                Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost),
                Err(Error::Limit { .. })
            ),
            "{source}"
        );
    }
    for method in ["sort", "toSorted"] {
        let source = format!(
            "let p={{}},o={{length:65,64:64}};for(let i=0;i<64;i++)p[i]=i;Object.setPrototypeOf(o,p);Array.prototype.{method}.call(o)"
        );
        let limits = Limits {
            properties: 64,
            ..Limits::default()
        };
        // Each source object fits the quota but the inherited combined list does not.
        assert!(matches!(
            Runtime::new(limits).run(&compile(&source, limits)?, &mut SilentHost),
            Err(Error::Limit {
                resource: "sort items"
            })
        ));
    }
    Ok(())
}

#[test]
fn sort_method_metadata_and_stable_equal_keys() {
    for name in ["sort", "toSorted"] {
        for source in [
            format!(
                "let f=Array.prototype.{name},d=Object.getOwnPropertyDescriptor(Array.prototype,'{name}');f.name==='{name}'&&f.length===1&&f.prototype===undefined&&d.writable&&!d.enumerable&&d.configurable"
            ),
            format!(
                "let f=Array.prototype.{name};f.extra=7;delete f.name;delete Array.prototype.{name};f.call([2,1]).join()==='1,2'&&f.extra===7&&!Object.hasOwn(f,'name')"
            ),
            format!(
                "let a=[{{k:2,id:'a'}},{{k:1,id:'b'}},{{k:2,id:'c'}},{{k:1,id:'d'}}];a.{name}((x,y)=>x.k-y.k).map(x=>x.id).join()==='b,d,a,c'"
            ),
        ] {
            assert_eq!(eval(&source), Ok(Value::Boolean(true)), "{source}");
        }
        assert!(matches!(
            eval(&format!("new Array.prototype.{name}()")),
            Err(Error::Type { .. })
        ));
    }
}
