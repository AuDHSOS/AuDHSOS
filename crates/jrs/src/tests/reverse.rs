// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use super::{Error, Limits, Runtime, SilentHost, Value, compile, eval};

#[test]
fn reverse_preserves_holes_inheritance_identity_and_length() {
    for source in [
        "let a=[1,2,3,4];a.reverse()===a&&a.join()==='4,3,2,1'&&a.length===4",
        "let a=[,2,,4];a.reverse();a.length===4&&a[0]===4&&a[2]===2&&!(1 in a)&&!(3 in a)",
        "let a=[1,,,];a.reverse();a.length===3&&!(0 in a)&&a[2]===1",
        "let a=[,,];a.reverse();Object.keys(a).length===0&&a.length===2",
        "Array.prototype[0]=7;let a=[,,3];a.reverse();a[0]===3&&a[2]===7&&Object.hasOwn(a,2)",
        "let a={length:4,0:'a',3:'d'};Array.prototype.reverse.call(a)===a&&a[0]==='d'&&a[3]==='a'",
        "let o={get length(){return 2},0:1,1:2};Array.prototype.reverse.call(o);o[0]===2&&o[1]===1",
        "let o={length:2.9,0:1,1:2,2:3};Array.prototype.reverse.call(o);o[0]===2&&o[1]===1&&o[2]===3&&o.length===2.9",
        "let o={length:-1,get 0(){throw 7}};Array.prototype.reverse.call(o)===o",
        "Array.prototype.reverse.call(true).valueOf()===true",
        "Array.prototype.reverse.call('a').valueOf()==='a'",
        "let a=[1];Object.freeze(a);a.reverse()===a",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn reverse_observes_order_and_keeps_partial_mutations_on_errors() {
    for source in [
        "let log='';let o={length:2,get 0(){log+='l';return 1},get 1(){log+='u';return 2},set 0(v){log+='L'+v},set 1(v){log+='U'+v}};Array.prototype.reverse.call(o);log==='luL2U1'",
        "let a=['first','second'];Object.defineProperty(a,0,{get(){a.length=0;return 'first'}});a.reverse();!(0 in a)&&a[1]==='first'",
        "let a=[1,2];Object.defineProperty(a,1,{writable:false});let yes=false;try{a.reverse()}catch(e){yes=e instanceof TypeError}yes&&a[0]===2&&a[1]===2",
        "let a=[1,,];Object.defineProperty(a,0,{configurable:false});let yes=false;try{a.reverse()}catch(e){yes=e instanceof TypeError}yes&&a[0]===1&&!(1 in a)",
        "let a=[,2];Object.defineProperty(a,1,{configurable:false});let yes=false;try{a.reverse()}catch(e){yes=e instanceof TypeError}yes&&a[0]===2&&a[1]===2",
        "let a=[1,,];Object.preventExtensions(a);try{a.reverse()}catch(e){}!(0 in a)&&!(1 in a)",
        "let a=[,2];Object.preventExtensions(a);try{a.reverse()}catch(e){}!(0 in a)&&a[1]===2",
        "let n=0;let o={length:2,get 0(){throw 7},get 1(){n++}};try{Array.prototype.reverse.call(o)}catch(e){}n===0",
        "let a=[1,2,3];Object.defineProperty(a,1,{get(){throw 7}});a.reverse();a[0]===3&&a[2]===1",
        "let n=0,o={get length(){n++;return {valueOf(){n++;return 2}}},0:1,1:2};Array.prototype.reverse.call(o);n===2",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "Array.prototype.reverse.call(null)",
        "Array.prototype.reverse.call(undefined)",
        "Array.prototype.reverse.call('ab')",
        "[1,2].reverse.call(Object.freeze([1,2]))",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn last_index_of_uses_strict_equality_and_distinguishes_missing_from_undefined() {
    for source in [
        "let a=[1,2,1];a.lastIndexOf(1)===2&&a.lastIndexOf(1,undefined)===0&&a.lastIndexOf(1,NaN)===0",
        "[1,2,1].lastIndexOf(1,Infinity)===2&&[1,2,1].lastIndexOf(1,-Infinity)===-1",
        "[1,2,1].lastIndexOf(1,-1)===2&&[1,2,1].lastIndexOf(1,-2)===0&&[1,2,1].lastIndexOf(1,-4)===-1",
        "[1,2,1].lastIndexOf(1,-0.5)===0&&[1,2,1].lastIndexOf(1,1.9)===0",
        "[NaN].lastIndexOf(NaN)===-1&&[-0].lastIndexOf(0)===0&&[1].lastIndexOf('1')===-1",
        "[,,].lastIndexOf(undefined)===-1&&[,undefined].lastIndexOf(undefined)===1",
        "Array.prototype[1]=undefined;[,,].lastIndexOf(undefined)===1",
        "let o={},a=[o,{},o];a.lastIndexOf(o)===2",
        "Array.prototype.lastIndexOf.call('abca','a')===3",
        "[].lastIndexOf(1,{valueOf(){throw 7}})===-1",
        "let a=[1,2,1];a.lastIndexOf(1,{valueOf(){a.length=0;return 2}})===-1",
        "let n=0,o={get length(){n++;return 3},get 2(){n++;return 7},get 1(){throw 9}};Array.prototype.lastIndexOf.call(o,7)===2&&n===2",
        "let s=Symbol();[s].lastIndexOf(s)===0",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "Array.prototype.lastIndexOf.call(null,1)",
        "[1].lastIndexOf(1,Symbol())",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })));
    }
}

#[test]
fn wide_indices_quota_and_native_temporaries_are_safe() -> Result<(), Error> {
    for source in [
        "let o={length:4294967297,4294967296:7};Array.prototype.lastIndexOf.call(o,7)===4294967296",
        "let o={length:9007199254740992,9007199254740990:7};Array.prototype.lastIndexOf.call(o,7)===9007199254740990",
        "let seen=false,o={length:9007199254740992,get 9007199254740990(){seen=true;throw 7}};try{Array.prototype.reverse.call(o)}catch(e){}seen",
        "let seen=false,o={length:4294967297,get 4294967296(){seen=true;throw 7}};try{Array.prototype.reverse.call(o)}catch(e){}seen",
        "let a=[{n:1},2];Object.defineProperty(a,1,{get(){for(let i=0;i<300;i++){let g={}}return {n:2}},set(v){if(v.n!==1)throw 7}});a.reverse();a[0].n===2",
    ] {
        let limits = Limits {
            heap_entries: 150,
            ..Limits::default()
        };
        assert_eq!(
            Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
            Value::Boolean(true),
            "{source}"
        );
    }
    for source in [
        "Array.prototype.reverse.call({length:Infinity})",
        "Array.prototype.lastIndexOf.call({length:Infinity},7)",
    ] {
        let limits = Limits {
            fuel: 3000,
            ..Limits::default()
        };
        assert!(matches!(
            Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost),
            Err(Error::Limit { .. })
        ));
    }
    for source in [
        "Array.prototype.reverse.name==='reverse'&&Array.prototype.reverse.length===0",
        "Array.prototype.lastIndexOf.name==='lastIndexOf'&&Array.prototype.lastIndexOf.length===1",
        "let f=Array.prototype.reverse;f.x=7;delete f.name;f.x===7&&!Object.hasOwn(f,'name')",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)));
    }
    Ok(())
}

#[test]
fn enumerable_values_and_entries_snapshot_keys_but_recheck_descriptors() {
    for source in [
        "JSON.stringify(Object.values({2:'b',1:'a',x:3}))==='[\"a\",\"b\",3]'",
        "JSON.stringify(Object.entries('ab'))==='[[\"0\",\"a\"],[\"1\",\"b\"]]'",
        "let o={get a(){delete this.b;this.c=3;return 1},b:2};Object.values(o).join()==='1'",
        "let o={get a(){Object.defineProperty(this,'b',{enumerable:false});return 1},b:2};Object.entries(o).length===1",
        "let o={a:1};o[Symbol()]=2;Object.defineProperty(o,'x',{value:3});Object.values(o).join()==='1'",
        "Object.values(Object.create({x:1})).length===0&&Object.entries(7).length===0",
        "Object.entries.length===1&&Object.entries.name==='entries'&&Object.values.name==='values'&&Object.isExtensible(Object.values)",
        "let f=Object.values;f.x=7;delete f.length;f===Object.values&&f.x===7&&!Object.hasOwn(f,'length')",
        "let d=Object.getOwnPropertyDescriptor(Object,'values');d.writable&&!d.enumerable&&d.configurable",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in ["Object.values(null)", "Object.entries(undefined)"] {
        assert!(matches!(eval(source), Err(Error::Type { .. })));
    }
    for method in ["indexOf", "includes"] {
        assert_eq!(
            eval(&format!(
                "let n=0;[].{method}(1,{{valueOf(){{n++;throw 7}}}});n===0"
            )),
            Ok(Value::Boolean(true))
        );
    }
}

#[test]
fn object_constructor_properties_are_mutable_rooted_and_not_recreated() -> Result<(), Error> {
    for source in [
        "let f=Object.entries;Object.entries=7;Object.entries===7&&f({a:1})[0][1]===1",
        "Object.keys=()=>{throw 7};Object.values({a:1})[0]===1&&Object.entries({a:1})[0][0]==='a'",
        "delete Object.entries;!Object.hasOwn(Object,'entries')&&Object.entries===undefined",
        "let n=0;Object.defineProperty(Object,'entries',{get(){n++;return 7}});Object.entries===7&&n===1",
        "let desc=Object.getOwnPropertyDescriptor(Object,'entries');desc.writable&&!desc.enumerable&&desc.configurable",
        "let f=Object.entries;Object.freeze(Object);Object.isFrozen(Object)&&!Object.isExtensible(Object)&&f({a:1})[0][1]===1",
        "Object.preventExtensions(Object);let yes=false;try{Object.defineProperty(Object,'extra',{value:1})}catch(e){yes=e instanceof TypeError}yes",
        "Object.setPrototypeOf(Object,{x:7});Object.x===7&&Object(3).valueOf()===3",
        "let s=Symbol();Object[s]={n:7};Object.getOwnPropertySymbols(Object)[0]===s&&Object[s].n===7",
        "let O=Object;Object.x=7;O===Object&&O.x===7&&Object.prototype.constructor===Object",
        "Object.keys(Object).length===0&&Object.getOwnPropertyNames(Object).includes('entries')",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    let limits = Limits {
        heap_entries: 170,
        ..Limits::default()
    };
    let source = "let o={get a(){return {n:7}},get b(){for(let i=0;i<300;i++){let g={}}return {n:8}}};let a=Object.entries(o);for(let i=0;i<300;i++){let g={}}a[0][1].n+a[1][1].n===15";
    assert_eq!(
        Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
        Value::Boolean(true)
    );
    let mut host = SilentHost;
    let mut realm = crate::Realm::new(limits, &mut host)?;
    realm.evaluate("Object.saved={n:7};let s=Symbol();Object[s]={n:8};delete Object.entries")?;
    assert_eq!(realm.evaluate("for(let i=0;i<300;i++){let g={}}Object.saved.n+Object[s].n===15&&Object.entries===undefined")?,Value::Boolean(true));
    Ok(())
}

#[test]
fn number_constants_cover_binary64_and_have_readonly_descriptors() {
    for source in [
        "Number.MAX_SAFE_INTEGER===9007199254740991&&Number.MIN_SAFE_INTEGER===-9007199254740991",
        "Number.MIN_VALUE===5e-324&&Number.MAX_VALUE===1.7976931348623157e308&&Number.EPSILON===2.220446049250313e-16",
        "isNaN(Number.NaN)&&Number.POSITIVE_INFINITY===Infinity&&Number.NEGATIVE_INFINITY===-Infinity",
        "let d=Object.getOwnPropertyDescriptor(Number,'MAX_SAFE_INTEGER');!d.writable&&!d.configurable&&!d.enumerable",
        "let o={length:Number.MAX_SAFE_INTEGER};o[Number.MAX_SAFE_INTEGER-3]=7;Array.prototype.lastIndexOf.call(o,7,Number.MAX_SAFE_INTEGER-1)===Number.MAX_SAFE_INTEGER-3",
        "Number.MAX_SAFE_INTEGER=1;Number.MAX_SAFE_INTEGER===9007199254740991",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn object_storage_symbol_descriptors_and_rejected_changes_are_consistent() -> Result<(), Error> {
    for source in [
        "Object.getOwnPropertySymbols(Object).length===0",
        "let O=Object,s=Symbol();Object.defineProperty(O,s,{value:7,writable:true,configurable:true});O[s]=8;delete O[s];Object.getOwnPropertySymbols(O).length===0",
        "let O=Object;Object.setPrototypeOf(O,null);let n=0;for(let key in O){n++}n===0&&Object.getPrototypeOf(O)===null",
        "Object.seal(Object);Object.isSealed(Object)&&!Object.isFrozen(Object)",
        "Object.preventExtensions(Object);Object.values=7;Object.values===7",
        "let O=Object;delete O.values;Object.defineProperty(O,'values',{value:7});O.values===7",
        "let yes=false;try{Object.setPrototypeOf(Object,Object)}catch(e){yes=e instanceof TypeError}yes",
        "let O=Object;O.extra=7;Object.keys(O).join()==='extra'&&Object.values(O)[0]===7",
        "let n=0;Object.defineProperty(Object,'extra',{get(){n++;return 7},enumerable:true});Object.entries(Object)[0][1]===7&&n===1",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "Array.prototype.reverse.call({get length(){throw 7}})",
        "Array.prototype.lastIndexOf.call({get length(){throw 7}},1)",
        "Object.entries({get a(){throw 7}})",
        "Object.values({get a(){throw 7}})",
    ] {
        assert!(
            matches!(
                eval(source),
                Err(Error::Thrown {
                    value: Value::Number(7.0)
                })
            ),
            "{source}"
        );
    }
    let limits = Limits {
        properties: 16,
        ..Limits::default()
    };
    assert!(matches!(
        Runtime::new(limits).run(&compile("Object.entries({})", limits)?, &mut SilentHost),
        Err(Error::Limit { .. })
    ));
    Ok(())
}
