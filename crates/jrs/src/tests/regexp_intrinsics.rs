// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use super::{Error, Limits, Runtime, SilentHost, Value, compile, eval};

#[test]
fn regexp_prototype_identity_constructor_and_own_properties() {
    for source in [
        "let r=/a/g;r instanceof RegExp&&Object.getPrototypeOf(r)===RegExp.prototype&&r.constructor===RegExp",
        "let r=new RegExp();r instanceof RegExp&&Object.getPrototypeOf(r)===RegExp.prototype&&r.source==='(?:)'",
        "Object.getPrototypeOf(RegExp)===Function.prototype&&Object.getPrototypeOf(RegExp.prototype)===Object.prototype",
        "Object.getOwnPropertyNames(/a/).join()==='lastIndex'&&Object.getOwnPropertyNames(RegExp).slice(0,3).join()==='length,name,prototype'",
        "RegExp.prototype.source==='(?:)'&&RegExp.prototype.flags===''&&RegExp.prototype.global===undefined&&!Object.hasOwn(RegExp.prototype,'lastIndex')",
        "RegExp.prototype.toString()==='/(?:)/'&&Object.prototype.toString.call(RegExp.prototype)==='[object Object]'",
        "Object.prototype.toString.call(/a/)==='[object RegExp]'&&Object.prototype.toString.call(Object.create(RegExp.prototype))==='[object Object]'",
        "let r=/a/;r[Symbol.toStringTag]='Custom';Object.prototype.toString.call(r)==='[object Custom]'",
        "let d=Object.getOwnPropertyDescriptor(RegExp,'prototype');!d.writable&&!d.enumerable&&!d.configurable&&RegExp.name==='RegExp'&&RegExp.length===2",
        "let d=Object.getOwnPropertyDescriptor(/a/,'lastIndex');d.writable&&!d.enumerable&&!d.configurable&&d.value===0",
        "let r=/a/g;r.lastIndex=7;RegExp(r)===r&&RegExp(r,undefined)===r&&r.lastIndex===7&&new RegExp(r)!==r&&new RegExp(r).lastIndex===0",
        "let r=/a/g;r.constructor=function(){};let x=RegExp(r);x!==r&&x.source==='a'&&x.flags==='g'",
        "let r=/a/g;r[Symbol.match]=false;let x=RegExp(r);x!==r&&x.source==='a'&&x.global",
        "let r=/a/g;Object.defineProperty(r,'source',{get(){throw 7}});Object.defineProperty(r,'flags',{get(){throw 8}});let x=new RegExp(r);x.source==='a'&&x.flags==='g'",
        "let r=/a/g;let x=new RegExp(r,'m');x.source==='a'&&x.flags==='m'&&!x.global",
        "let o={[Symbol.match]:true,constructor:RegExp,get source(){throw 7},get flags(){throw 8}};RegExp(o)===o",
        "let o={[Symbol.match]:true,source:'a',flags:'myg'};let r=RegExp(o);r.source==='a'&&r.flags==='gmy'&&new RegExp(r).flags==='gmy'",
        "class R extends RegExp{}let r=new R('a','g');r instanceof R&&r instanceof RegExp&&r.constructor===R&&r.test('a')&&RegExp(r)!==r",
        "function R(){}let p={};R.prototype=p;let r=Reflect.construct(RegExp,['a'],R);Object.getPrototypeOf(r)===p&&RegExp.prototype.test.call(r,'a')",
        "function R(){}R.prototype=null;Object.getPrototypeOf(Reflect.construct(RegExp,[],R))===RegExp.prototype",
        "let Bound=RegExp.bind(null,'a');new Bound() instanceof RegExp&&new Bound().test('a')",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    assert_eq!(
        eval("class R extends RegExp{}R[Symbol.species]===R&&RegExp[Symbol.species]===RegExp"),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn regexp_accessors_are_branded_except_generic_flags() {
    for (name, flag) in [
        ("hasIndices", "d"),
        ("global", "g"),
        ("ignoreCase", "i"),
        ("multiline", "m"),
        ("dotAll", "s"),
        ("unicode", "u"),
        ("unicodeSets", "v"),
        ("sticky", "y"),
    ] {
        let source = format!(
            "let d=Object.getOwnPropertyDescriptor(RegExp.prototype,'{name}');d.get.name==='get {name}'&&d.get.length===0&&d.set===undefined&&!d.enumerable&&d.configurable&&d.get.call(RegExp.prototype)===undefined&&d.get.call(/a/)===false"
        );
        assert_eq!(eval(&source), Ok(Value::Boolean(true)), "{source}");
        for receiver in [
            "{}",
            "Object.create(RegExp.prototype)",
            "null",
            "1",
            "'a'",
            "undefined",
        ] {
            let source = format!(
                "Object.getOwnPropertyDescriptor(RegExp.prototype,'{name}').get.call({receiver})"
            );
            assert!(matches!(eval(&source), Err(Error::Type { .. })), "{source}");
        }
        if !matches!(flag, "i" | "u" | "v") {
            assert_eq!(
                eval(&format!("new RegExp('a','{flag}').{name}")),
                Ok(Value::Boolean(true))
            );
        }
    }
    for source in [
        "let d=Object.getOwnPropertyDescriptor(RegExp.prototype,'source');d.get.name==='get source'&&d.get.length===0&&d.set===undefined&&!d.enumerable&&d.configurable&&d.get.call(RegExp.prototype)==='(?:)'",
        "let f=Object.getOwnPropertyDescriptor(RegExp.prototype,'flags').get;f.call({hasIndices:1,global:{},ignoreCase:[],multiline:true,dotAll:1,unicode:1,unicodeSets:1,sticky:1})==='dgimsuvy'",
        "let log='',o={get hasIndices(){log+='d';return false},get global(){log+='g';return true},get ignoreCase(){log+='i'},get multiline(){log+='m';return true},get dotAll(){log+='s'},get unicode(){log+='u'},get unicodeSets(){log+='v'},get sticky(){log+='y';return true}};Object.getOwnPropertyDescriptor(RegExp.prototype,'flags').get.call(o)==='gmy'&&log==='dgimsuvy'",
        "let r=/a/g;Object.defineProperty(r,'global',{value:false});r.flags===''&&Object.getOwnPropertyDescriptor(RegExp.prototype,'global').get.call(r)===true",
        "let e={},ok=false;try{Object.getOwnPropertyDescriptor(RegExp.prototype,'flags').get.call({get global(){throw e}})}catch(x){ok=x===e}ok",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "Object.getOwnPropertyDescriptor(RegExp.prototype,'source').get.call({})",
        "Object.getOwnPropertyDescriptor(RegExp.prototype,'flags').get.call('')",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn regexp_constructor_order_and_abrupt_completion() {
    for source in [
        "let log='',o={get [Symbol.match](){log+='m';return true},get constructor(){log+='c'},get source(){log+='s';return {toString(){log+='p';return 'a'}}},get flags(){log+='f';return {toString(){log+='g';return ''}}}};RegExp(o);log==='mcsfpg'",
        "let log='',o={get [Symbol.match](){log+='m';return true},get constructor(){throw 7},get source(){log+='s';return 'a'},get flags(){log+='f';return ''}};new RegExp(o);log==='msf'",
        "let log='',o={get [Symbol.match](){log+='m';return true},get constructor(){throw 7},get source(){log+='s';return 'a'},get flags(){throw 7}};RegExp(o,'m');log==='ms'",
        "let log='',o={get [Symbol.match](){log+='m';return false},toString(){log+='s';return 'a'}};class C extends Function{}function R(){}let bound=R.bind(null);Object.defineProperty(bound,'prototype',{get(){log+='p';return {}}});Reflect.construct(RegExp,[o,{toString(){log+='f';return ''}}],bound);log==='mpsf'",
        "let n=0,e={},ok=false;try{RegExp({get [Symbol.match](){throw e},toString(){n++}},{toString(){n++}})}catch(x){ok=x===e}ok&&n===0",
        "let e={},ok=false;let r=/a/;Object.defineProperty(r,'constructor',{get(){throw e}});try{RegExp(r)}catch(x){ok=x===e}ok&&new RegExp(r).source==='a'",
        "let n=0,e={},ok=false;try{RegExp({toString(){throw e}},{toString(){n++}})}catch(x){ok=x===e}ok&&n===0",
        "let e={},ok=false;try{RegExp('a',{toString(){throw e}})}catch(x){ok=x===e}ok",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in ["RegExp(Symbol())", "RegExp('a',Symbol())"] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
    for flags in ["ii", "uv", "iiq", "q", "gg"] {
        assert!(
            matches!(
                eval(&format!("RegExp('a','{flags}')")),
                Err(Error::Syntax { .. })
            ),
            "{flags}"
        );
    }
}

#[test]
fn regexp_test_exec_override_and_generic_tostring_observe_hooks() {
    for source in [
        "let r=/a/;r.exec=function(s){return {}};r.test('x')",
        "let r=/a/;r.exec=()=>null;!r.test('a')",
        "let log='',o={get exec(){log+='e';return function(s){log+=s;return {}}}};RegExp.prototype.test.call(o,{toString(){log+='s';return 'x'}})&&log==='sex'",
        "let o={exec(s){return this===o&&s==='7'?function(){}:null}};RegExp.prototype.test.call(o,7)",
        "let r=/a/;r.exec=7;RegExp.prototype.test.call(r,'a')",
        "let r=/a/;r.exec=null;!RegExp.prototype.test.call(r,'b')",
        "let r=/a/,n=0;r.lastIndex={valueOf(){n++;return 99}};r.exec('a')[0]==='a'&&n===1",
        "let r=/a/,n=0;r.lastIndex={valueOf(){n++;return -Infinity}};r.test('a')&&n===1",
        "let r=Object.freeze(/a/);r.exec('a')[0]==='a'&&r.lastIndex===0",
        "let r=/a/g;r.lastIndex=1.9;r.exec('ba').index===1&&r.lastIndex===2",
        "let r=/(?:)/g;r.lastIndex=1.9;r.exec('a').index===1&&r.lastIndex===1",
        "let r=/(?:)/y;r.lastIndex=0.9;r.exec('').index===0&&r.lastIndex===0",
        "let r=/a/g;Object.defineProperty(r,'global',{value:false});r.test('a')&&r.lastIndex===1",
        "let log='',o={get source(){log+='s';return {toString(){log+='p';return 'a'}}},get flags(){log+='f';return {toString(){log+='g';return 'm'}}}};RegExp.prototype.toString.call(o)==='/a/m'&&log==='spfg'",
        "RegExp.prototype.toString.call({})==='/undefined/undefined'",
        "let n=0;try{RegExp.prototype.exec.call({}, {toString(){n++;return ''}})}catch(e){}n===0",
        "let n=0;try{RegExp.prototype.test.call({}, {toString(){n++;return ''}})}catch(e){}n===1",
        "let e={},ok=false,r=/a/;r.lastIndex={valueOf(){throw e}};try{r.exec('a')}catch(x){ok=x===e}ok",
        "let e={},n=0,ok=false;try{RegExp.prototype.toString.call({source:{toString(){throw e}},get flags(){n++}})}catch(x){ok=x===e}ok&&n===0",
        "let e={},ok=false;try{RegExp.prototype.test.call({get exec(){throw e}},'a')}catch(x){ok=x===e}ok",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "RegExp.prototype.exec('')",
        "RegExp.prototype.test('')",
        "RegExp.prototype.toString.call(null)",
        "let r=/a/;r.exec=()=>7;r.test('a')",
        "let r=/a/;r.exec=()=>undefined;r.test('a')",
        "let r=/a/;r.lastIndex=Symbol();r.test('a')",
        "Object.freeze(/a/g).test('a')",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn regexp_metadata_and_gc_roots_are_stable() -> Result<(), Error> {
    for source in [
        "let s=Symbol();RegExp[s]={n:7};let f=RegExp.prototype.test;f.extra=7;delete f.name;RegExp[s].n===7&&f.extra===7&&!Object.hasOwn(f,'name')",
        "let old=RegExp.prototype.test;delete RegExp.prototype.test;old.call(/a/,'a')&&!Object.hasOwn(/a/,'test')",
        "let d=Object.getOwnPropertyDescriptor(RegExp,Symbol.species);d.get.name==='get [Symbol.species]'&&d.get.length===0&&!d.enumerable&&d.configurable&&d.set===undefined&&d.get.call(7)===7",
        "Object.freeze(RegExp);Object.isFrozen(RegExp)&&new RegExp('a').test('a')",
        "let p={x:7};Object.setPrototypeOf(RegExp,p);RegExp.x===7&&RegExp('a').test('a')",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    let limits = Limits {
        heap_entries: 180,
        ..Limits::default()
    };
    for source in [
        "let n=0;for(let i=0;i<300;i++){let r=/a/;if(r.test('a'))n++}n===300",
        "let o={[Symbol.match]:true,get source(){return {toString(){return 'a'}}},get flags(){for(let i=0;i<300;i++){let g={}}return 'g'}};RegExp(o).test('a')",
        "let r=/a/;r.exec=function(s){for(let i=0;i<300;i++){let g={}}return {}};r.test('x')",
        "let f=Object.getOwnPropertyDescriptor(RegExp.prototype,'source').get;for(let i=0;i<300;i++){let g={}}f.call(/a/)==='a'",
        "function R(){}let b=R.bind(null);Object.defineProperty(b,'prototype',{get(){for(let i=0;i<300;i++){let g={}}return {n:7}}});let r=Reflect.construct(RegExp,['a'],b);Object.getPrototypeOf(r).n===7&&RegExp.prototype.test.call(r,'a')",
    ] {
        assert_eq!(
            Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
            Value::Boolean(true),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn regexp_sources_roundtrip_line_terminators_slashes_and_backslashes() {
    for pattern in [
        "",
        "a/b",
        "[/]",
        "\\/",
        "\\\\/",
        "\n",
        "\r",
        "\u{2028}",
        "\u{2029}",
        "\\\n",
        "[\n/]",
        "a\n\r\u{2028}b",
    ] {
        let mut units = String::new();
        for unit in pattern.encode_utf16() {
            use std::fmt::Write;
            write!(units, "\\u{unit:04x}").unwrap();
        }
        let source = format!(
            "let r=RegExp('{units}');let s=r.source;let literal=Function('return /'+s+'/')();literal.source===s&&literal.test('{units}')===r.test('{units}')"
        );
        assert_eq!(eval(&source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn regexp_getter_output_limits_and_fatal_errors() -> Result<(), Error> {
    let limits = Limits {
        string_units: 16,
        ..Limits::default()
    };
    assert_eq!(
        Runtime::new(limits).run(
            &compile("/aaaaaaaaaaaaaa/.toString()", limits)?,
            &mut SilentHost
        )?,
        Value::string("/aaaaaaaaaaaaaa/")
    );
    for source in [
        "/aaaaaaaaaaaaaaa/.toString()",
        "RegExp('////////////////').source",
        "RegExp('\u{2028}\u{2028}\u{2028}').source",
    ] {
        assert!(
            matches!(
                Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost),
                Err(Error::Limit {
                    resource: "string units"
                })
            ),
            "{source}"
        );
    }
    let limits = Limits {
        fuel: 1000,
        ..Limits::default()
    };
    let source = "let r=/a/;r.exec=()=>r.test('a');try{r.test('a')}catch(e){throw 'caught'}";
    assert!(matches!(
        Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost),
        Err(Error::Limit { .. })
    ));
    Ok(())
}
