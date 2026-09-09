// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::{Error, Limits, Runtime, SilentHost, Value, compile, eval};

#[test]
fn wrappers_have_distinct_identity_slots_and_prototypes() {
    for (source, value) in [
        ("new Number(4).valueOf()", Value::Number(4.0)),
        ("new Number().valueOf()", Value::Number(0.0)),
        ("new Number(undefined).valueOf()", Value::Number(f64::NAN)),
        ("new Boolean(false).valueOf()", Value::Boolean(false)),
        ("new Boolean().valueOf()", Value::Boolean(false)),
        ("Boolean(new Boolean(false))", Value::Boolean(true)),
        ("new String(12).valueOf()", Value::string("12")),
        ("new String().valueOf()", Value::string("")),
        ("String(new Boolean(true))", Value::string("true")),
        ("Number(new String('42'))", Value::Number(42.0)),
        ("Number.prototype.valueOf()", Value::Number(0.0)),
        ("Boolean.prototype.valueOf()", Value::Boolean(false)),
        ("String.prototype.valueOf()", Value::string("")),
        (
            "let n=Object(-0);1/n.valueOf()",
            Value::Number(f64::NEG_INFINITY),
        ),
        (
            "Object.prototype.toString.call(Object('x'))",
            Value::string("[object String]"),
        ),
        (
            "Object.prototype.toString.call(Object(2))",
            Value::string("[object Number]"),
        ),
        (
            "Object.prototype.toString.call(Object(true))",
            Value::string("[object Boolean]"),
        ),
    ] {
        let actual = eval(source);
        if matches!(&value, Value::Number(n) if n.is_nan()) {
            assert!(
                matches!(actual, Ok(Value::Number(n)) if n.is_nan()),
                "{source}"
            );
        } else {
            assert_eq!(actual, Ok(value), "{source}");
        }
    }
    for source in [
        "Object(1)!==Object(1)",
        "Object(true) instanceof Boolean",
        "Object('x') instanceof String",
        "Object(3) instanceof Number",
        "Object.getPrototypeOf(1)===Number.prototype",
        "Object.getPrototypeOf(true)===Boolean.prototype",
        "Object.getPrototypeOf('x')===String.prototype",
        "Object.getPrototypeOf(Number.prototype)===Object.prototype",
        "let o=Object(3);Object(o)===o",
        "Object.prototype.valueOf.call(1) instanceof Number",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn primitive_accessors_preserve_receivers_and_sloppy_this_boxes_once() {
    for source in [
        "Number.prototype.x=7;(3).x===7",
        "Boolean.prototype.x=7;true.x===7",
        "String.prototype.x=7;'a'.x===7",
        "Object.defineProperty(Number.prototype,'x',{get(){'use strict';return this}});(3).x===3",
        "Object.defineProperty(Number.prototype,'x',{get(){return this}});(3).x instanceof Number",
        "let receiver;Object.defineProperty(Boolean.prototype,'x',{set(v){'use strict';receiver=this}});true.x=1;receiver===true",
        "let n=0;Object.defineProperty(String.prototype,'x',{set(v){n=v}});'a'.x=7;n===7",
        "function f(){return this===this && this instanceof Number}f.call(1)",
        "function f(){'use strict';return this===1}f.call(1)",
        "function f(){return ()=>this}let g=f.call(1);g()===g() && g() instanceof Number",
        "Number.prototype.x=4;delete Number.prototype.x;(1).x===undefined",
        "String.prototype.replace=7;'a'.replace===7",
        "Object.prototype.hasOwnProperty=7;true.hasOwnProperty===7",
        "let value;Object.defineProperty(Number.prototype,'x',{set(v){value=this}});(1).x=2;value instanceof Number",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn string_wrapper_indices_are_virtual_nonconfigurable_and_utf16() {
    for (source, expected) in [
        (
            "Object.getOwnPropertyNames(Object('ab')).join()",
            "0,1,length",
        ),
        ("Object.keys('ab').join()", "0,1"),
        ("Object.entries('ab').join('|')", "0,a|1,b"),
        ("Object.values('ab').join()", "a,b"),
        (
            "let s=Object('ab');s[8]='z';s.x=1;s['01']=2;s['-0']=3;Object.getOwnPropertyNames(s).join()",
            "0,1,8,length,x,01,-0",
        ),
        ("let s=Object('ab');s[0]='z';delete s[0];s[0]", "a"),
        (
            "let s=Object('ab');Object.defineProperty(s,0,{value:'a'});s[0]",
            "a",
        ),
        ("let s=Object('ab');Object.freeze(s);String(s)", "ab"),
        (
            "let out='';for(let c of Object('😀a'))out+=c+'|';out",
            "😀|a|",
        ),
        (
            "String.prototype.x=1;let out='';for(let k in 'ab')out+=k+',';out",
            "0,1,x,",
        ),
        (
            "Number.prototype.x=1;let out='';for(let k in 2)out+=k;out",
            "x",
        ),
    ] {
        assert_eq!(eval(source), Ok(Value::string(expected)), "{source}");
    }
    for source in [
        "let s=Object('😀');s.length===2 && s[0]==='\\ud83d' && s[1]==='\\ude00'",
        "let d=Object.getOwnPropertyDescriptor('a','0');d.value==='a'&&!d.writable&&!d.configurable&&d.enumerable",
        "let d=Object.getOwnPropertyDescriptor(Object('a'),'length');d.value===1&&!d.writable&&!d.configurable&&!d.enumerable",
        "Object.isFrozen(Object.freeze(Object('abc')))",
        "Object.keys(true).length===0 && Object.values(3).length===0",
        "Object.defineProperties({},true) instanceof Object",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn generic_arrays_box_once_before_length_getters_and_callbacks() {
    for (source, expected) in [
        ("Array.prototype.join.call('ab','-')", Value::string("a-b")),
        (
            "let seen;Object.defineProperty(Number.prototype,'length',{get(){'use strict';seen=this;return 0}});Array.prototype.join.call(1);seen instanceof Number",
            Value::Boolean(true),
        ),
        (
            "let n=0;Array.prototype.forEach.call('ab',(v,i,o)=>{if(o instanceof String)n++});n",
            Value::Number(2.0),
        ),
        (
            "Array.prototype.slice.call('abc',1).join()",
            Value::string("b,c"),
        ),
        (
            "Array.prototype.indexOf.call('abc','b')",
            Value::Number(1.0),
        ),
        (
            "Array.prototype.includes.call('abc','b')",
            Value::Boolean(true),
        ),
        (
            "Array.prototype.sort.call(true) instanceof Boolean",
            Value::Boolean(true),
        ),
        (
            "let a=Array.prototype.map.call('ab',(v,i,o)=>o instanceof String);a.every(v=>v)",
            Value::Boolean(true),
        ),
        (
            "String.prototype.split.call(Object('a,b'),',').join()",
            Value::string("a,b"),
        ),
        (
            "String.prototype.replace.call(Object('abc'),'b','z')",
            Value::string("azc"),
        ),
        (
            "String.prototype.match.call(Object('abc'),/b/)[0]",
            Value::string("b"),
        ),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
}

#[test]
fn boxing_failure_order_and_wrapper_brand_checks() {
    for source in [
        "Number.prototype.valueOf.call(true)",
        "Boolean.prototype.toString.call(1)",
        "String.prototype.valueOf.call({})",
        "Number.prototype.valueOf.call(Object.create(Number.prototype))",
        "Object.prototype.valueOf.call(null)",
        "Object.keys(null)",
        "Object.getPrototypeOf(undefined)",
        "'use strict';(2).x=1",
        "'use strict';delete Object('a')[0]",
        "Object.defineProperty(Object('a'),'0',{value:'b'})",
        "Object.defineProperty(Object('a'),'0',{writable:true})",
        "Object.defineProperty(Object('a'),'0',{configurable:true})",
        "Object.defineProperty(Object('a'),'0',{enumerable:false})",
        "Object.defineProperty(Object('a'),'0',{get(){return 'a'}})",
        "String.prototype.search.call(null,'a')",
        "Array.prototype.sort.call('ab')",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
    assert_eq!(
        eval(
            "let n=0;try{Number.prototype.toString.call(true,{valueOf(){n++;return 10}})}catch(e){}n"
        ),
        Ok(Value::Number(0.0))
    );
    assert_eq!(eval("(12).toString(10.9)"), Ok(Value::string("12")));
    for source in [
        "(1).toString(1)",
        "(1).toString(NaN)",
        "(1).toString(Infinity)",
    ] {
        assert!(matches!(eval(source), Err(Error::Range { .. })), "{source}");
    }
}

#[test]
fn constructor_and_math_properties_have_correct_attributes_and_order() {
    for source in [
        "let d=Object.getOwnPropertyDescriptor(Number,'prototype');d.value===Number.prototype&&!d.writable&&!d.enumerable&&!d.configurable",
        "let d=Object.getOwnPropertyDescriptor(Boolean,'prototype');d.value===Boolean.prototype&&!d.writable&&!d.enumerable&&!d.configurable",
        "let d=Object.getOwnPropertyDescriptor(String,'prototype');d.value===String.prototype&&!d.writable&&!d.enumerable&&!d.configurable",
        "Number.name==='Number' && Boolean.name==='Boolean' && String.name==='String'",
        "Number.length===1 && Boolean.length===1 && String.length===1",
        "Object.hasOwn(String,'prototype') && Object.hasOwn(Number,'length')",
        "Object.setPrototypeOf(1,{})===1",
        "let n=0;try{Object.hasOwn(null,{toString(){n++;return 'x'}})}catch(e){}n===0",
        "let n=0;try{Object.defineProperty(true,{toString(){n++;return 'x'}},{})}catch(e){}n===0",
        "let n=0;try{Object.prototype.hasOwnProperty.call(null,{toString(){n++;return 'x'}})}catch(e){}n===1",
        "Math.PI===3.141592653589793 && Math.E===2.718281828459045",
        "Math.LN2===0.6931471805599453 && Math.LN10===2.302585092994046",
        "Math.LOG2E===1.4426950408889634 && Math.LOG10E===0.4342944819032518",
        "Math.SQRT2===1.4142135623730951 && Math.SQRT1_2===0.7071067811865476",
        "let d=Object.getOwnPropertyDescriptor(Math,'PI');!d.writable&&!d.enumerable&&!d.configurable",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn wrappers_prototypes_and_virtual_indices_have_bounded_gc_safe_storage() -> Result<(), Error> {
    let limits = Limits {
        heap_entries: 70,
        ..Limits::default()
    };
    let source = "let n=Object(3);let b=Object(true);let s=Object('ab');Number.prototype.x={n:8};for(let i=0;i<200;i++){let t=Object(i);}n.valueOf()+n.x.n+s.length+b.valueOf()";
    assert_eq!(
        Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
        Value::Number(14.0)
    );
    let limits = Limits {
        // String.prototype now has the full basic method set; the independent
        // wrapper-index quota must be above that intrinsic property count.
        properties: 40,
        ..Limits::default()
    };
    let source = "let s=Object('abcdefghijklmnopqrstuvwxyz');s[25]";
    assert_eq!(
        Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
        Value::string("z")
    );
    let source = "Object.keys(Object('abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz'))";
    assert!(matches!(
        Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost),
        Err(Error::Limit { .. })
    ));
    Ok(())
}
