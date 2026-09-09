// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use super::{Error, Limits, Runtime, SilentHost, Value, compile, eval};

#[test]
fn fill_preserves_identity_and_uses_clamped_relative_indices() {
    for source in [
        "let a=[1,2,3];a.fill(7)===a&&a.join()==='7,7,7'&&a.length===3",
        "let a=[,2,,];a.fill(undefined,1);!(0 in a)&&(1 in a)&&(2 in a)&&a[1]===undefined",
        "let x={};let a=Array(3);a.fill(x);a[0]===x&&a[1]===x&&a[2]===x",
        "[1,2,3,4].fill(7,-3,-1).join()==='1,7,7,4'",
        "[1,2,3].fill(7,1.9,2.9).join()==='1,7,3'",
        "[1,2,3].fill(7,NaN,undefined).join()==='7,7,7'",
        "[1,2,3].fill(7,-Infinity,Infinity).join()==='7,7,7'",
        "[1,2,3].fill(7,Infinity,-Infinity).join()==='1,2,3'",
        "[1,2,3].fill(7,-20,-10).join()==='1,2,3'",
        "let a=[1,2];Object.freeze(a);a.fill(9,1,0)===a",
        "Array.prototype.fill.call(false,9).valueOf()===false",
        "let o={length:2.9};Array.prototype.fill.call(o,7)===o&&o[0]===7&&o[1]===7&&!(2 in o)&&o.length===2.9",
        "let o={length:-1};Array.prototype.fill.call(o,7)===o&&!Object.hasOwn(o,0)",
        "let n=0,v={valueOf(){n++},toString(){n++}};[1].fill(v)[0]===v&&n===0",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn copy_within_overlap_holes_and_generics() {
    for source in [
        "let a=[1,2,3,4];a.copyWithin(1,0,3)===a&&a.join()==='1,1,2,3'&&a.length===4",
        "[1,2,3,4].copyWithin(0,1).join()==='2,3,4,4'",
        "[1,2,3,4].copyWithin(-2,0,-1).join()==='1,2,1,2'",
        "[1,2,3,4].copyWithin(-Infinity,2,Infinity).join()==='3,4,3,4'",
        "[1,2,3,4].copyWithin(Infinity,0).join()==='1,2,3,4'",
        "[1,2,3].copyWithin(0,2,1).join()==='1,2,3'",
        "[1,2,3].copyWithin(-0.5,1.9,2.9).join()==='2,2,3'",
        "let a=[1,,3,4];a.copyWithin(2,0);a[2]===1&&!(3 in a)&&a.length===4",
        "let a=[1,,3,,];a.copyWithin(1,0,3);a[1]===1&&!(2 in a)&&a[3]===3",
        "Array.prototype[1]=7;let a=[1,,3];a.copyWithin(0,1,2);a[0]===7&&Object.hasOwn(a,0)",
        "let o={0:'a',1:'b',length:2};Array.prototype.copyWithin.call(o,1,0)===o&&o[1]==='a'&&o.length===2",
        "Array.prototype.copyWithin.call(true,0,1).valueOf()===true",
        "Array.prototype.copyWithin.call('',0,1).valueOf()===''",
        "let a=[1];Object.freeze(a);a.copyWithin(1,0)===a",
        "let a=[1,2];Object.defineProperty(a,'length',{writable:false});a.copyWithin(0,1)===a&&a.join()==='2,2'",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn conversion_order_length_snapshot_and_no_species_lookup() {
    for source in [
        "let log='';let o={get length(){log+='l';return 2}};Array.prototype.fill.call(o,7,{valueOf(){log+='s';return 0}},{valueOf(){log+='e';return 1}});log==='lse'&&o[0]===7",
        "let log='';let o={get length(){log+='l';return 0}};Array.prototype.copyWithin.call(o,{valueOf(){log+='t';return 0}},{valueOf(){log+='s';return 0}},{valueOf(){log+='e';return 0}});log==='ltse'",
        "let n=0;try{Array.prototype.fill.call(null,0,{valueOf(){n++}})}catch(e){}n===0",
        "let n=0;try{Array.prototype.copyWithin.call(null,{valueOf(){n++}})}catch(e){}n===0",
        "let n=0;try{[].fill(7,{valueOf(){throw 8}},{valueOf(){n++}})}catch(e){}n===0",
        "let n=0;try{[].copyWithin({valueOf(){throw 8}},{valueOf(){n++}})}catch(e){}n===0",
        "let n=0;try{[1].copyWithin(2,1,{valueOf(){n++;throw 7}})}catch(e){}n===1",
        "let a=[1,2,3];a.fill(7,{valueOf(){a.length=1;return 1}});a.length===3&&a.join()==='1,7,7'",
        "let a=[1,2,3,4];a.copyWithin(1,{valueOf(){a.length=2;return 0}},3);a.length===3&&a.join()==='1,1,2'",
        "let a=[1,2];Object.defineProperty(a,'constructor',{get(){throw 7}});a.copyWithin(0,1);a.fill(3);a.join()==='3,3'",
        "let o={get length(){return 2},set length(v){throw 7},0:1,1:2};Array.prototype.copyWithin.call(o,0,1);Array.prototype.fill.call(o,3);o[0]===3&&o[1]===3",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn property_effects_copy_direction_and_partial_mutations() {
    for source in [
        "let log='';let o={length:3,get 0(){log+='a';return 1},get 1(){log+='b';return 2},set 1(v){log+='B'+v},set 2(v){log+='C'+v}};Array.prototype.copyWithin.call(o,1,0,2);log==='bC2aB1'",
        "let log='';let o={length:3,get 1(){log+='b';return 2},get 2(){log+='c';return 3},set 0(v){log+='A'+v},set 1(v){log+='B'+v}};Array.prototype.copyWithin.call(o,0,1);log==='bA2cB3'",
        "let log='';let o={length:1,get 0(){log+='g';return 7},set 0(v){log+='s'+v}};Array.prototype.copyWithin.call(o,0,0);log==='gs7'",
        "let log='';let p={set 0(v){if(this!==o)throw 7;log+='a'+v},set 1(v){log+='b'+v}};let o=Object.create(p);o.length=2;Array.prototype.fill.call(o,3);log==='a3b3'&&!Object.hasOwn(o,0)",
        "let a=[1,2,3];Object.defineProperty(a,1,{writable:false});let ok=false;try{a.fill(7)}catch(e){ok=e instanceof TypeError}ok&&a.join()==='7,2,3'",
        "let a=[1,2,3];Object.defineProperty(a,1,{writable:false});try{a.copyWithin(0,1)}catch(e){}a.join()==='2,2,3'",
        "let a=[1,2,3];Object.defineProperty(a,2,{writable:false});try{a.copyWithin(1,0)}catch(e){}a.join()==='1,2,3'",
        "let a=[1,2,,];Object.defineProperty(a,1,{configurable:false});try{a.copyWithin(0,1)}catch(e){}a[0]===2&&a[1]===2",
        "let a=[1,2];Object.defineProperty(a,0,{get(){delete a[1];return 7},configurable:true});a.copyWithin(1,0);a[1]===7",
        "let p={0:7},o=Object.create(p);o.length=2;o[1]=9;Array.prototype.copyWithin.call(o,1,0);o[1]===7",
        "let p={1:7},o=Object.create(p);o.length=2;o[1]=9;Array.prototype.copyWithin.call(o,1,0);o[1]===7&&!Object.hasOwn(o,1)",
        "let a=[,,];Object.freeze(a);a.copyWithin(0,1)===a",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "Array.prototype.fill.call('a','x')",
        "Array.prototype.copyWithin.call('ab',0,1)",
        "[1].fill(1,Symbol())",
        "[1].fill(1,0,Symbol())",
        "[1].copyWithin(Symbol(),0)",
        "[1].copyWithin(0,Symbol())",
        "[1].copyWithin(0,0,Symbol())",
        "Array.prototype.fill.call({length:Symbol()},1)",
        "Array.prototype.copyWithin.call({length:Symbol()},0,0)",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn wide_indices_gc_and_fatal_limits() -> Result<(), Error> {
    for source in [
        "let x={},o={length:4294967298};Array.prototype.fill.call(o,x,4294967296);o[4294967296]===x&&o[4294967297]===x&&!Object.hasOwn(o,0)",
        "let n=Number.MAX_SAFE_INTEGER,o={length:Infinity};Array.prototype.fill.call(o,7,-2);o[n-2]===7&&o[n-1]===7&&!Object.hasOwn(o,n)",
        "let n=Number.MAX_SAFE_INTEGER,o={length:n};o[n-3]=7;o[n-1]=9;o[1]=3;Array.prototype.copyWithin.call(o,0,-3);o[0]===7&&!(1 in o)&&o[2]===9",
        "let n=Number.MAX_SAFE_INTEGER,o={length:n};o[n-3]=1;o[n-2]=2;o[n-1]=3;Array.prototype.copyWithin.call(o,-2,-3);o[n-2]===1&&o[n-1]===2",
        "let o={length:2,set 0(v){for(let i=0;i<300;i++){let g={}}if(v.n!==7)throw 7},set 1(v){this.saved=v}};Array.prototype.fill.call(o,{n:7});o.saved.n===7",
        "let o={length:2,get 0(){return {n:7}},set 1(v){for(let i=0;i<300;i++){let g={}}this.saved=v}};Array.prototype.copyWithin.call(o,1,0);o.saved.n===7",
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
        "let caught=false;try{Array.prototype.fill.call({length:Infinity},0)}catch(e){caught=true}",
        "Array.prototype.copyWithin.call({length:Infinity},0,1)",
    ] {
        let limits = Limits {
            fuel: 3000,
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
    Ok(())
}

#[test]
fn mutable_method_metadata_and_nonconstructibility() {
    for source in [
        "Array.prototype.fill.name==='fill'&&Array.prototype.fill.length===1&&Array.prototype.copyWithin.name==='copyWithin'&&Array.prototype.copyWithin.length===2",
        "let d=Object.getOwnPropertyDescriptor(Array.prototype,'fill');d.writable&&!d.enumerable&&d.configurable",
        "let f=Array.prototype.copyWithin;f.extra=7;delete f.name;!Object.hasOwn(f,'name')&&f.extra===7&&f.prototype===undefined",
        "let f=Array.prototype.fill;delete Array.prototype.fill;f.call([1],7)[0]===7&&!Object.hasOwn(Array.prototype,'fill')",
        "Object.getOwnPropertyNames(Array.prototype.fill).join()==='length,name'&&Object.isExtensible(Array.prototype.copyWithin)",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "new Array.prototype.fill()",
        "new Array.prototype.copyWithin()",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn unscopables_is_a_mutable_null_prototype_record_not_a_feature_mask() -> Result<(), Error> {
    for source in [
        "let u=Array.prototype[Symbol.unscopables];Object.getPrototypeOf(u)===null&&u.fill&&u.copyWithin&&!Object.hasOwn(u,'with')",
        "Object.keys(Array.prototype[Symbol.unscopables]).join()==='at,copyWithin,entries,fill,find,findIndex,findLast,findLastIndex,flat,flatMap,includes,keys,toReversed,toSorted,toSpliced,values'",
        "let d=Object.getOwnPropertyDescriptor(Array.prototype,Symbol.unscopables);!d.writable&&!d.enumerable&&d.configurable",
        "let u=Array.prototype[Symbol.unscopables],d=Object.getOwnPropertyDescriptor(u,'fill');d.writable&&d.enumerable&&d.configurable",
        "let u=Array.prototype[Symbol.unscopables];u.fill=false;delete u.copyWithin;u.fill===false&&u.copyWithin===undefined&&[1].fill(7)[0]===7",
        "delete Array.prototype[Symbol.unscopables];new Array()[Symbol.unscopables]===undefined",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    let mut host = SilentHost;
    let mut realm = crate::Realm::new(
        Limits {
            heap_entries: 180,
            ..Limits::default()
        },
        &mut host,
    )?;
    realm.evaluate("let u=Array.prototype[Symbol.unscopables];u.saved={n:7};")?;
    assert_eq!(
        realm.evaluate(
            "for(let i=0;i<300;i++){let x={}}u===Array.prototype[Symbol.unscopables]&&u.saved.n===7"
        )?,
        Value::Boolean(true)
    );
    Ok(())
}

#[test]
fn abrupt_conversion_and_mutation_side_effects_do_not_get_skipped() {
    for source in [
        "let e={},ok=false;try{Array.prototype.fill.call({get length(){throw e}},7)}catch(x){ok=x===e}ok",
        "let e={},ok=false;try{Array.prototype.copyWithin.call({length:{valueOf(){throw e}}},0,1)}catch(x){ok=x===e}ok",
        "let n=0;try{[].fill(1,0,{valueOf(){n++;throw 7}})}catch(e){}n===1",
        "let n=0;try{[].copyWithin(0,{valueOf(){n++;throw 7}})}catch(e){}n===1",
        "let n=0;let o={length:2,get 0(){throw 7},set 1(v){n++}};try{Array.prototype.copyWithin.call(o,1,0)}catch(e){}n===0",
        "let n=0;let o={length:3,set 0(v){n++;delete this[1]},get 1(){return 7},get 2(){return 8}};Array.prototype.copyWithin.call(o,0,1);n===1&&o[1]===8",
        "let o={length:4};Object.defineProperty(o,0,{set(v){this.length=0}});Array.prototype.fill.call(o,7);o.length===0&&o[1]===7&&o[2]===7&&o[3]===7",
        "let a=[1,,3];Object.preventExtensions(a);try{a.fill(7)}catch(e){}a[0]===7&&!(1 in a)&&a[2]===3",
        "let a=[1,,3];Object.preventExtensions(a);try{a.copyWithin(1,0,2)}catch(e){}!(1 in a)&&!(2 in a)",
        "[1,2].fill(7,null,null).join()==='1,2'&&[1,2].copyWithin(null,1,null).join()==='1,2'",
        "let n=0;[1,2].fill(7,0,1,{valueOf(){n++}});n===0",
        "let n=0;[1,2].copyWithin(0,1,2,{valueOf(){n++}});n===0",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}
