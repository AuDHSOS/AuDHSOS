// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use super::{Error, Limits, Runtime, SilentHost, Value, compile, eval};

#[test]
fn integer_radix_and_decimal_prefix_parsing() {
    for source in [
        "0x200000000000011===144115188075855900&&Number('0x1000000000000081')===1152921504606847200",
        "parseInt('0x200000000000011')===144115188075855900&&parseInt('1000000000000081',16)===1152921504606847200",
        "parseInt('  -0xFz')===-15&&parseInt('0xF',16)===15&&parseInt('0xF',10)===0",
        "parseInt('011')===11&&parseInt('0b11')===0&&parseInt('0o11')===0",
        "parseInt('11',2)===3&&parseInt('z',36)===35&&parseInt('Z',36)===35",
        "parseInt('12.9')===12&&parseInt('1e3')===1&&parseInt('12garbage')===12",
        "parseInt('10',4294967298)===2&&parseInt('10',-4294967294)===2&&parseInt('10',2.9)===2",
        "isNaN(parseInt('1',1))&&isNaN(parseInt('1',37))&&isNaN(parseInt('1',-2))",
        "isNaN(parseInt())&&isNaN(parseInt(''))&&isNaN(parseInt('0x'))&&isNaN(parseInt('+'))&&isNaN(parseInt('2',2))",
        "Object.is(parseInt('-0'),-0)&&Object.is(parseInt('-0x0'),-0)&&Object.is(parseInt('-000000'),-0)",
        "parseInt('\\u2028\\ufeff\\u00a0  +42x')===42&&isNaN(parseInt('\\u008542'))&&isNaN(parseInt('\\u180e42'))",
        "parseFloat('  -12.5e2tail')===-1250&&parseFloat('.25')===0.25&&parseFloat('1.')===1",
        "parseFloat('1e+')===1&&parseFloat('1e-')===1&&parseFloat('1e')===1&&parseFloat('1.e2')===100",
        "parseFloat('Infinityx')===Infinity&&parseFloat('-Infinityx')===-Infinity&&parseFloat('+Infinity')===Infinity",
        "isNaN(parseFloat())&&isNaN(parseFloat(''))&&isNaN(parseFloat('.'))&&isNaN(parseFloat('inf'))&&isNaN(parseFloat('infinity'))",
        "parseFloat('0x11')===0&&parseFloat('1_2')===1&&parseFloat('1.2.3')===1.2&&parseFloat('1e-2x')===0.01",
        "Object.is(parseFloat('-0'),-0)&&Object.is(parseFloat('-1e-9999'),-0)&&parseFloat('1e9999')===Infinity",
        "parseFloat('\\ufeff\\u2029.5x')===0.5&&isNaN(parseFloat('\\u00851'))&&parseFloat('12\\ud800')===12",
        "parseInt('12\\ud800')===12&&isNaN(parseInt('\\ud80012'))",
        "Number.parseInt===parseInt&&Number.parseFloat===parseFloat&&parseInt.length===2&&parseFloat.length===1",
        "let log='';parseInt({toString(){log+='s';return '7'}},{valueOf(){log+='r';return 10}});log==='sr'",
        "let n=0;parseFloat({toString(){n++;return '7'}},{valueOf(){throw 1}});n===1",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn numeric_parser_errors_metadata_and_constructor_storage() {
    for source in [
        "let n=0,e={},ok=false;try{parseInt({toString(){throw e}},{valueOf(){n++}})}catch(x){ok=x===e}ok&&n===0",
        "let log='',e={},ok=false;try{parseInt({toString(){log+='s';return ''}},{valueOf(){log+='r';throw e}})}catch(x){ok=x===e}ok&&log==='sr'",
        "let e={},ok=false;try{parseFloat({toString(){throw e}})}catch(x){ok=x===e}ok",
        "let d=Object.getOwnPropertyDescriptor(globalThis,'parseInt');d.writable&&!d.enumerable&&d.configurable",
        "let d=Object.getOwnPropertyDescriptor(Number,'parseFloat');d.writable&&!d.enumerable&&d.configurable",
        "let f=parseInt;let d=Object.getOwnPropertyDescriptor(f,'length');!d.writable&&!d.enumerable&&d.configurable&&f.name==='parseInt'&&f.prototype===undefined",
        "let f=parseFloat;let d=Object.getOwnPropertyDescriptor(f,'name');!d.writable&&!d.enumerable&&d.configurable&&f.name==='parseFloat'&&f.prototype===undefined",
        "let f=parseInt;f.extra=7;delete f.name;f.extra===7&&!Object.hasOwn(f,'name')&&Number.parseInt===f",
        "let f=Number.parseInt;Number.parseInt=7;Number.parseInt===7&&parseInt===f&&f('11')===11",
        "let f=parseFloat;delete globalThis.parseFloat;typeof parseFloat==='undefined'&&Number.parseFloat===f",
        "let f=parseInt;delete Number.parseInt;!Object.hasOwn(Number,'parseInt')&&parseInt===f",
        "let n=0;Object.defineProperty(Number,'parseFloat',{get(){n++;return 7}});Number.parseFloat===7&&n===1",
        "let s=Symbol();Number[s]={n:7};Object.getOwnPropertySymbols(Number)[0]===s&&Number[s].n===7&&delete Number[s]",
        "let p={x:7};Object.setPrototypeOf(Number,p);Object.getPrototypeOf(Number)===p&&Number.x===7&&Number('42')===42",
        "Object.freeze(Number);let old=Number.parseFloat;Number.parseFloat=7;Number.parseFloat===old&&Object.isFrozen(Number)",
        "Object.seal(Number);Object.isSealed(Number)&&!Object.isFrozen(Number)",
        "Object.preventExtensions(Number);Number.x=7;!Object.isExtensible(Number)&&Number.x===undefined",
        "let keys=Object.getOwnPropertyNames(Number);keys.includes('MAX_VALUE')&&keys.includes('parseInt')&&keys.includes('prototype')&&Object.keys(Number).length===0",
        "Object.getPrototypeOf(Number)===Function.prototype&&Number.prototype.constructor===Number&&new Number(7).valueOf()===7",
        "let s=Symbol.toPrimitive,log='';let o={[s](hint){log+=hint;return '42'}};parseInt(o)===42&&parseFloat(o)===42&&log==='stringstring'",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "parseInt(Symbol())",
        "parseFloat(Symbol())",
        "parseInt('1',Symbol())",
        "parseInt({toString(){return {}},valueOf(){return {}}})",
        "new parseInt()",
        "new parseFloat()",
        "Object.defineProperty(Number,'MAX_VALUE',{value:7})",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn numeric_parser_gc_resource_limits_and_realm_identity() -> Result<(), Error> {
    let limits = Limits {
        heap_entries: 180,
        ..Limits::default()
    };
    for source in [
        "parseInt({toString(){for(let i=0;i<300;i++){let g={}}return '123'}},{valueOf(){for(let i=0;i<300;i++){let g={}}return 10}})===123",
        "parseFloat({toString(){for(let i=0;i<300;i++){let g={}}return '1.5'}})===1.5",
        "let s=Symbol();Number[s]={n:7};Number.parseInt.saved={n:8};for(let i=0;i<300;i++){let g={}}Number[s].n===7&&parseInt.saved.n===8",
    ] {
        assert_eq!(
            Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
            Value::Boolean(true),
            "{source}"
        );
    }
    let mut host = SilentHost;
    let mut realm = crate::Realm::new(limits, &mut host)?;
    realm.evaluate("let saved=parseInt;delete globalThis.parseInt;delete Number.parseFloat;")?;
    realm.evaluate("for(let i=0;i<300;i++){let garbage={}}")?;
    assert_eq!(realm.evaluate("typeof parseInt==='undefined'&&Number.parseInt===saved&&!Object.hasOwn(Number,'parseFloat')")?,Value::Boolean(true));
    for source in [
        format!("parseInt('{}')", "9".repeat(5000)),
        format!("parseFloat('{}')", "9".repeat(5000)),
    ] {
        let limits = Limits {
            fuel: 1000,
            ..Limits::default()
        };
        assert!(matches!(
            Runtime::new(limits).run(&compile(&source, limits)?, &mut SilentHost),
            Err(Error::Limit {
                resource: "execution fuel"
            })
        ));
    }
    Ok(())
}
