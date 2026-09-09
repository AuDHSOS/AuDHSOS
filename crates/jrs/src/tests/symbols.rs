// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::{Error, Limits, Runtime, SilentHost, Value, compile, eval};

#[test]
fn symbol_resource_limits_and_rejected_properties_are_explicit() -> Result<(), Error> {
    for source in [
        "'use strict';let s=Symbol(),o={};Object.defineProperty(o,s,{get(){return 1}});o[s]=1",
        "'use strict';let s=Symbol(),o={};Object.defineProperty(o,s,{value:1});o[s]=1",
        "'use strict';let s=Symbol();(1)[s]=2",
        "let s=Symbol();Object.defineProperty(Object.preventExtensions({}),s,{value:1})",
        "let s=Symbol(),o={};Object.defineProperty(o,s,{value:1});Object.defineProperty(o,s,{enumerable:true})",
        "let s=Symbol(),o={};Object.defineProperty(o,s,{get(){return 1}});Object.defineProperty(o,s,{value:1})",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
    for source in [
        "let s=Symbol(),o={};Object.defineProperty(o,s,{value:1});Object.defineProperty(o,s,{value:1});o[s]===1",
        "let s=Symbol(),o={get [s](){return 3}};o[s]=1;o[s]===3",
        "let s=Symbol(),o={};Object.defineProperty(o,s,{set(v){}});o[s]===undefined",
        "let s=Symbol(),o=Object.create({[s]:1});s in o&&!Object.hasOwn(o,s)",
        "Object.getOwnPropertySymbols('x').length===0",
        "delete Symbol.prototype[Symbol.toStringTag];Object.prototype.toString.call(Object(Symbol()))==='[object Object]'",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    let limits = Limits {
        properties: 4,
        ..Limits::default()
    };
    for source in [
        "let o={};for(let i=0;i<5;i++)o[Symbol()]=i",
        "for(let i=0;i<5;i++)Symbol.for(String(i))",
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
        string_units: 7,
        ..Limits::default()
    };
    assert!(matches!(
        Runtime::new(limits).run(&compile("String(Symbol())", limits)?, &mut SilentHost),
        Err(Error::Limit { .. })
    ));
    Ok(())
}

#[test]
fn symbol_identity_registry_description_and_wrapper_brands() {
    for source in [
        "Symbol('x')!==Symbol('x')",
        "let s=Symbol();s===s && typeof s==='symbol' && Boolean(s)",
        "Symbol().description===undefined && Symbol('').description===''",
        "Symbol('\\ud800').description==='\\ud800'",
        "Symbol.for('x')===Symbol.for('x') && Symbol.for('x')!==Symbol('x')",
        "Symbol.keyFor(Symbol.for('x'))==='x' && Symbol.keyFor(Symbol('x'))===undefined",
        "Symbol.iterator===Symbol.iterator && Symbol.iterator!==Symbol.for('Symbol.iterator')",
        "Symbol.toStringTag.description==='Symbol.toStringTag'",
        "let s=Symbol('x');Object(s).valueOf()===s && Object(s)!==Object(s)",
        "Object.getPrototypeOf(Symbol())===Symbol.prototype && Object.getPrototypeOf(Symbol.prototype)===Object.prototype",
        "let d=Object.getOwnPropertyDescriptor(Symbol,'iterator');!d.writable&&!d.configurable&&!d.enumerable",
        "let s=Symbol('x');Object(s)==s && !(Object(s)===s)",
        "Object.freeze(Symbol())!==undefined && !Object.isExtensible(Symbol()) && Object.isFrozen(Symbol())",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for (source, expected) in [
        ("String(Symbol('x'))", "Symbol(x)"),
        ("String(Symbol())", "Symbol()"),
        (
            "Symbol.prototype.toString.call(Object(Symbol('x')))",
            "Symbol(x)",
        ),
        (
            "Object.prototype.toString.call(Symbol('x'))",
            "[object Symbol]",
        ),
        (
            "Object.prototype.toString.call(Symbol.prototype)",
            "[object Symbol]",
        ),
    ] {
        assert_eq!(eval(source), Ok(Value::string(expected)), "{source}");
    }
    for source in [
        "new Symbol()",
        "Symbol.keyFor(1)",
        "Symbol.prototype.valueOf()",
        "Symbol.prototype.toString.call({})",
        "Symbol.prototype.description",
        "Number(Symbol())",
        "''+Symbol()",
        "`${Symbol()}`",
        "+Symbol()",
        "-Symbol()",
        "~Symbol()",
        "Symbol()<1",
        "Symbol()-1",
        "Symbol()|1",
        "String(Object(Symbol()))",
        "Symbol(Symbol())",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn symbol_properties_do_not_collide_and_follow_descriptor_and_order_rules() {
    for source in [
        "let a=Symbol('x'),b=Symbol('x');let o={[a]:1,[b]:2,'Symbol(x)':3};o[a]===1&&o[b]===2&&o['Symbol(x)']===3",
        "let s=Symbol(),o={[s]:1};o[s]+=2;o[s]++;++o[s];o[s]===5&&s in o&&delete o[s]&&!(s in o)",
        "let s=Symbol(),o={};Object.defineProperty(o,s,{value:7});let d=Object.getOwnPropertyDescriptor(o,s);d.value===7&&!d.writable&&!d.enumerable&&!d.configurable",
        "let s=Symbol(),o={[s]:1};Object.hasOwn(o,s)&&o.hasOwnProperty(s)&&!Object.keys(o).length&&!Object.getOwnPropertyNames(o).length",
        "let a=Symbol(),b=Symbol(),o={[a]:1,[b]:2};delete o[a];o[a]=3;let k=Object.getOwnPropertySymbols(o);k[0]===b&&k[1]===a",
        "let s=Symbol(),o=Object.create(null,{[s]:{value:3,enumerable:true}});o[s]===3&&Object.getOwnPropertySymbols(o)[0]===s",
        "let s=Symbol(),o={};Object.defineProperties(o,{[s]:{get(){return 4}}});o[s]===4",
        "let s=Symbol(),o={[s]:1};Object.freeze(o);o[s]=9;o[s]===1&&Object.isFrozen(o)&&!delete o[s]",
        "let s=Symbol(),o={[s]:1};Object.seal(o);o[s]=9;o[s]===9&&Object.isSealed(o)&&!Object.isFrozen(o)",
        "let s=Symbol(),o=Object.preventExtensions({});o[s]=1;!Object.hasOwn(o,s)",
        "let s=Symbol(),p={get [s](){return this.x}},o=Object.create(p);o.x=7;o[s]===7",
        "let s=Symbol(),v;let p={set [s](x){v=this}},o=Object.create(p);o[s]=1;v===o",
        "let s=Symbol();Number.prototype[s]=3;(1)[s]===3",
        "let s=Symbol(),v;Object.defineProperty(Number.prototype,s,{set(x){'use strict';v=this}});(1)[s]=9;v===1",
        "let s=Symbol(),o={[s]:3,x:1},k='';for(let p in o)k+=p;k==='x'",
        "let s=Symbol(),o={[s]:()=>8};o[s]()===8",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "let s=Symbol();Object.defineProperty({},s,{get:3})",
        "let s=Symbol(),o={};Object.defineProperty(o,s,{value:1});Object.defineProperty(o,s,{value:2})",
        "'use strict';let s=Symbol();Object.freeze({})[s]=1",
        "let s=Symbol();null[s]",
        "let s=Symbol();s in 1",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn well_known_to_primitive_and_tag_hooks_observe_hints_and_original_receiver() {
    for (source, expected) in [
        (
            "let hints='';let o={[Symbol.toPrimitive](h){hints+=h+',';return 3}};o+1;Number(o);String(o);o<5;o==3;hints",
            "default,number,string,number,default,",
        ),
        (
            "let s=Symbol('x'),o={[Symbol.toPrimitive](){return s}};let a={[o]:7};String(a[s])",
            "7",
        ),
        (
            "let o={[Symbol.toStringTag]:'Custom'};Object.prototype.toString.call(o)",
            "[object Custom]",
        ),
        (
            "let o={[Symbol.toStringTag]:3};Object.prototype.toString.call(o)",
            "[object Object]",
        ),
        (
            "let p={get [Symbol.toStringTag](){return this.x}},o=Object.create(p);o.x='Mine';Object.prototype.toString.call(o)",
            "[object Mine]",
        ),
        (
            "let v;Object.defineProperty(Number.prototype,Symbol.toStringTag,{get(){'use strict';v=this;return 'N'}});Object.prototype.toString.call(3);String(v instanceof Number)",
            "true",
        ),
        (
            "Object.prototype.toString.call(new WeakMap())",
            "[object WeakMap]",
        ),
        (
            "Object.prototype.toString.call(WeakMap.prototype)",
            "[object WeakMap]",
        ),
        (
            "delete WeakMap.prototype[Symbol.toStringTag];Object.prototype.toString.call(new WeakMap())",
            "[object Object]",
        ),
    ] {
        assert_eq!(eval(source), Ok(Value::string(expected)), "{source}");
    }
    for source in [
        "Number({[Symbol.toPrimitive]:3})",
        "Number({[Symbol.toPrimitive](){return {}}})",
        "Object.prototype.toString.call({get [Symbol.toStringTag](){throw 7}})",
    ] {
        assert!(eval(source).is_err(), "{source}");
    }
}

#[test]
fn symbol_gc_traces_keys_and_wrappers_but_not_weakmap_keys() -> Result<(), Error> {
    let limits = Limits {
        heap_entries: 90,
        weak_entries: 3,
        ..Limits::default()
    };
    for (source, expected) in [
        (
            "let o={};o[Symbol('x')]={n:7};for(let i=0;i<200;i++){let t=Symbol()}let s=Object.getOwnPropertySymbols(o)[0];o[s].n",
            Value::Number(7.0),
        ),
        (
            "let o=Object(Symbol('x'));for(let i=0;i<200;i++){let t=Symbol()}o.valueOf().description",
            Value::string("x"),
        ),
        (
            "let m=new WeakMap();for(let i=0;i<200;i++){let s=Symbol();m.set(s,{s})}true",
            Value::Boolean(true),
        ),
        (
            "let m=new WeakMap(),s=Symbol();m.set(s,{n:7});for(let i=0;i<200;i++){let t=Symbol()}m.get(s).n",
            Value::Number(7.0),
        ),
        (
            "let m=new WeakMap();m.set(Symbol.iterator,{n:9});for(let i=0;i<200;i++){let t=Symbol()}m.get(Symbol.iterator).n",
            Value::Number(9.0),
        ),
        (
            "let s=Symbol.for('x');for(let i=0;i<200;i++){let t=Symbol()}Symbol.for('x')===s",
            Value::Boolean(true),
        ),
        (
            "let o={};let key={ [Symbol.toPrimitive](){return Symbol('k')} };Object.defineProperty(o,key,{get value(){for(let i=0;i<200;i++){let t=Symbol()}return 9}});o[Object.getOwnPropertySymbols(o)[0]]",
            Value::Number(9.0),
        ),
    ] {
        assert_eq!(
            Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
            expected,
            "{source}"
        );
    }
    assert!(matches!(
        eval("new WeakMap().set(Symbol.for('x'),1)"),
        Err(Error::Type { .. })
    ));
    assert_eq!(
        eval(
            "let m=new WeakMap();!m.has(Symbol.for('x'))&&m.get(Symbol.for('x'))===undefined&&!m.delete(Symbol.for('x'))"
        ),
        Ok(Value::Boolean(true))
    );
    Ok(())
}
