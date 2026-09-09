// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use super::{Error, Limits, Runtime, SilentHost, Value, compile, eval};

#[test]
fn functions_and_arrows_preserve_exact_grammar_source() {
    for text in [
        "function f ( /* args */ a ) { /* body */ return a; }",
        "function /*a*/ (x=7,...rest) {return rest}",
        "( /*a*/ x /*b*/ ) /*c*/ => /*d*/ x+1",
        "x /*a*/ => /*b*/ ({x})",
        "() => {return 7; /*tail*/}",
        "async /*a*/ function f /*b*/ (a){return await a}",
        "async /*a*/ x /*b*/ => /*c*/ x",
        "async /*a*/ (x=7) => {return x}",
        "function f(){return '😀'; /*é*/}",
        "function f()\r\n{\r return 1;\n}\u{2028}",
        "function f(){return `x${1}y`}",
        "function f(){return /x/}",
    ] {
        let expected = text.trim_end_matches(['\u{2028}']);
        let source = format!("let f=/*outside*/({text})/*after*/;f.toString()");
        assert_eq!(eval(&source), Ok(Value::string(expected)), "{text}");
    }
    for source in [
        "function /*a*/ f(x){return x} f.toString()==='function /*a*/ f(x){return x}'",
        "async /*a*/ function f(x){return x} f.toString()==='async /*a*/ function f(x){return x}'",
        "let f=()=>1 /* after */;f.toString()==='()=>1'",
        "let f=()=>()=>1;f.toString()==='()=>()=>1'&&f().toString()==='()=>1'",
        "let f=function outer(){return function inner(){return 7}};f().toString()==='function inner(){return 7}'",
        "let f=(x=()=>1)=>x;f().toString()==='()=>1'",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn method_getter_setter_and_class_sources_use_their_own_ranges() {
    for (source, expected) in [
        (
            "let o={/* before */ m /*a*/ (x) {return x} /* after */};o.m.toString()",
            "m /*a*/ (x) {return x}",
        ),
        (
            "let key='m';let o={ [ /*a*/ key /*b*/ ] (){return 7} };o.m.toString()",
            "[ /*a*/ key /*b*/ ] (){return 7}",
        ),
        (
            "let o={get /*a*/ x(){return 7}};Object.getOwnPropertyDescriptor(o,'x').get.toString()",
            "get /*a*/ x(){return 7}",
        ),
        (
            "let o={set /*a*/ x(v){}};Object.getOwnPropertyDescriptor(o,'x').set.toString()",
            "set /*a*/ x(v){}",
        ),
        (
            "let o={async /*a*/ m(x){return x}};o.m.toString()",
            "async /*a*/ m(x){return x}",
        ),
        (
            "class /*a*/ A { /*b*/ m(){} } /*after*/ A.toString()",
            "class /*a*/ A { /*b*/ m(){} }",
        ),
        (
            "class A {constructor(){this.x=7} m(){}}A.toString()",
            "class A {constructor(){this.x=7} m(){}}",
        ),
        (
            "let A=class extends Object{constructor(){super()}};A.toString()",
            "class extends Object{constructor(){super()}}",
        ),
        (
            "class A{static /*exclude*/ async /*keep*/ m(){}}A.m.toString()",
            "async /*keep*/ m(){}",
        ),
        (
            "class A{static /*exclude*/ get /*keep*/ x(){return 7}}Object.getOwnPropertyDescriptor(A,'x').get.toString()",
            "get /*keep*/ x(){return 7}",
        ),
        (
            "class A{get x(){return 7}}Object.getOwnPropertyDescriptor(A.prototype,'x').get.toString()",
            "get x(){return 7}",
        ),
        ("let o={ 'x y' (){} };o['x y'].toString()", "'x y' (){}"),
    ] {
        assert_eq!(eval(source), Ok(Value::string(expected)), "{source}");
    }
}

#[test]
fn source_survives_gc_and_separate_scripts_and_names_do_not_rewrite_it() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut realm = crate::Realm::new(
        Limits {
            heap_entries: 150,
            ..Limits::default()
        },
        &mut host,
    )?;
    realm.evaluate("let f=function original (x) {return x};let method=({m(){}}).m")?;
    realm.evaluate("for(let i=0;i<300;i++){let x={}}delete f.name;Object.defineProperty(f,'name',{value:'renamed'})")?;
    assert_eq!(
        realm.evaluate("f.toString()")?,
        Value::string("function original (x) {return x}")
    );
    assert_eq!(realm.evaluate("method.toString()")?, Value::string("m(){}"));
    for source in [
        "Function('return function inner(){return 7}')().toString()==='function inner(){return 7}'",
        "Function('f=()=>7','return f')().toString()==='()=>7'",
        "Function('return class A{m(){}}')().toString()==='class A{m(){}}'",
        "Function('return class A{m(){}}')().prototype.m.toString()==='m(){}'",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    Ok(())
}

#[test]
fn source_sharing_and_to_string_limits_are_explicit() -> Result<(), Error> {
    let program = compile("let a=()=>1;let b=()=>2;", Limits::default())?;
    let a = program.functions[0].source.as_ref().unwrap();
    let b = program.functions[1].source.as_ref().unwrap();
    assert!(alloc::rc::Rc::ptr_eq(&a.text, &b.text));
    let source = "function f(){/* this comment is longer than the output quota */}f.toString()";
    let program = compile(source, Limits::default())?;
    assert!(matches!(
        Runtime::new(Limits {
            string_units: 32,
            ..Limits::default()
        })
        .run(&program, &mut SilentHost),
        Err(Error::Limit { .. })
    ));
    let source = format!("let f=()=>1{};f.toString()", "/*outside*/".repeat(100));
    assert_eq!(
        Runtime::new(Limits {
            string_units: 32,
            ..Limits::default()
        })
        .run(&compile(&source, Limits::default())?, &mut SilentHost)?,
        Value::string("()=>1")
    );
    assert!(matches!(
        eval("Function.prototype.toString.call({})"),
        Err(Error::Type { .. })
    ));
    Ok(())
}

#[test]
fn native_functions_and_modified_properties_keep_native_source() {
    for source in [
        "let f=Function.prototype.toString;f.name==='toString'&&f.length===0&&Object.isExtensible(f)&&f.prototype===undefined",
        "Object.getOwnPropertyNames(Function.prototype.toString).join()==='length,name'",
        "Function.prototype.toString.toString()==='function toString() { [native code] }'",
        "let f=Math.pow;Object.defineProperty(f,'name',{value:'changed'});f.toString()==='function pow() { [native code] }'",
        "let f=()=>7;let b=f.bind(null);b.toString()==='function () { [native code] }'",
        "Function.prototype.toString()==='function () { [native code] }'",
        "let original=Function.prototype.toString;Function.prototype.toString=function(){return 'changed'};Number.toString()==='changed'&&original.call(Number)==='function () { [native code] }'",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn source_text_covers_token_tails_templates_and_persisting_closures() -> Result<(), Error> {
    for (source, expected) in [
        ("let f=x=>x++;f.toString()", "x=>x++"),
        (
            "let f=()=>`x${(()=>7)()}y`;f.toString()",
            "()=>`x${(()=>7)()}y`",
        ),
        ("let f=()=> /[}]/;f.toString()", "()=> /[}]/"),
        (
            "let f=()=> /*internal*/ (/*inside*/ 1); /*outside*/ f.toString()",
            "()=> /*internal*/ (/*inside*/ 1)",
        ),
        (
            "let A=class /*name*/ {\nstatic\n m(){return 1}\n};A.m.toString()",
            "m(){return 1}",
        ),
        ("let o={async(){}};o.async.toString()", "async(){}"),
        (
            "class A{static(){}}A.prototype.static.toString()",
            "static(){}",
        ),
        ("let f=async()=>await 1;f.toString()", "async()=>await 1"),
    ] {
        assert_eq!(eval(source), Ok(Value::string(expected)), "{source}");
    }
    let mut host = SilentHost;
    let mut realm = crate::Realm::new(Limits::default(), &mut host)?;
    let eval = realm.eval_script_function()?;
    realm.set_global("evalScript", &eval)?;
    realm.evaluate("let f=evalScript('(()=> /*keep*/ 7)');")?;
    assert_eq!(
        realm.evaluate("f.toString()")?,
        Value::string("()=> /*keep*/ 7")
    );
    realm.evaluate("let saved; async function a(){saved=()=>7;await 1}a()")?;
    assert_eq!(realm.evaluate("saved.toString()")?, Value::string("()=>7"));
    Ok(())
}
