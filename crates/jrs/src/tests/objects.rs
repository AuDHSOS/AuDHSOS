// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::eval;
use crate::{Error, Limits, Runtime, SilentHost, Value, compile};

#[test]
fn object_literals_keys_identity_and_references() {
    for (source, expected) in [
        ("let o={x:42};o.x", Value::Number(42.0)),
        ("let x=42;({x}).x", Value::Number(42.0)),
        ("let key='x';({[key]:42})[key]", Value::Number(42.0)),
        ("({1:42})['1']", Value::Number(42.0)),
        ("({'\\ud800':42})['\\ud800']", Value::Number(42.0)),
        ("({true:42}).true", Value::Number(42.0)),
        ("({x:1,x:42,}).x", Value::Number(42.0)),
        ("let o={};o.x=40;o['x']+=2;o.x", Value::Number(42.0)),
        ("let o={x:1};o.x++ + ++o.x", Value::Number(4.0)),
        ("let o={};let p=o;p.x=42;o.x", Value::Number(42.0)),
        ("let o={};o===o", Value::Boolean(true)),
        ("({})===({})", Value::Boolean(false)),
        ("typeof {}", Value::string("object")),
        ("Boolean({})", Value::Boolean(true)),
        ("'😀'.length", Value::Number(2.0)),
        ("'abc'[1]", Value::string("b")),
        ("'😀'[0] === '\\ud83d'", Value::Boolean(true)),
        ("({}).absent", Value::Undefined),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
}

#[test]
fn property_base_and_key_are_evaluated_once_before_value() {
    assert_eq!(
        eval(
            "let n=0;let o={x:1};function base(){n+=1;return o;}function key(){n+=10;return 'x';}base()[key()]+=2;n*10+o.x"
        ),
        Ok(Value::Number(113.0))
    );
    assert_eq!(
        eval("let o={x:1};let p=o;o.x=(o={x:9});p.x===o"),
        Ok(Value::Boolean(true))
    );
    assert_eq!(
        eval("let n=0;let o={};o[n++]=n;n+o[0]"),
        Ok(Value::Number(2.0))
    );
}

#[test]
fn method_receivers_and_lexical_arrow_this() {
    for (source, expected) in [
        (
            "let o={x:42,f(){return this.x;}};o.f()",
            Value::Number(42.0),
        ),
        (
            "let o={x:42,f:function(){return this.x;}};(o.f)()",
            Value::Number(42.0),
        ),
        (
            "let o={x:42,f(){return ()=>this.x;}};o.f()()",
            Value::Number(42.0),
        ),
        (
            "let o={x:42};function f(){return this.x;}f.call(o)",
            Value::Number(42.0),
        ),
        (
            "function f(){'use strict';return this;}f()",
            Value::Undefined,
        ),
        (
            "function f(){return this;}f()===globalThis",
            Value::Boolean(true),
        ),
        ("(()=>this)()===globalThis", Value::Boolean(true)),
        ("this===globalThis", Value::Boolean(true)),
        (
            "let o={f(a){return this.x+a;},x:40};o.f.call({x:1},41)",
            Value::Number(42.0),
        ),
        (
            "let o={x:42,f(){return ()=>this;}};o.f().call({})===o",
            Value::Boolean(true),
        ),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
}

#[test]
fn prototypes_and_property_attributes_follow_ordinary_rules() {
    for (source, expected) in [
        (
            "let p={x:42};let o=Object.create(p);o.x",
            Value::Number(42.0),
        ),
        (
            "let p={x:1};let o={__proto__:p};o.x=42;p.x+o.x",
            Value::Number(43.0),
        ),
        ("let p={x:42};({__proto__:p}).x", Value::Number(42.0)),
        (
            "Object.getPrototypeOf({__proto__:null})===null",
            Value::Boolean(true),
        ),
        (
            "Object.getPrototypeOf({})===Object.prototype",
            Value::Boolean(true),
        ),
        (
            "let o={};Object.setPrototypeOf(o,{x:42});o.x",
            Value::Number(42.0),
        ),
        ("let o={x:42};Object.hasOwn(o,'x')", Value::Boolean(true)),
        ("let o={x:42};o.hasOwnProperty('x')", Value::Boolean(true)),
        ("'x' in {__proto__:{x:42}}", Value::Boolean(true)),
        (
            "Object.hasOwn({__proto__:{x:1}},'x')",
            Value::Boolean(false),
        ),
        (
            "let o={};Object.defineProperty(o,'x',{value:42});o.x=3;o.x",
            Value::Number(42.0),
        ),
        (
            "let o={x:42};Object.freeze(o);o.x=3;delete o.x;o.x",
            Value::Number(42.0),
        ),
        (
            "let o={x:1};Object.seal(o);o.x=42;o.y=1;delete o.x;o.x",
            Value::Number(42.0),
        ),
        (
            "let o={};Object.preventExtensions(o);o.x=42;Object.hasOwn(o,'x')",
            Value::Boolean(false),
        ),
        ("Object.isExtensible({})", Value::Boolean(true)),
        (
            "Object.isExtensible(Object.freeze({}))",
            Value::Boolean(false),
        ),
        ("Object.is(NaN,NaN)", Value::Boolean(true)),
        ("Object.is(0,-0)", Value::Boolean(false)),
        (
            "let o={x:42};delete o.x;Object.hasOwn(o,'x')",
            Value::Boolean(false),
        ),
        (
            "let d=Object.getOwnPropertyDescriptor({x:42},'x');d.value+d.writable",
            Value::Number(43.0),
        ),
        ("({}).toString()", Value::string("[object Object]")),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
}

#[test]
fn object_errors_are_structured_and_early_errors_have_no_effects() {
    for source in [
        "({__proto__:null,__proto__:null})",
        "({x=1})",
        "({[1]})",
        "'use strict';delete x",
        "o.=1",
    ] {
        assert!(compile(source, Limits::default()).is_err(), "{source}");
    }
    for source in [
        "null.x",
        "undefined.x=1",
        "'x' in 1",
        "Object.create(1)",
        "let o={};Object.setPrototypeOf(o,o)",
        "'use strict';let o=Object.freeze({x:1});o.x=2",
        "'use strict';let o=Object.freeze({x:1});delete o.x",
        "Object.defineProperty({},'x',{get:1})",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn tracing_keeps_object_cycles_prototypes_and_arrow_receivers() -> Result<(), Error> {
    let limits = Limits {
        heap_entries: 32,
        ..Limits::default()
    };
    let source = "let keep={x:42,f(){return ()=>this.x;}}.f();for(let i=0;i<500;i++){let o={};o.self=o;o.f=()=>o;}keep()";
    assert_eq!(
        Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
        Value::Number(42.0)
    );
    let limits = Limits {
        properties: 3,
        ..Limits::default()
    };
    assert!(matches!(
        Runtime::new(limits).run(&compile("({a:1,b:2,c:3,d:4})", limits)?, &mut SilentHost),
        Err(Error::Limit {
            resource: "object properties"
        })
    ));
    Ok(())
}

#[test]
fn integrity_levels_and_nonconfigurable_descriptors() {
    for (source, expected) in [
        ("Object.isFrozen(Object.freeze({x:1}))", true),
        ("Object.isSealed(Object.seal({x:1}))", true),
        ("Object.isFrozen(Object.seal({x:1}))", false),
        ("Object.isSealed(Object.preventExtensions({x:1}))", false),
        ("Object.isFrozen(Object.preventExtensions({}))", true),
        ("Object.isFrozen(42)", true),
        ("Object.isSealed(null)", true),
        ("Object.hasOwn(42,'x')", false),
        ("delete (42).x", true),
        (
            "let o={};Object.defineProperty(o,'x',{value:NaN});Object.defineProperty(o,'x',{value:NaN});Object.is(o.x,NaN)",
            true,
        ),
        (
            "let p={};Object.defineProperty(p,'x',{value:42});let o=Object.create(p);o.x=1;Object.hasOwn(o,'x')",
            false,
        ),
        ("Object.getOwnPropertyDescriptor({},'x')===undefined", true),
        (
            "let o={__proto__:42};Object.getPrototypeOf(o)===Object.prototype",
            true,
        ),
        (
            "let o={['__proto__']:42};Object.hasOwn(o,'__proto__')",
            true,
        ),
        (
            "let o={};Object.preventExtensions(o);Object.setPrototypeOf(o,Object.getPrototypeOf(o))===o",
            true,
        ),
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(expected)), "{source}");
    }
    for source in [
        "let o={};Object.defineProperty(o,'x',{value:0});Object.defineProperty(o,'x',{value:-0})",
        "let o={};Object.defineProperty(o,'x',{value:1});Object.defineProperty(o,'x',{writable:true})",
        "let o=Object.freeze({x:1});Object.defineProperty(o,'x',{configurable:true})",
        "let o=Object.freeze({x:1});Object.defineProperty(o,'x',{enumerable:false})",
        "Object.setPrototypeOf(Object.preventExtensions({}),{})",
        "Object.setPrototypeOf({},42)",
        "delete null.x",
        "Object() .x[0]",
        "Object.create(null,{x:{get:1}})",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
    assert!(compile("o.'x'", Limits::default()).is_err());
}

#[test]
fn ordinary_key_order_and_redefinition_preserve_creation_order() -> Result<(), Error> {
    use crate::object::{Object, Property, array_index};
    let mut object = Object::new(Value::Null);
    for name in ["b", "10", "2", "a", "01", "4294967295"] {
        assert!(object.define(Value::string(name).units(), Property::data(Value::Null), 16)?);
    }
    let keys: Vec<_> = object.keys().into_iter().map(Value::String).collect();
    assert_eq!(
        keys,
        ["2", "10", "b", "a", "01", "4294967295"].map(Value::string)
    );
    assert!(object.delete(&Value::string("b").units()));
    assert!(object.define(Value::string("b").units(), Property::data(Value::Null), 16)?);
    assert_eq!(
        object.keys().last().cloned().map(Value::String),
        Some(Value::string("b"))
    );
    for name in ["", "-1", "1.0", "01", "4294967295", "999999999999999999"] {
        assert_eq!(array_index(&Value::string(name).units()), None);
    }
    Ok(())
}

#[test]
fn instanceof_walks_prototypes_without_coercing_left_operand() {
    for (source, expected) in [
        ("function C(){};new C instanceof C", true),
        ("function C(){};({}) instanceof C", false),
        ("[] instanceof Array", true),
        ("[] instanceof Object", true),
        ("42 instanceof Object", false),
        (
            "function C(){};let o=Object.create(C.prototype);o instanceof C",
            true,
        ),
        (
            "function C(){};let o=new C;C.prototype={};o instanceof C",
            false,
        ),
        ("function C(){};C.prototype=3;42 instanceof C", false),
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(expected)), "{source}");
    }
    for source in [
        "1 instanceof 1",
        "({}) instanceof {}",
        "({}) instanceof (()=>1)",
        "function C(){};C.prototype=3;({}) instanceof C",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn for_in_key_order_shadowing_deletion_and_binding_lifetimes() {
    for (source, expected) in [
        (
            "let s='';for(const k in {b:1,2:2,a:3}){s+=k;}s",
            Value::string("2ba"),
        ),
        (
            "let p={a:1,b:2};let o=Object.create(p);o.c=3;Object.defineProperty(o,'a',{value:4});let s='';for(let k in o)s+=k;s",
            Value::string("cb"),
        ),
        (
            "let o={a:1,b:2,c:3};let s='';for(let k in o){s+=k;if(k==='a')delete o.b;}s",
            Value::string("ac"),
        ),
        ("let s='';for(let k in 'abc'){s+=k;}s", Value::string("012")),
        (
            "let n=0;for(let k in null)n++;for(let k in undefined)n++;n",
            Value::Number(0.0),
        ),
        ("let k;for(k in {a:1,b:2}){}k", Value::string("b")),
        ("for(var k in {a:1}){}k", Value::string("a")),
        (
            "let a;let b;for(let k in {a:1,b:2}){if(k==='a')a=()=>k;else b=()=>k;}a()+b()",
            Value::string("ab"),
        ),
        ("let o={};for(o.k in {a:1,b:2}){}o.k", Value::string("b")),
        (
            "let s='';for(let a in {x:1,y:1}){for(let b in {a:1,b:1}){s+=a+b;}}s",
            Value::string("xaxbyayb"),
        ),
        (
            "let s='';for(let k in {a:1,b:2,c:3}){try{if(k==='b')continue;if(k==='c')break;s+=k;}finally{s+='!';}}s",
            Value::string("a!!!"),
        ),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
    for source in ["for(const k in {a:1})k='b'", "let k={};for(let k in k){}"] {
        assert!(eval(source).is_err(), "{source}");
    }
    for source in [
        "for(let k=1 in {}){}",
        "for(let k,x in {}){}",
        "for(1 in {}){}",
        "for(let k in {}){var k;}",
    ] {
        assert!(compile(source, Limits::default()).is_err(), "{source}");
    }
}
