// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use super::{Builtin, Error, Execution, Property, Value, index_number};
use crate::{Limits, Runtime, SilentHost, compile};

fn eval(source: &str) -> Result<Value, Error> {
    let limits = Limits::default();
    Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)
}

#[test]
fn wide_array_iterator_indices_cross_u32_and_stop_at_safe_integer_limit() -> Result<(), Error> {
    // Seed internal slots directly to exercise astronomical boundary indices
    // without billions of next calls or a production test-only fast-forward API.
    for kind in [
        Builtin::ArrayKeys,
        Builtin::ArrayValues,
        Builtin::ArrayEntries,
    ] {
        for index in [
            4_294_967_294,
            4_294_967_295,
            4_294_967_296,
            9_007_199_254_740_989,
        ] {
            let mut host = SilentHost;
            let mut e = Execution::new(&mut host, Limits::default());
            e.fuel = 10_000;
            let proto = e.prototype()?;
            let source = e.allocate_object(proto)?;
            e.native_roots.push(source.clone());
            let length = index + 2;
            e.define(
                &source,
                Value::string("length").units(),
                Property::data(Value::Number(index_number(length))),
            )?;
            for k in index..length {
                e.define(
                    &source,
                    Value::string(&format!("{k}")).units(),
                    Property::data(Value::string(&format!("v{k}"))),
                )?;
            }
            let iterator = e
                .iterator_call(kind, &source)?
                .ok_or(Error::InvalidBytecode)?;
            e.native_roots.push(iterator.clone());
            e.object_mut(&iterator)?
                .iterator
                .as_mut()
                .ok_or(Error::InvalidBytecode)?
                .index = index;
            for k in index..length {
                let result = e.builtin_iterator_next(&iterator, Builtin::ArrayIteratorNext)?;
                e.native_roots.push(result.clone());
                assert_eq!(
                    e.get(&result, &Value::string("done").units())?,
                    Value::Boolean(false)
                );
                let value = e.get(&result, &Value::string("value").units())?;
                assert_yield(&mut e, kind, &value, k)?;
                assert_eq!(
                    e.object_ref(&iterator)?
                        .iterator
                        .as_ref()
                        .ok_or(Error::InvalidBytecode)?
                        .index,
                    k + 1
                );
            }
            let done = e.builtin_iterator_next(&iterator, Builtin::ArrayIteratorNext)?;
            e.native_roots.push(done.clone());
            assert_eq!(
                e.get(&done, &Value::string("done").units())?,
                Value::Boolean(true)
            );
            assert_eq!(
                e.get(&done, &Value::string("value").units())?,
                Value::Undefined
            );
            assert!(matches!(
                e.object_ref(&iterator)?
                    .iterator
                    .as_ref()
                    .ok_or(Error::InvalidBytecode)?
                    .source,
                Value::Undefined
            ));
            e.define(
                &source,
                Value::string("length").units(),
                Property::data(Value::Number(f64::INFINITY)),
            )?;
            let done = e.builtin_iterator_next(&iterator, Builtin::ArrayIteratorNext)?;
            assert_eq!(
                e.get(&done, &Value::string("done").units())?,
                Value::Boolean(true)
            );
        }
    }
    Ok(())
}

fn assert_yield(
    e: &mut Execution<'_>,
    kind: Builtin,
    value: &Value,
    index: u64,
) -> Result<(), Error> {
    match kind {
        Builtin::ArrayKeys => assert_eq!(*value, Value::Number(index_number(index))),
        Builtin::ArrayValues => assert_eq!(*value, Value::string(&format!("v{index}"))),
        _ => {
            assert_eq!(
                e.get(value, &Value::string("0").units())?,
                Value::Number(index_number(index))
            );
            assert_eq!(
                e.get(value, &Value::string("1").units())?,
                Value::string(&format!("v{index}"))
            );
        }
    }
    Ok(())
}

#[test]
fn live_length_wide_receivers_and_exception_state_follow_the_spec() {
    for source in [
        "let o={length:Infinity,0:7,1:8};let i=Array.prototype.values.call(o);i.next().value===7&&i.next().value===8",
        "let o={length:4294967297,0:7};Array.prototype.keys.call(o).next().value===0&&Array.prototype.entries.call(o).next().value.join()==='0,7'",
        "let n=0,o={get length(){n++;return 2.9},0:7,1:8};let i=Array.prototype.values.call(o);n===0&&i.next().value===7&&i.next().value===8&&i.next().done&&i.next().done&&n===3",
        "let n=0,o={get length(){if(n++===0)throw 7;return 1},0:8};let i=Array.prototype.values.call(o);try{i.next()}catch(e){}i.next().value===8&&i.next().done",
        "let n=0,o={length:{valueOf(){if(n++===0)throw 7;return 1}},0:8};let i=Array.prototype.values.call(o);try{i.next()}catch(e){}i.next().value===8",
        "let o={length:2,get 0(){throw 7},1:8},i=Array.prototype.values.call(o);try{i.next()}catch(e){}i.next().value===8&&i.next().done",
        "let o={length:2,get 0(){throw 7},1:8},i=Array.prototype.entries.call(o);try{i.next()}catch(e){}i.next().value.join()==='1,8'",
        "let o={length:1,0:7},i=Array.prototype.values.call(o);Object.freeze(i);i.next().value===7&&i.next().done",
        "let o={length:1,0:7},i=Array.prototype.values.call(o);o.length=0;i.next().done&&((o.length=2),i.next().done)",
        "let o={length:2,get 0(){throw 7},get 1(){throw 8}},i=Array.prototype.keys.call(o);i.next().value===0&&i.next().value===1",
        "let o=Object.create({1:7});o.length=2;let i=Array.prototype.values.call(o);i.next().value===undefined&&i.next().value===7",
        "let a=[];a.length=2;let i=a.entries();let r=i.next().value;Object.keys(r).join()==='0,1'&&r[0]===0&&r[1]===undefined",
        "let i=Array.prototype.values.call('😀');i.next().value==='\\ud83d'&&i.next().value==='\\ude00'&&i.next().done",
        "let i='😀'[Symbol.iterator]();i.next().value==='😀'&&i.next().done",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn reentrant_length_and_element_getters_snapshot_index_but_not_future_values() {
    for source in [
        "let log='',n=0,i,o={get length(){log+='l';if(n++===0){let inner=i.next();log+=inner.value}return 2},0:'a',1:'b'};i=Array.prototype.values.call(o);let first=i.next().value,second=i.next().value;first==='a'&&second==='b'&&log==='llal'",
        "let n=0,i,o={get length(){if(n++===0){let inner=i.next();if(!inner.done)throw 7;return 1}return 0},0:8};i=Array.prototype.values.call(o);i.next().value===8&&i.next().done",
        "let i,o={length:2,get 0(){return i.next().value},1:8};i=Array.prototype.values.call(o);i.next().value===8&&i.next().done",
        "let i,o={length:2,get 0(){return i.next().value[1]},1:8};i=Array.prototype.entries.call(o);i.next().value.join()==='0,8'&&i.next().done",
        "let i,o={length:2,get 0(){i.next();throw 7},1:8};i=Array.prototype.values.call(o);try{i.next()}catch(e){}i.next().done",
        "let i,o={get length(){Object.defineProperty(this,'length',{value:1});try{i.next()}catch(e){}return 1},get 0(){throw 7}};i=Array.prototype.values.call(o);try{i.next()}catch(e){}i.next().done",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn iterator_intrinsics_are_mutable_and_aliases_do_not_reappear() {
    for source in [
        "Array.prototype.values===Array.prototype[Symbol.iterator]",
        "let f=Array.prototype.values;delete f.name;f.extra=7;f===Array.prototype[Symbol.iterator]&&!Object.hasOwn(f,'name')&&f.extra===7",
        "let f=Array.prototype.values;delete Array.prototype.values;!Object.hasOwn(Array.prototype,'values')&&Array.prototype[Symbol.iterator]===f&&[...new Array(2)].length===2",
        "let f=Array.prototype.values;Array.prototype.values=7;Array.prototype[Symbol.iterator]===f&&[...new Array(2)].length===2",
        "delete Array.prototype[Symbol.iterator];let i=[1].values();i.next().value===1&&!Object.hasOwn(Array.prototype,Symbol.iterator)",
        "let p=Object.getPrototypeOf([].values()),f=p.next;delete f.name;f.x=7;p.next===f&&!Object.hasOwn(f,'name')&&f.x===7",
        "let p=Object.getPrototypeOf([].values()),f=p.next;delete p.next;let i=[1].values();i.next===undefined&&f.call(i).value===1",
        "let p=Object.getPrototypeOf(Object.getPrototypeOf([].values())),f=p[Symbol.iterator];f.call(7)===7&&f.call(null)===null",
        "let f=Array.prototype[Symbol.iterator];function check(){return arguments[Symbol.iterator]===f}Array.prototype.values=7;delete Array.prototype[Symbol.iterator];check()",
        "let p=Object.getPrototypeOf([][Symbol.iterator]()),s=Object.getPrototypeOf(''[Symbol.iterator]());Object.getPrototypeOf(p)===Object.getPrototypeOf(s)&&p.next!==s.next",
        "let p=Object.getPrototypeOf([].values()),d=Object.getOwnPropertyDescriptor(p,Symbol.toStringTag);d.value==='Array Iterator'&&!d.writable&&!d.enumerable&&d.configurable",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for (expr, name) in [
        ("Array.prototype.keys", "keys"),
        ("Array.prototype.values", "values"),
        ("Array.prototype.entries", "entries"),
        ("[].values().next", "next"),
        ("''[Symbol.iterator]().next", "next"),
        ("String.prototype[Symbol.iterator]", "[Symbol.iterator]"),
    ] {
        let source = format!(
            "let f={expr},l=Object.getOwnPropertyDescriptor(f,'length'),n=Object.getOwnPropertyDescriptor(f,'name');f.name==='{name}'&&f.length===0&&!l.writable&&!l.enumerable&&l.configurable&&!n.writable&&!n.enumerable&&n.configurable&&f.prototype===undefined"
        );
        assert_eq!(eval(&source), Ok(Value::Boolean(true)), "{source}");
        assert!(matches!(
            eval(&format!("new ({expr})()")),
            Err(Error::Type { .. })
        ));
    }
}

#[test]
fn wide_iterator_gc_and_fatal_resource_limits() -> Result<(), Error> {
    let limits = Limits {
        heap_entries: 180,
        ..Limits::default()
    };
    for source in [
        "let i=Array.prototype.values.call({length:Infinity,get 0(){for(let j=0;j<300;j++){let g={}}return {n:7}}});i.next().value.n===7",
        "let i=Array.prototype.entries.call({length:Infinity,get 0(){for(let j=0;j<300;j++){let g={}}return {n:7}}});i.next().value[1].n===7",
        "let f=Array.prototype.values;f.saved={n:7};delete Array.prototype.values;for(let i=0;i<300;i++){let g={}}Array.prototype[Symbol.iterator]===f&&f.saved.n===7",
        "let n=0,o={length:Infinity,0:7,1:8};for(let v of Array.prototype.values.call(o)){n+=v;if(v===8)break}n===15",
        "let i=[1].values();i.next();i.next();for(let n=0;n<300;n++){let g={}}i.next().done",
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
        "Array.from(Array.prototype.values.call({length:Infinity}))",
        "let i;let o={get length(){return i.next().value}};i=Array.prototype.values.call(o);i.next()",
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
