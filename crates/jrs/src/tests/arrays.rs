// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::eval;
use crate::{Error, Limits, Runtime, SilentHost, Value, compile};

#[test]
fn literals_holes_length_and_array_identity() {
    for (source, expected) in [
        ("[].length", Value::Number(0.0)),
        ("[1,2,].length", Value::Number(2.0)),
        ("[,,].length", Value::Number(2.0)),
        ("[1,,3][1]", Value::Undefined),
        ("0 in [,]", Value::Boolean(false)),
        ("0 in [undefined]", Value::Boolean(true)),
        ("Array.isArray([])", Value::Boolean(true)),
        ("Array.isArray({})", Value::Boolean(false)),
        ("Array.isArray(Array.prototype)", Value::Boolean(true)),
        ("Array(4).length", Value::Number(4.0)),
        ("Array('4')[0]", Value::string("4")),
        ("Array.of(4)[0]", Value::Number(4.0)),
        ("let a=[];a[9]=42;a.length", Value::Number(10.0)),
        ("let a=[1,2,3];delete a[2];a.length", Value::Number(3.0)),
        ("let a=[1,2,3];a.length=1;2 in a", Value::Boolean(false)),
        ("let a=[];a['4294967295']=1;a.length", Value::Number(0.0)),
        (
            "let a=[];a[4294967294]=1;a.length",
            Value::Number(4_294_967_295.0),
        ),
        (
            "Object.getOwnPropertyDescriptor([],'length').configurable",
            Value::Boolean(false),
        ),
        (
            "Object.getPrototypeOf([])===Array.prototype",
            Value::Boolean(true),
        ),
        (
            "Object.prototype.toString.call([])",
            Value::string("[object Array]"),
        ),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
}

#[test]
fn array_length_truncation_preserves_nonconfigurable_invariants() {
    for (source, expected) in [
        (
            "let a=[1,2,3,4];Object.defineProperty(a,'1',{configurable:false});a.length=0;a.length",
            Value::Number(2.0),
        ),
        (
            "let a=[1,2,3];Object.defineProperty(a,'1',{configurable:false});try{Object.defineProperty(a,'length',{value:0,writable:false});}catch(e){}a.length",
            Value::Number(2.0),
        ),
        (
            "let a=[1,2,3];Object.defineProperty(a,'1',{configurable:false});try{Object.defineProperty(a,'length',{value:0,writable:false});}catch(e){}Object.getOwnPropertyDescriptor(a,'length').writable",
            Value::Boolean(false),
        ),
        (
            "let a=[1];Object.defineProperty(a,'length',{writable:false});a[1]=2;a.length",
            Value::Number(1.0),
        ),
        ("let a=[1];a.length='2';a.length", Value::Number(2.0)),
        ("let a=[1];a.length=-0;a.length", Value::Number(0.0)),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
    for source in [
        "Array(-1)",
        "Array(1.5)",
        "Array(NaN)",
        "Array(4294967296)",
        "let a=[];a.length=Infinity",
        "let a=[];a.length=-1",
    ] {
        assert!(matches!(eval(source), Err(Error::Range { .. })), "{source}");
    }
    assert_eq!(
        eval("try{Array(-1);}catch(e){e.name}"),
        Ok(Value::string("RangeError"))
    );
}

#[test]
fn basic_array_methods_are_generic_and_preserve_holes() {
    for (source, expected) in [
        ("let a=[1];a.push(2,3);a.pop()+a.length", Value::Number(5.0)),
        ("[].pop()", Value::Undefined),
        ("[1,null,,undefined,5].join('-')", Value::string("1----5")),
        ("[1,2,3].slice(1).join()", Value::string("2,3")),
        ("[1,2,3].slice(-2,-1).join()", Value::string("2")),
        ("1 in [1,,3].slice()", Value::Boolean(false)),
        ("[1,2,3].indexOf(2)", Value::Number(1.0)),
        ("[NaN].indexOf(NaN)", Value::Number(-1.0)),
        ("[NaN].includes(NaN)", Value::Boolean(true)),
        ("[,].indexOf(undefined)", Value::Number(-1.0)),
        ("[,].includes(undefined)", Value::Boolean(true)),
        ("[1,2,1].indexOf(1,-1)", Value::Number(2.0)),
        (
            "Array.prototype.join.call({0:'a',2:'c',length:3},'-')",
            Value::string("a--c"),
        ),
        (
            "let o={length:0};Array.prototype.push.call(o,42);o[0]+o.length",
            Value::Number(43.0),
        ),
        ("Object.keys({b:1,2:2,a:3}).join()", Value::string("2,b,a")),
        (
            "Object.getOwnPropertyNames([1]).join()",
            Value::string("0,length"),
        ),
        ("Object.keys([1,,3]).join()", Value::string("0,2")),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
}

#[test]
fn callbacks_share_frames_and_observe_mutation_in_order() {
    for (source, expected) in [
        (
            "let s=0;[1,,3].forEach((v,i,a)=>s+=v+i+a.length);s",
            Value::Number(12.0),
        ),
        ("[1,,3].map(x=>x*2).join()", Value::string("2,,6")),
        ("[1,2,3].filter(x=>x%2).join()", Value::string("1,3")),
        ("[1,2,3].some(x=>x==2)", Value::Boolean(true)),
        ("[1,2,3].every(x=>x>0)", Value::Boolean(true)),
        ("[].every(()=>false)", Value::Boolean(true)),
        ("[].some(()=>true)", Value::Boolean(false)),
        ("[1,2,3].reduce((a,b)=>a+b,10)", Value::Number(16.0)),
        ("[,1,,2].reduce((a,b)=>a+b)", Value::Number(3.0)),
        ("[].reduce((a,b)=>a+b,42)", Value::Number(42.0)),
        (
            "let a=[1,2,3];let s=0;a.forEach((v,i)=>{s+=v;if(i==0){delete a[1];a.push(4);a[2]=30;}});s",
            Value::Number(31.0),
        ),
        (
            "let o={s:0};[1,2].forEach(function(v){this.s+=v;},o);o.s",
            Value::Number(3.0),
        ),
        (
            "[1,2].map(x=>[3,4].reduce((a,b)=>a+b,x)).join()",
            Value::string("8,9"),
        ),
        (
            "try{[1,2].forEach(x=>{throw x;});}catch(e){e}",
            Value::Number(1.0),
        ),
        (
            "let o={0:42,length:1};let n=0;Array.prototype.forEach.call(o,v=>n=v);n",
            Value::Number(42.0),
        ),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
    for source in [
        "[].forEach(1)",
        "[].map(null)",
        "[].reduce(()=>1)",
        "Array(10).reduce(()=>1)",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn array_callbacks_and_native_loops_respect_quotas() -> Result<(), Error> {
    let limits = Limits {
        fuel: 100,
        ..Limits::default()
    };
    let program = compile("Array(4294967295).forEach(()=>1)", limits)?;
    assert!(matches!(
        Runtime::new(limits).run(&program, &mut SilentHost),
        Err(Error::Limit { .. })
    ));
    let program = compile("Array(4294967295).join()", limits)?;
    assert!(matches!(
        Runtime::new(limits).run(&program, &mut SilentHost),
        Err(Error::Limit { .. })
    ));
    let limits = Limits {
        // Array.prototype retains GC-owned search and species-aware methods.
        // The 100 transient objects below still force repeated collection.
        heap_entries: 72,
        ..Limits::default()
    };
    let program = compile(
        "let a=[1,2,3];a.map(x=>{for(let i=0;i<100;i++){let o={};o.o=o;}return {x};})[2].x",
        limits,
    )?;
    assert_eq!(
        Runtime::new(limits).run(&program, &mut SilentHost)?,
        Value::Number(3.0)
    );
    Ok(())
}

#[test]
fn callback_generics_internal_hooks_and_early_exit() {
    for (source, expected) in [
        (
            "Array.prototype.map.call('abc',x=>x).join('-')",
            Value::string("a-b-c"),
        ),
        (
            "let f=x=>x+1;f.call=()=>99;[1].map(f)[0]",
            Value::Number(2.0),
        ),
        ("let Array=()=>99;[1].map(x=>x+1)[0]", Value::Number(2.0)),
        (
            "let n=0;[1,2,3].some(x=>{n++;return x==2;});n",
            Value::Number(2.0),
        ),
        (
            "let n=0;[1,2,3].every(x=>{n++;return x<2;});n",
            Value::Number(2.0),
        ),
        (
            "let a=[1,,3];let n=0;a.forEach((x,i)=>{n+=x;if(i==0)a[1]=10;});n",
            Value::Number(14.0),
        ),
        (
            "let a=[1,2,3];a.filter((x,i)=>{a[i]=99;return true;}).join()",
            Value::string("1,2,3"),
        ),
        ("Array.prototype.forEach.call(42,()=>1)", Value::Undefined),
        ("[1,2].toString()", Value::string("1,2")),
        ("let a=[];a.join=()=>42;a.toString()", Value::Number(42.0)),
        (
            "let a=[];a.join=1;a.toString()",
            Value::string("[object Array]"),
        ),
        (
            "try{[1].map(()=>{throw 42;});}catch(e){e}",
            Value::Number(42.0),
        ),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
}

#[test]
fn array_methods_negative_paths_and_boundaries() {
    for (source, expected) in [
        ("[1,2].indexOf(1,Infinity)", Value::Number(-1.0)),
        ("[1,2].indexOf(1,-Infinity)", Value::Number(0.0)),
        ("[1,2].slice(2,1).length", Value::Number(0.0)),
        ("[1,2].slice(-0.5).join()", Value::string("1,2")),
        ("[1,2].includes(9)", Value::Boolean(false)),
        ("let a=[];a['01']=42;a.length", Value::Number(0.0)),
        (
            "let a=Object.freeze([1]);a[0]=2;a[1]=3;a.length",
            Value::Number(1.0),
        ),
        (
            "let a=Object.freeze([1]);try{Object.defineProperty(a,'length',{value:0});}catch(e){e.name}",
            Value::string("TypeError"),
        ),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
    for source in [
        "Object.freeze([1]).pop()",
        "Object.freeze([]).push(1)",
        "Array.prototype.join.call(null)",
        "Object.defineProperty([],'length',{configurable:true})",
        "Array.prototype.map.call(undefined,()=>1)",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn array_integrity_transitions_match_the_wpt_object_scenarios() {
    // Project-owned regression, not an imported WPT test or a suite pass.
    assert_eq!(
        eval(
            "let count=0;[{},[]].forEach(function(that){that.prop='exist';Object.preventExtensions(that);if(Object.isExtensible(that))throw 1;if(Object.isFrozen(that))throw 2;if(Object.isSealed(that))throw 3;that.extension=1;if(that.extension!==undefined)throw 4;that.prop='changed';if(that.prop!=='changed')throw 5;delete that.prop;if(that.prop!==undefined)throw 6;count++;});count"
        ),
        Ok(Value::Number(2.0))
    );
    assert_eq!(
        eval(
            "let count=0;[{},[]].forEach(function(that){that.prop='exist';Object.freeze(that);if(Object.isExtensible(that))throw 1;if(!Object.isFrozen(that))throw 2;if(!Object.isSealed(that))throw 3;that.prop='changed';delete that.prop;if(that.prop!=='exist')throw 4;count++;});count"
        ),
        Ok(Value::Number(2.0))
    );
    assert_eq!(
        eval(
            "let count=0;[{},[]].forEach(function(that){that.prop='exist';Object.seal(that);if(Object.isExtensible(that))throw 1;if(Object.isFrozen(that))throw 2;if(!Object.isSealed(that))throw 3;that.prop='changed';delete that.prop;if(that.prop!=='changed')throw 4;count++;});count"
        ),
        Ok(Value::Number(2.0))
    );
}

#[test]
fn callback_failures_unwind_and_later_execution_still_works() -> Result<(), Error> {
    let limits = Limits {
        call_frames: 12,
        ..Limits::default()
    };
    let mut runtime = Runtime::new(limits);
    let recursive = compile("function f(){[1].forEach(f);}f()", limits)?;
    assert!(matches!(
        runtime.run(&recursive, &mut SilentHost),
        Err(Error::Limit {
            resource: "call frames"
        })
    ));
    let program = compile("[1,2].map(x=>x+1).join()", limits)?;
    assert_eq!(
        runtime.run(&program, &mut SilentHost)?,
        Value::string("2,3")
    );
    assert_eq!(
        runtime.run(&program, &mut SilentHost)?,
        Value::string("2,3")
    );
    assert_eq!(
        eval("let n=0;try{[1,2].map(x=>{try{throw x;}finally{n++;}});}catch(e){n+=e;}n"),
        Ok(Value::Number(2.0))
    );
    let length_limits = Limits {
        fuel: 300,
        ..limits
    };
    let length_limit = compile(
        "Array.prototype.join.call({length:Infinity})",
        length_limits,
    )?;
    assert!(matches!(
        Runtime::new(length_limits).run(&length_limit, &mut SilentHost),
        Err(Error::Limit {
            resource: "execution fuel"
        })
    ));
    Ok(())
}

#[test]
fn constructors_bind_this_and_select_object_returns() {
    for (source, expected) in [
        ("(new Array).length", Value::Number(0.0)),
        ("new Array(3).length", Value::Number(3.0)),
        ("new Array(1,2,3).join()", Value::string("1,2,3")),
        (
            "function C(x){this.x=x;}let o=new C(42);o.x",
            Value::Number(42.0),
        ),
        (
            "function C(){this.x=42;return 3;}new C().x",
            Value::Number(42.0),
        ),
        (
            "function C(){this.x=1;return {x:42};}new C().x",
            Value::Number(42.0),
        ),
        (
            "function C(){};Object.getPrototypeOf(new C)===C.prototype",
            Value::Boolean(true),
        ),
        (
            "function C(){};C.prototype.x=42;(new C).x",
            Value::Number(42.0),
        ),
        (
            "function C(){};C.prototype=1;Object.getPrototypeOf(new C)===Object.prototype",
            Value::Boolean(true),
        ),
        (
            "let ns={C:function(){this.x=42;}};new ns.C().x",
            Value::Number(42.0),
        ),
        (
            "function C(){};C.prototype.constructor===C",
            Value::Boolean(true),
        ),
        (
            "function C(){};Object.hasOwn(C,'prototype')",
            Value::Boolean(true),
        ),
        (
            "function C(){};Object.getOwnPropertyDescriptor(C,'prototype').configurable",
            Value::Boolean(false),
        ),
        (
            "function C(){try{return 1;}finally{this.x=42;}}new C().x",
            Value::Number(42.0),
        ),
        (
            "function C(){throw 42;}try{new C;}catch(e){e}",
            Value::Number(42.0),
        ),
        (
            "Object.getPrototypeOf(new Object)===Object.prototype",
            Value::Boolean(true),
        ),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
    for source in [
        "new (()=>1)",
        "new ({}).x",
        "new (1)",
        "new ({f(){}}).f",
        "new Array.isArray([])",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn inherited_elements_are_visited_and_result_elements_are_own_data() {
    assert_eq!(
        eval("Array.prototype[1]=40;let a=[1,,1];a.reduce((x,y)=>x+y,0)"),
        Ok(Value::Number(42.0))
    );
    assert_eq!(
        eval(
            "Object.defineProperty(Array.prototype,0,{value:99,writable:false});let a=[1].map(x=>42);Object.hasOwn(a,0) && a[0]===42"
        ),
        Ok(Value::Boolean(true))
    );
    assert_eq!(
        eval("let a=Array(4294967295);a.length=0;a.length"),
        Ok(Value::Number(0.0))
    );
    assert_eq!(
        eval("let a=[1,2,3];a.length=2;Object.keys(a).join()"),
        Ok(Value::string("0,1"))
    );
    assert_eq!(
        eval("let a=[];Object.preventExtensions(a);try{a.push(1);}catch(e){e.name}"),
        Ok(Value::string("TypeError"))
    );
}

#[test]
fn native_search_join_and_slice_limits_terminate_large_sparse_inputs() -> Result<(), Error> {
    let limits = Limits {
        fuel: 200,
        ..Limits::default()
    };
    for source in [
        "Array(4294967295).includes(1)",
        "Array(4294967295).slice()",
        "Array(4294967295).indexOf(1)",
    ] {
        assert!(
            matches!(
                Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost),
                Err(Error::Limit { .. })
            ),
            "{source}"
        );
    }
    let limits = Limits {
        string_units: 10,
        ..Limits::default()
    };
    assert!(matches!(
        Runtime::new(limits).run(&compile("Array(20).join()", limits)?, &mut SilentHost),
        Err(Error::Limit {
            resource: "string units"
        })
    ));
    Ok(())
}

#[test]
fn for_of_arrays_strings_and_pair_binding_patterns() {
    for (source, expected) in [
        ("let s=0;for(const x of [1,2,3])s+=x;s", Value::Number(6.0)),
        (
            "let s='';for(const x of 'a😀b')s+=x.length;s",
            Value::string("121"),
        ),
        (
            "let n=0;for(let x of [,undefined]){if(x===undefined)n++;}n",
            Value::Number(2.0),
        ),
        (
            "let s='';for(const [k,v] of Object.entries({b:2,a:1}))s+=k+v;s",
            Value::string("b2a1"),
        ),
        (
            "let s=0;for(const [a,[b,c]] of [[1,[2,3]],[4,[5,6]]])s+=a+b+c;s",
            Value::Number(21.0),
        ),
        (
            "let s='';for(const [a,b] of ['😀a','xy'])s+=a+b;s",
            Value::string("😀axy"),
        ),
        (
            "let a=[1,2];let s=0;for(let x of a){s+=x;if(x==1)a.push(3);}s",
            Value::Number(6.0),
        ),
        ("let o={};for(o.x of [1,42]){}o.x", Value::Number(42.0)),
        ("for(var [a,b] of [[40,2]]){}a+b", Value::Number(42.0)),
        (
            "let a;let b;for(let x of [1,2]){if(x==1)a=()=>x;else b=()=>x;}a()+b()",
            Value::Number(3.0),
        ),
        (
            "Object.values({a:40,b:2}).reduce((a,b)=>a+b,0)",
            Value::Number(42.0),
        ),
        (
            "let s=0;for(let x of [1,2,3]){try{if(x==2)continue;if(x==3)break;s+=x;}finally{s+=10;}}s",
            Value::Number(31.0),
        ),
        (
            "let n=0;for(let [a,,b] of [[40,999,2]])n=a+b;n",
            Value::Number(42.0),
        ),
        ("let x=1;for(let [a,b] of [[42]]){x=b;}x", Value::Undefined),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
    for source in [
        "for(let x of null){}",
        "for(let x of {}){}",
        "for(let [a] of [1]){}",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
    for source in [
        "for(const [a,a] of []){}",
        "for(let x of []){var x;}",
        "for(let x=1 of []){}",
    ] {
        assert!(compile(source, Limits::default()).is_err(), "{source}");
    }
}

#[test]
fn iteration_and_entry_values_remain_rooted_during_getters() -> Result<(), Error> {
    let limits = Limits {
        // Mutable Array methods are permanent intrinsic roots; temporary cycles
        // below still outnumber the remaining space and force collection.
        heap_entries: 64,
        ..Limits::default()
    };
    for source in [
        "let s=0;for(let x of [40,2]){for(let i=0;i<100;i++){let o={};o.o=o;}s+=x;}s",
        "let o={a:{x:42},get b(){delete this.a;for(let i=0;i<100;i++){let o={};o.o=o;}return 1;}};Object.entries(o)[0][1].x",
        "let o={get a(){return [40,2];}};let s=0;for(let [a,b] of Object.values(o)){s=a+b;}s",
    ] {
        assert_eq!(
            Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
            Value::Number(42.0),
            "{source}"
        );
    }
    let limits = Limits {
        fuel: 200,
        ..limits
    };
    assert!(matches!(
        Runtime::new(limits).run(
            &compile("for(let x of Array(4294967295)){}", limits)?,
            &mut SilentHost
        ),
        Err(Error::Limit { .. })
    ));
    Ok(())
}
