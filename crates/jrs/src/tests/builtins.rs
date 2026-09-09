// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#[test]
fn object_predicates_order_symbols_boxing_and_inheritance() {
    for source in [
        "let o={a:1};o.propertyIsEnumerable('a')&&!o.propertyIsEnumerable('b')&&!o.propertyIsEnumerable('toString')",
        "let s=Symbol(),o={};o[s]=7;o.propertyIsEnumerable(s)&&!Object.prototype.propertyIsEnumerable.call(o,'s')",
        "Object.prototype.propertyIsEnumerable.call('x',0)&&!Object.prototype.propertyIsEnumerable.call('x','length')",
        "let o={get x(){throw 7}};o.propertyIsEnumerable('x')",
        "let e={};let yes=false;try{Object.prototype.propertyIsEnumerable.call(null,{toString(){throw e}})}catch(x){yes=x===e}yes",
        "let p={},o=Object.create(p);p.isPrototypeOf(o)&&Object.prototype.isPrototypeOf(o)&&!o.isPrototypeOf(p)&&!o.isPrototypeOf(o)",
        "!Object.prototype.isPrototypeOf.call(null,7)&&!Object.prototype.isPrototypeOf.call(undefined,null)",
        "Function.prototype.isPrototypeOf(JSON.parse)&&Object.prototype.isPrototypeOf(JSON.parse)",
        "!Number.isPrototypeOf({})&&Number.propertyIsEnumerable('prototype')===false&&Object.prototype.isPrototypeOf(Number)",
        "Object.prototype.propertyIsEnumerable.name==='propertyIsEnumerable'&&Object.prototype.propertyIsEnumerable.length===1&&Object.isExtensible(Object.prototype.propertyIsEnumerable)",
        "let yes=false;try{Object.prototype.isPrototypeOf.call(null,{})}catch(e){yes=e instanceof TypeError}yes",
        "let yes=false;try{Object.prototype.propertyIsEnumerable.call(null,'x')}catch(e){yes=e instanceof TypeError}yes",
    ] {
        assert_eq!(
            super::eval(source),
            Ok(crate::Value::Boolean(true)),
            "{source}"
        );
    }
}

use super::{Error, Limits, Runtime, SilentHost, Value, compile, eval};

#[test]
fn stable_sort_preserves_holes_order_and_generic_receivers() {
    for (source, expected) in [
        ("[20,3,100,2].sort().join()", "100,2,20,3"),
        ("[20,3,100,2].sort((a,b)=>a-b).join()", "2,3,20,100"),
        (
            "let a=[{k:1,id:'a'},{k:0,id:'b'},{k:1,id:'c'},{k:0,id:'d'}];a.sort((x,y)=>x.k-y.k).map(x=>x.id).join()",
            "b,d,a,c",
        ),
        (
            "let a=[,undefined,3,,1];a.sort();a.join()+'|'+Object.keys(a).join()",
            "1,3,,,|0,1,2",
        ),
        (
            "let a={0:'z',2:'a',length:4};Array.prototype.sort.call(a);a[0]+a[1]+'|'+Object.keys(a).join()",
            "az|0,1,length",
        ),
        ("let a=[3,1,2];a.sort(()=>NaN).join()", "3,1,2"),
        (
            "let a=[2,1];a.sort(()=>({valueOf(){return -1}})).join()",
            "2,1",
        ),
        (
            "let a=[2,undefined,1];a.sort((a,b)=>{if(a===undefined||b===undefined)throw 1;return a-b}).join()",
            "1,2,",
        ),
        (
            "let a=[2,1];let p={1:3};Object.setPrototypeOf(a,p);delete a[1];Array.prototype.sort.call(a);Array.prototype.join.call(a)",
            "2,3",
        ),
    ] {
        assert_eq!(eval(source), Ok(Value::string(expected)), "{source}");
    }
    assert_eq!(eval("let a=[];a.sort()===a"), Ok(Value::Boolean(true)));
}

#[test]
fn sort_observes_errors_before_writeback_and_bounds_work() -> Result<(), Error> {
    for (source, expected) in [
        (
            "let a={get length(){throw 1}};try{Array.prototype.sort.call(a,5)}catch(e){e.name}",
            "TypeError",
        ),
        (
            "let a=[2,1];try{a.sort(()=>{throw 7})}catch(e){}a.join()",
            "2,1",
        ),
        (
            "let a=[2,1];Object.freeze(a);try{a.sort()}catch(e){e.name}",
            "TypeError",
        ),
        (
            "let a=[,2];Object.defineProperty(a,1,{configurable:false});try{a.sort()}catch(e){e.name}",
            "TypeError",
        ),
        (
            "let a=[{toString(){throw 9}},1];try{a.sort()}catch(e){String(e)}",
            "9",
        ),
    ] {
        assert_eq!(eval(source), Ok(Value::string(expected)), "{source}");
    }
    let limits = Limits {
        fuel: 200,
        ..Limits::default()
    };
    assert!(matches!(
        Runtime::new(limits).run(
            &compile("Array.prototype.sort.call({length:100000})", limits)?,
            &mut SilentHost
        ),
        Err(Error::Limit { .. })
    ));
    let limits = Limits {
        heap_entries: 80,
        ..Limits::default()
    };
    let source = "let a=[{n:2},{n:1}];a.sort((x,y)=>{a.length=0;for(let i=0;i<150;i++){let t={};}return x.n-y.n});a[0].n+a[1].n";
    assert_eq!(
        Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
        Value::Number(3.0)
    );
    Ok(())
}

#[test]
fn define_properties_converts_all_descriptors_before_defining() {
    for (source, expected) in [
        (
            "let a=Object.create(null,{x:{value:2},y:{get(){return 3},enumerable:true}});a.x+a.y",
            Value::Number(5.0),
        ),
        (
            "let a={};Object.defineProperties(a,{x:{value:2,enumerable:true,writable:true}});a.x=3;Object.keys(a).join()+a.x",
            Value::string("x3"),
        ),
        (
            "let a={};try{Object.defineProperties(a,{x:{value:1},y:{get:1}})}catch(e){}Object.keys(a).length+('x' in a)",
            Value::Number(0.0),
        ),
        (
            "let a={x:0};let trace='';Object.defineProperties(a,{x:{get value(){trace+='a';return 1}},y:{get value(){trace+=a.x;return 2}}});trace+a.x+a.y",
            Value::string("a012"),
        ),
        (
            "let a={};Object.defineProperties(a,{get x(){delete this.y;return {value:1}},y:{value:2}});('x' in a)&&!('y' in a)",
            Value::Boolean(true),
        ),
        (
            "let a=Object.create({},{x:{value:1}});Object.getOwnPropertyDescriptor(a,'x').writable",
            Value::Boolean(false),
        ),
        (
            "let a={};Object.defineProperty(a,'x',{value:0});try{Object.defineProperties(a,{y:{value:1},x:{value:2},z:{value:3}})}catch(e){}('y' in a)&&!('z' in a)",
            Value::Boolean(true),
        ),
        (
            "let a={};let p=Object.create({inherited:{value:9}});Object.defineProperty(p,'hidden',{value:{value:3}});Object.defineProperties(a,p);Object.getOwnPropertyNames(a).length",
            Value::Number(0.0),
        ),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
}

#[test]
fn arguments_mapping_aliases_only_simple_sloppy_parameters() {
    for (source, expected) in [
        (
            "function f(a){arguments[0]=7;return a}f(1)",
            Value::Number(7.0),
        ),
        (
            "function f(a){a=8;return arguments[0]}f(1)",
            Value::Number(8.0),
        ),
        (
            "function f(a){'use strict';a=8;return arguments[0]}f(1)",
            Value::Number(1.0),
        ),
        (
            "function f(a=2){a=8;return arguments[0]}f(1)",
            Value::Number(1.0),
        ),
        (
            "function f(a){delete arguments[0];a=8;return arguments[0]}f(1)",
            Value::Undefined,
        ),
        (
            "function f(a){Object.defineProperty(arguments,'0',{writable:false});a=8;return arguments[0]}f(1)",
            Value::Number(1.0),
        ),
        (
            "function f(a){Object.defineProperty(arguments,'0',{value:6});return a}f(1)",
            Value::Number(6.0),
        ),
        (
            "function f(a){Object.defineProperty(arguments,'0',{get(){return 6}});a=8;return arguments[0]}f(1)",
            Value::Number(6.0),
        ),
        (
            "function f(a,a){arguments[0]=5;return a+arguments[0]}f(1,2)",
            Value::Number(7.0),
        ),
        (
            "function f(a){return ()=>arguments[0]}f(4)()",
            Value::Number(4.0),
        ),
        (
            "function f(){return arguments.callee===f}f()",
            Value::Boolean(true),
        ),
        (
            "function f(){'use strict';try{return arguments.callee}catch(e){return e.name}}f()",
            Value::string("TypeError"),
        ),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
    assert_eq!(
        eval("function f(a){Object.freeze(arguments);a=7;return arguments[0]}f(2)"),
        Ok(Value::Number(2.0))
    );
    assert_eq!(
        eval("function f(a){Object.seal(arguments);a=7;return arguments[0]}f(2)"),
        Ok(Value::Number(7.0))
    );
}

#[test]
fn apply_bind_and_error_intrinsics_have_observable_identity() {
    for (source, expected) in [
        (
            "function f(a,b){return this.n+a+b}f.apply({n:3},{0:1,1:2,length:2})",
            Value::Number(6.0),
        ),
        (
            "function f(a,b){return this.n+a+b}f.bind({n:3},1)(2)",
            Value::Number(6.0),
        ),
        (
            "function f(a,b){return this.n+a+b}f.bind({n:3},1).bind({n:9},2)()",
            Value::Number(6.0),
        ),
        ("function f(a,b){}f.bind(null,1).length", Value::Number(1.0)),
        ("function f(){}f.bind(null).name", Value::string("bound f")),
        (
            "function f(){return arguments.length}f.apply(null,null)",
            Value::Number(0.0),
        ),
        (
            "let a=new TypeError('bad',{cause:7});a instanceof Error && a instanceof TypeError && a.cause===7",
            Value::Boolean(true),
        ),
        ("String(new Error('bad'))", Value::string("Error: bad")),
        ("String(new RangeError())", Value::string("RangeError")),
        (
            "Error.prototype.toString.call({name:'',message:'bad'})",
            Value::string("bad"),
        ),
        ("Error.prototype.toString.call({})", Value::string("Error")),
        (
            "Error.isError(new SyntaxError())&&!Error.isError({})",
            Value::Boolean(true),
        ),
        (
            "Object.getPrototypeOf(TypeError)===Error",
            Value::Boolean(true),
        ),
        (
            "Object.prototype.toString.call(new URIError())",
            Value::string("[object Error]"),
        ),
        (
            "new ReferenceError().name+new EvalError().name",
            Value::string("ReferenceErrorEvalError"),
        ),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
    for source in [
        "function f(){};f.call=7;f.call===7",
        "function f(){};f.bind=7;f.bind===7",
        "function f(){};f.apply=7;f.apply===7",
        "function f(){};Object.defineProperty(f,'length',{value:2.9});f.bind(null,0).length===1",
        "function f(){};Object.defineProperty(f,'length',{value:Infinity});f.bind(null).length===Infinity",
        "function f(){};Object.defineProperty(f,'length',{value:NaN});f.bind(null).length===0",
        "function f(){};Object.defineProperty(f,'name',{value:'\\ud800'});f.bind(null).name==='bound \\ud800'",
        "function f(){};let bind=f.bind;let p={length:9};delete f.length;Object.setPrototypeOf(f,p);let b=bind.call(f,null);b.length===0 && Object.getPrototypeOf(b)===p",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn split_replace_and_character_code_preserve_utf16_and_substitutions() {
    for (source, expected) in [
        ("'a,b,c'.split(',').join('|')", "a|b|c"),
        ("'a,b,c'.split(',',2).join('|')", "a|b"),
        ("'a,b,c'.split(',',0).join('|')", ""),
        ("'abc'.split('').join('|')", "a|b|c"),
        ("'abc'.split().join('|')", "abc"),
        ("'abc'.replace('b',\"[$$][$&][$`][$']\")", "a[$][b][a][c]c"),
        ("'xab'.replace(/(a)(b)/,'$2$1$01$10$99')", "xbaaa0$99"),
        ("'b'.replace(/(a)?b/,'$1x')", "x"),
        ("'a1a2'.replace(/a(.)/g,(m,c,i,s)=>c+i+s.length)", "104224"),
        ("'ab'.replace(/(?:)/g,'-')", "-a-b-"),
        ("'abc'.replace('x','q')", "abc"),
        ("'abc'.replace('','-')", "-abc"),
        ("'abc'.replace('b',()=>({toString(){return 'q'}}))", "aqc"),
    ] {
        assert_eq!(eval(source), Ok(Value::string(expected)), "{source}");
    }
    assert_eq!(
        eval("String.fromCharCode(65,65536+66,-1)"),
        Ok(Value::String(alloc::rc::Rc::from([65, 66, 65535])))
    );
    assert_eq!(eval("'😀'.split('').length"), Ok(Value::Number(2.0)));
}
