// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use super::{Error, Limits, Runtime, SilentHost, Value, compile, eval};

#[test]
fn copying_arrays_preserve_sources_but_materialize_holes() {
    for source in [
        "let a=[1,,3],b=a.toReversed();a!==b&&b.join()==='3,,1'&&Object.keys(b).join()==='0,1,2'&&a.join()==='1,,3'&&!(1 in a)",
        "let a=[1,,3],b=a.with(0,7);b.join()==='7,,3'&&Object.hasOwn(b,1)&&!(1 in a)&&a[0]===1",
        "let a=[1,,3],b=a.toSpliced();b!==a&&b.join()==='1,,3'&&Object.hasOwn(b,1)&&!(1 in a)",
        "let a=Object.freeze([1,2,3]);a.toReversed().join()==='3,2,1'&&a.with(-1,7).join()==='1,2,7'&&a.toSpliced(1,1,7).join()==='1,7,3'",
        "Array.prototype.toReversed.call('a😀').join('')==='\\ude00\\ud83da'&&Array.prototype.with.call('ab',0,'x').join('')==='xb'&&Array.prototype.toSpliced.call('abc',1,1).join('')==='ac'",
        "Array.prototype.toReversed.call(false).length===0&&Array.prototype.toSpliced.call(true).length===0",
        "let x={},a=[x];a.toReversed()[0]===x&&a.with(0,x)[0]===x&&a.toSpliced()[0]===x",
        "Array.prototype[1]=7;let a=[1,,3];a.toReversed()[1]===7&&a.with(0,8)[1]===7&&a.toSpliced()[1]===7",
        "let o={length:'2.9',0:1,1:2,2:3};Array.prototype.toReversed.call(o).join()==='2,1'&&Array.prototype.with.call(o,-1,7).join()==='1,7'&&Array.prototype.toSpliced.call(o).join()==='1,2'",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn copying_indices_omitted_arguments_and_error_order() {
    for source in [
        "[1,2,3].with().join()===',2,3'&&[1,2,3].with(-0.9,7).join()==='7,2,3'&&[1,2,3].with(-1.9,7).join()==='1,2,7'",
        "[1,2].with(NaN,7).join()==='7,2'&&[1,2].with(-2,7).join()==='7,2'",
        "[1,2,3].toSpliced(undefined).length===0&&[1,2,3].toSpliced().length===3&&[1,2,3].toSpliced(1,undefined).join()==='1,2,3'",
        "[1,2,3].toSpliced(1).join()==='1'&&[1,2,3].toSpliced(1,1,7,8).join()==='1,7,8,3'",
        "[1,2,3].toSpliced(-2.9,1.9,7).join()==='1,7,3'&&[1,2].toSpliced(Infinity,1,7).join()==='1,2,7'",
        "[1,2].toSpliced(-Infinity,Infinity,7).join()==='7'&&[1,2].toSpliced(0,-Infinity,7).join()==='7,1,2'",
        "let n=0;let ok=false;try{[].with({valueOf(){n++;return 0}},7)}catch(e){ok=e instanceof RangeError}ok&&n===1",
        "let n=0;try{Array.prototype.with.call(null,{valueOf(){n++}},7)}catch(e){}n===0",
        "let log='',o={get length(){log+='l';return 1},0:7};let a=Array.prototype.toSpliced.call(o,{valueOf(){log+='s';return 0}},{valueOf(){log+='c';return 0}},8);log==='lsc'&&a.join()==='8,7'",
        "let n=0,e={},ok=false;try{[].toSpliced({valueOf(){throw e}},{valueOf(){n++}})}catch(x){ok=x===e}ok&&n===0",
        "let n=0;[].toSpliced(0,{valueOf(){n++;return 9}});n===1",
        "let a=[1,2,3];a.with({valueOf(){a.length=1;return -1}},7).join()==='1,,7'&&a.length===1",
        "let a=[1,2,3];a.toSpliced({valueOf(){a.length=1;return 0}},1).join()===','&&a.length===1",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for index in ["2", "-3", "Infinity", "-Infinity"] {
        assert!(matches!(
            eval(&format!("[1,2].with({index},7)")),
            Err(Error::Range { .. })
        ));
    }
    for source in [
        "[1].with(Symbol(),7)",
        "[1].toSpliced(Symbol())",
        "[1].toSpliced(0,Symbol())",
        "Array.prototype.toReversed.call(undefined)",
        "Array.prototype.with.call(undefined,0)",
        "Array.prototype.toSpliced.call(null)",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn only_retained_element_getters_run_and_no_species_or_writes_occur() {
    for source in [
        "let log='',o={length:3,get 0(){log+='a';return 1},get 1(){log+='b';return 2},get 2(){log+='c';return 3}};Array.prototype.toReversed.call(o).join()==='3,2,1'&&log==='cba'",
        "let log='',o={length:3,get 0(){log+='a';return 1},get 1(){throw 7},get 2(){log+='c';return 3}};Array.prototype.with.call(o,1,8).join()==='1,8,3'&&log==='ac'",
        "let log='',o={length:4,get 0(){log+='a';return 1},get 1(){throw 7},get 2(){throw 8},get 3(){log+='d';return 4}};Array.prototype.toSpliced.call(o,1,2,9).join()==='1,9,4'&&log==='ad'",
        "let o={length:2,get 0(){throw 7},get 1(){throw 8}};Array.prototype.toSpliced.call(o,0).length===0",
        "let o={length:3,get 2(){delete this[0];return 3},0:1,1:2};Array.prototype.toReversed.call(o).join()==='3,2,'",
        "let a=[1,2];Object.defineProperty(a,1,{get(){a.length=3;a[0]=7;return 2}});a.toReversed().join()==='2,7'",
        "let e={},n=0,o={length:2,get 1(){throw e},get 0(){n++}},ok=false;try{Array.prototype.toReversed.call(o)}catch(x){ok=x===e}ok&&n===0",
        "let e={},n=0,o={length:3,get 0(){throw e},get 2(){n++}},ok=false;try{Array.prototype.with.call(o,1,7)}catch(x){ok=x===e}ok&&n===0",
        "let e={},n=0,o={length:3,get 0(){throw e},get 2(){n++}},ok=false;try{Array.prototype.toSpliced.call(o,1,1)}catch(x){ok=x===e}ok&&n===0",
        "class A extends Array{}let a=new A(1,2,3);Object.defineProperty(a,'constructor',{get(){throw 7}});[a.toReversed(),a.with(0,7),a.toSpliced()].every(b=>!(b instanceof A)&&Object.getPrototypeOf(b)===Array.prototype)",
        "Object.defineProperty(Array,Symbol.species,{get(){throw 7}});[1].toReversed()[0]===1&&[1].with(0,2)[0]===2&&[1].toSpliced()[0]===1",
        "Object.defineProperty(Array.prototype,0,{set(v){throw 7}});let a=Array.prototype.toReversed.call({length:1,0:7});Object.getOwnPropertyDescriptor(a,0).value===7",
        "Object.defineProperty(Array.prototype,0,{set(v){throw 7}});let a=Array.prototype.with.call({length:1,get 0(){throw 8}},0,7);Object.getOwnPropertyDescriptor(a,0).value===7",
        "Object.defineProperty(Array.prototype,0,{set(v){throw 7}});let a=Array.prototype.toSpliced.call({length:0},0,0,7);Object.getOwnPropertyDescriptor(a,0).value===7",
        "let o={get length(){return 2},set length(v){throw 7},0:1,1:2};Array.prototype.toReversed.call(o).join()==='2,1'&&Array.prototype.with.call(o,0,7).join()==='7,2'&&Array.prototype.toSpliced.call(o).join()==='1,2'",
        "let v={toString(){throw 7},valueOf(){throw 8}};[1].with(0,v)[0]===v&&[].toSpliced(0,0,v)[0]===v",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn copying_wide_bounds_can_skip_huge_deleted_ranges() {
    for source in [
        "let n=Number.MAX_SAFE_INTEGER,o={length:Infinity};o[n-2]=7;o[n-1]=8;Array.prototype.toSpliced.call(o,0,n-2).join()==='7,8'",
        "let o={length:4294967298,4294967296:7,4294967297:8};Array.prototype.toSpliced.call(o,0,4294967296,9).join()==='9,7,8'",
        "let o={length:Infinity,get 0(){throw 7}};Array.prototype.toSpliced.call(o,0,Infinity,9).join()==='9'",
        "let n=0,o={length:Infinity,get 0(){n++;return 7}},ok=false;try{Array.prototype.toSpliced.call(o,0,0,1)}catch(e){ok=e instanceof TypeError}ok&&n===0",
        "let n=0,o={length:4294967296,get 0(){n++;return 7}},ok=false;try{Array.prototype.toReversed.call(o)}catch(e){ok=e instanceof RangeError}ok&&n===0",
        "let n=0,o={length:Infinity,get 0(){n++;return 7}},ok=false;try{Array.prototype.with.call(o,-1,8)}catch(e){ok=e instanceof RangeError}ok&&n===0",
        "let n=0,o={length:4294967296,get 0(){n++;return 7}},ok=false;try{Array.prototype.toSpliced.call(o)}catch(e){ok=e instanceof RangeError}ok&&n===0",
        "let e={},o={length:Infinity},ok=false;try{Array.prototype.with.call(o,{valueOf(){throw e}},7)}catch(x){ok=x===e}ok",
        "let e={},o={length:Infinity},ok=false;try{Array.prototype.toSpliced.call(o,0,{valueOf(){throw e}})}catch(x){ok=x===e}ok",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn copying_array_gc_and_quota_failures() -> Result<(), Error> {
    let limits = Limits {
        heap_entries: 180,
        ..Limits::default()
    };
    for source in [
        "let a=[1,{n:7}];Object.defineProperty(a,0,{get(){a.length=0;for(let i=0;i<300;i++){let g={}}return 8}});let b=a.toReversed();b[0].n===7&&b[1]===8",
        "let o={length:2,get 1(){for(let i=0;i<300;i++){let g={}}return 8}};let b=Array.prototype.with.call(o,0,{n:7});b[0].n===7&&b[1]===8",
        "let o={length:1,get 0(){for(let i=0;i<300;i++){let g={}}return 8}};let b=Array.prototype.toSpliced.call(o,0,0,{n:7});b[0].n===7&&b[1]===8",
        "let o={length:2,get 0(){return {n:7}},get 1(){for(let i=0;i<300;i++){let g={}}return 8}};Array.prototype.toSpliced.call(o)[0].n===7",
    ] {
        assert_eq!(
            Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
            Value::Boolean(true),
            "{source}"
        );
    }
    for source in [
        "Array(4294967295).toReversed()",
        "Array(4294967295).with(-1,7)",
        "Array(4294967295).toSpliced()",
    ] {
        let limits = Limits {
            fuel: 500,
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
fn copying_array_metadata_is_mutable_and_nonconstructible() {
    for (name, length) in [("toReversed", 0), ("with", 2), ("toSpliced", 2)] {
        let source = format!(
            "let f=Array.prototype.{name},d=Object.getOwnPropertyDescriptor(Array.prototype,'{name}'),l=Object.getOwnPropertyDescriptor(f,'length'),n=Object.getOwnPropertyDescriptor(f,'name');f.name==='{name}'&&f.length==={length}&&f.prototype===undefined&&d.writable&&!d.enumerable&&d.configurable&&!l.writable&&!l.enumerable&&l.configurable&&!n.writable&&!n.enumerable&&n.configurable"
        );
        assert_eq!(eval(&source), Ok(Value::Boolean(true)), "{source}");
        let source = format!(
            "let f=Array.prototype.{name};f.extra=7;delete f.name;delete Array.prototype.{name};f.extra===7&&!Object.hasOwn(f,'name')&&!Object.hasOwn(Array.prototype,'{name}')"
        );
        assert_eq!(eval(&source), Ok(Value::Boolean(true)), "{source}");
        assert!(matches!(
            eval(&format!("new Array.prototype.{name}()")),
            Err(Error::Type { .. })
        ));
    }
    assert_eq!(
        eval(
            "let u=Array.prototype[Symbol.unscopables];u.toReversed&&u.toSpliced&&!Object.hasOwn(u,'with')"
        ),
        Ok(Value::Boolean(true))
    );
}
