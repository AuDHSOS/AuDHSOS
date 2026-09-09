// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::{Error, Limits, Runtime, SilentHost, Value, compile, eval};

#[test]
fn parse_strict_json_values_descriptors_and_duplicate_keys() {
    for source in [
        r"JSON.parse('null')===null&&JSON.parse('true')===true&&JSON.parse('false')===false",
        r"1/JSON.parse('-0')===-Infinity&&JSON.parse('1e400')===Infinity&&JSON.parse('1e-400')===0",
        r"JSON.parse('1.25e+3')===1250&&JSON.parse(' \t\r\n42\n')===42",
        r#"let a=JSON.parse('[1,null,true,"x",{}]');Array.isArray(a)&&a.length===5&&a[3]==='x'"#,
        r#"let a=JSON.parse('{"2":2,"b":1,"1":1,"b":3,"a":4}');Object.keys(a).join()==='1,2,b,a'&&a.b===3"#,
        r#"let o=JSON.parse('{"__proto__":1,"__proto__":{"x":2}}');Object.getPrototypeOf(o)===Object.prototype&&o.__proto__.x===2&&Object.keys(o).join()==='__proto__'"#,
        r#"let d=Object.getOwnPropertyDescriptor(JSON.parse('{"x":1}'),'x');d.value===1&&d.writable&&d.enumerable&&d.configurable"#,
        r"let d=Object.getOwnPropertyDescriptor(JSON.parse('[1]'),'length');d.value===1&&d.writable&&!d.enumerable&&!d.configurable",
        r"JSON.parse({toString(){return '7'}})===7&&JSON.parse('8',{})===8",
        r#"JSON.parse('"\\ud800"')==='\ud800'&&JSON.parse('"\\udc00"')==='\udc00'"#,
        r#"Object.defineProperty(Object.prototype,'x',{set(){throw 7},configurable:true});JSON.parse('{"x":1}').x===1"#,
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for text in [
        "",
        "undefined",
        "NaN",
        "Infinity",
        "+1",
        "01",
        "-01",
        ".1",
        "1.",
        "1e+",
        "[1,]",
        "{\"a\":1,}",
        "{'a':1}",
        "/*x*/0",
        "1 2",
        "\u{feff}1",
        "\u{a0}1",
        "\"\n\"",
        "\"\\x41\"",
    ] {
        // A JS string literal carrying the exact invalid JSON, without another JSON codec.
        let literal = text
            .replace('\\', "\\\\")
            .replace('\'', "\\'")
            .replace('\n', "\\n");
        let source =
            format!("try{{JSON.parse('{literal}');false}}catch(e){{e instanceof SyntaxError}}");
        assert_eq!(eval(&source), Ok(Value::Boolean(true)), "{text:?}");
    }
    for source in [
        "JSON.parse(Symbol())",
        "JSON.parse({toString(){return {}}})",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn stringify_primitives_arrays_wrappers_and_well_formed_utf16() {
    for (source, expected) in [
        ("JSON.stringify(null)", "null"),
        ("JSON.stringify(true)", "true"),
        ("JSON.stringify(false)", "false"),
        ("JSON.stringify(-0)", "0"),
        (
            "JSON.stringify([NaN,Infinity,-Infinity,undefined,Symbol(),function(){},,])",
            "[null,null,null,null,null,null,null]",
        ),
        (
            "JSON.stringify({a:undefined,b:Symbol(),c:function(){},d:null,e:1})",
            r#"{"d":null,"e":1}"#,
        ),
        (
            "JSON.stringify([new Number(3),new String('x'),new Boolean(false),Object(Symbol())])",
            r#"[3,"x",false,{}]"#,
        ),
        (
            "JSON.stringify({2:2,b:1,1:1,a:3})",
            r#"{"1":1,"2":2,"b":1,"a":3}"#,
        ),
        (r"JSON.stringify('\ud800x\udc00😀')", r#""\ud800x\udc00😀""#),
        (
            r#"JSON.stringify('\x00\b\f\n\r\t\"\\/')"#,
            r#""\u0000\b\f\n\r\t\"\\/""#,
        ),
        (
            "let a=[1,,];a.other=7;Array.prototype[1]=2;JSON.stringify(a)",
            "[1,2]",
        ),
        ("let a={x:1};JSON.stringify([a,a])", r#"[{"x":1},{"x":1}]"#),
        (
            "JSON.stringify([1e-7,1e-6,1e20,1e21])",
            "[1e-7,0.000001,100000000000000000000,1e+21]",
        ),
    ] {
        assert_eq!(eval(source), Ok(Value::string(expected)), "{source}");
    }
    for source in [
        "JSON.stringify()",
        "JSON.stringify(undefined)",
        "JSON.stringify(Symbol())",
        "JSON.stringify(function(){})",
    ] {
        assert_eq!(eval(source), Ok(Value::Undefined), "{source}");
    }
    for source in [
        "let a={};a.a=a;JSON.stringify(a)",
        "let a=[];a[0]=a;JSON.stringify(a)",
        "let a={};JSON.stringify(a,function(k,v){return k===''?{x:a}:this})",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn stringify_hooks_order_receivers_and_mutation_snapshots() {
    for source in [
        r#"let log='';let o={get toJSON(){log+='g';return function(k){log+='t'+k;return {a:1}}}};let s=JSON.stringify(o,function(k,v){log+='r'+k;return v});s==='{"a":1}'&&log==='gtrra'"#,
        r#"let o={x:{toJSON(k){return this===o.x?k:'bad'}}};JSON.stringify(o)==='{"x":"x"}'"#,
        r"let o={a:1};let root=false;JSON.stringify(o,function(k,v){if(k==='')root=this['']===o;else if(this!==o)throw 1;return v});root",
        r#"let o={a:1,b:2};JSON.stringify(o,function(k,v){if(k==='a'){delete this.b;this.c=3}return v})==='{"a":1}'"#,
        r"let a=[1,2];JSON.stringify(a,function(k,v){if(k==='0'){this.length=1;this[3]=4}return v})==='[1,null]'",
        r#"let o={a:1,b:2};Object.defineProperty(o,'a',{get(){Object.defineProperty(o,'b',{enumerable:false});return 1}});JSON.stringify(o)==='{"a":1,"b":2}'"#,
        r"let o={};o[Symbol()]=1;Object.defineProperty(o,'hidden',{value:2});JSON.stringify(o)==='{}'",
        r#"let n=new Number(7);n.valueOf=function(){return 4};let s=new String('x');s.toString=function(){return 'y'};let b=new Boolean(true);b.valueOf=function(){throw 7};JSON.stringify([n,s,b])==='[4,"y",true]'"#,
        r"let n=0;let f=function(){};f.toJSON=function(){n++;return 7};JSON.stringify(f)==='7'&&n===1",
        r#"JSON.stringify({a:1},function(k,v){return k==='a'?{toJSON(){throw 7},b:2}:v})==='{"a":{"b":2}}'"#,
        r"let o={};o.self=o;JSON.stringify(o,function(k,v){return k==='self'?undefined:v})==='{}'",
        r"let n=0;try{JSON.stringify({get a(){throw 7},get b(){n++}})}catch(e){if(e!==7)throw e}n===0",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn replacer_property_list_and_gap_follow_coercion_order() {
    for (source, expected) in [
        (
            "JSON.stringify({a:1,b:2,1:3},['b','a','b',1,new String('a'),true,{},null])",
            r#"{"b":2,"a":1,"1":3}"#,
        ),
        (
            "let o=Object.create({inherited:2});Object.defineProperty(o,'hidden',{value:1});JSON.stringify(o,['hidden','inherited'])",
            r#"{"hidden":1,"inherited":2}"#,
        ),
        (
            "JSON.stringify({a:{a:1,b:2},b:3},['a'])",
            r#"{"a":{"a":1}}"#,
        ),
        ("JSON.stringify([1,2],[])", "[1,2]"),
        (
            "JSON.stringify({a:[1,{}]},null,2)",
            "{\n  \"a\": [\n    1,\n    {}\n  ]\n}",
        ),
        ("JSON.stringify([1],null,Infinity)", "[\n          1\n]"),
        ("JSON.stringify([1],null,1.9)", "[\n 1\n]"),
        (
            "JSON.stringify([1],null,'abcdefghijkl')",
            "[\nabcdefghij1\n]",
        ),
        ("JSON.stringify([1],null,new String('--'))", "[\n--1\n]"),
        ("JSON.stringify([1],null,new Number(2))", "[\n  1\n]"),
        ("JSON.stringify({a:undefined},null,2)", "{}"),
    ] {
        assert_eq!(eval(source), Ok(Value::string(expected)), "{source}");
    }
    for source in [
        "let log='';let a=[new String('x')];a[0].toString=function(){log+='r';return 'x'};let gap=new Number(2);gap.valueOf=function(){log+='s';return 2};JSON.stringify({toJSON(){log+='t';return {x:1}}},a,gap);log==='rst'",
        "let a=['a','b'];Object.defineProperty(a,0,{get(){a.length=1;return 'a'}});JSON.stringify({a:1,b:2},a)==='{\"a\":1}'",
        "let n=0;JSON.stringify([1],null,{toString(){n++}});n===0",
        "JSON.stringify([1],null,NaN)==='[1]'&&JSON.stringify([1],null,-Infinity)==='[1]'",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn reviver_postorder_context_source_and_deletion() {
    for source in [
        r#"let log='';let a=JSON.parse('{"2":2,"a":[1,true]}',function(k,v,c){log+=k+':'+c.source+';';return v});log==='2:2;0:1;1:true;a:undefined;:undefined;'&&a.a[1]"#,
        r#"let src='';JSON.parse(' {"x":1e+2,"x":1E2,"s":"\\u0041","n":-0} ',function(k,v,c){if(k)src+=c.source+'|';return v});src==='1E2|"\\u0041"|-0|'"#,
        r#"let x=JSON.parse('{"a":1,"b":2}',function(k,v){return k==='a'?undefined:v});!('a' in x)&&x.b===2"#,
        r"let x=JSON.parse('[1,2]',function(k,v){return k==='0'?undefined:v});x.length===2&&!(0 in x)&&x[1]===2",
        r"JSON.parse('1',function(k,v,c){return k===''&&this['']===1&&arguments.length===3&&c.source==='1'})===true",
        r"JSON.parse('1',function(){})===undefined",
        r"let last;let unique=true;JSON.parse('[1,2]',function(k,v,c){if(c===last)unique=false;last=c;return v});unique",
        r#"let a=JSON.parse('{"x":1}',function(k,v,c){if(k==='x'){let d=Object.getOwnPropertyDescriptor(c,'source');if(!d.writable||!d.enumerable||!d.configurable)throw 7}return v});a.x===1"#,
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn reviver_mutations_disable_sources_and_ignore_rejected_definitions() {
    for source in [
        r#"let log='';JSON.parse('{"a":1,"b":2}',function(k,v,c){if(k==='a')this.b=3;if(k==='b')log=typeof c.source;return v});log==='undefined'"#,
        r#"let log='';JSON.parse('{"a":1,"b":{"x":2}}',function(k,v,c){if(k==='a')this.b={x:2};if(k==='x')log=typeof c.source;return v});log==='undefined'"#,
        r#"let log='';JSON.parse('{"a":1,"b":-0}',function(k,v,c){if(k==='a')this.b=0;if(k==='b')log=typeof c.source;return v});log==='undefined'"#,
        r#"let log='';JSON.parse('{"a":1,"b":2}',function(k,v,c){if(k==='a')this.b=2;if(k==='b')log=c.source;return v});log==='2'"#,
        r#"let x=JSON.parse('{"a":1,"b":2}',function(k,v){if(k==='a')Object.freeze(this);return k==='b'?undefined:3});x===3"#,
        r"let x=JSON.parse('[1,2]',function(k,v){if(k==='0')Object.freeze(this);return k===''?v:undefined});x[0]===1&&x[1]===2",
        r#"let x=JSON.parse('{"a":1}',function(k,v){if(k==='a'){Object.defineProperty(this,k,{get(){return 7},configurable:false});return 9}return v});x.a===7"#,
        r"let log='';JSON.parse('[1,2]',function(k,v,c){log+=k+':'+c.source+';';if(k==='0'){delete this[1];this.push(3)}return v});log==='0:1;1:undefined;:undefined;'",
        r#"let log='';JSON.parse('{"a":1,"b":2}',function(k,v,c){if(k==='a'){Object.defineProperty(this,'b',{get(){return 2}});this.c=3}log+=k;return v});log==='ab'"#,
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn raw_json_brand_integrity_and_hook_integration() {
    for source in [
        r#"let r=JSON.rawJSON('9007199254740993');JSON.isRawJSON(r)&&JSON.stringify({r})==='{"r":9007199254740993}'"#,
        r"let r=JSON.rawJSON('-0');Object.getPrototypeOf(r)===null&&Object.isFrozen(r)&&r.rawJSON==='-0'&&Object.keys(r).join()==='rawJSON'",
        r"!JSON.isRawJSON({rawJSON:'1'})&&!JSON.isRawJSON(1)&&!JSON.isRawJSON()&&!JSON.isRawJSON(null)&&!JSON.isRawJSON(Symbol())",
        r"JSON.stringify({toJSON(){return JSON.rawJSON('1e400')}})==='1e400'",
        r"JSON.stringify(1,function(){return JSON.rawJSON('false')})==='false'",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for text in ["", " 1", "1 ", "[]", "{}", "NaN", "undefined", "1\n"] {
        let literal = text.replace('\n', "\\n");
        let source =
            format!("try{{JSON.rawJSON('{literal}');false}}catch(e){{e instanceof SyntaxError}}");
        assert_eq!(eval(&source), Ok(Value::Boolean(true)), "{source}");
    }
    assert_eq!(
        eval("JSON.stringify(JSON.rawJSON('true'),function(k,v){return k===''?7:v})"),
        Ok(Value::string("7"))
    );
}

#[test]
fn json_intrinsic_and_persistent_realm_identity() -> Result<(), Error> {
    for source in [
        "Object.getPrototypeOf(JSON)===Object.prototype&&Object.prototype.toString.call(JSON)==='[object JSON]'",
        "globalThis.JSON===JSON",
        "Object.keys(JSON).length===0&&JSON.parse.length===2&&JSON.stringify.length===3&&JSON.rawJSON.length===1&&JSON.isRawJSON.length===1",
        "JSON.parse.name==='parse'&&JSON.stringify.name==='stringify'&&JSON.rawJSON.name==='rawJSON'&&JSON.isRawJSON.name==='isRawJSON'",
        "let d=Object.getOwnPropertyDescriptor(JSON,'parse');d.writable&&!d.enumerable&&d.configurable",
        "let d=Object.getOwnPropertyDescriptor(JSON,Symbol.toStringTag);d.value==='JSON'&&!d.writable&&!d.enumerable&&d.configurable",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "JSON()",
        "new JSON()",
        "new JSON.parse('1')",
        "new JSON.stringify(1)",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
    let mut host = SilentHost;
    let mut realm = crate::Realm::new(Limits::default(), &mut host)?;
    realm.evaluate("let saved=JSON;let parse=JSON.parse;JSON.x=7")?;
    assert_eq!(
        realm.evaluate("JSON===saved&&globalThis.JSON===saved&&parse('7')===JSON.x")?,
        Value::Boolean(true)
    );
    Ok(())
}

#[test]
fn json_native_functions_have_ordinary_mutable_identity() -> Result<(), Error> {
    for source in [
        "Object.isExtensible(JSON.parse)&&Object.isExtensible(JSON.stringify)&&Object.isExtensible(JSON.rawJSON)&&Object.isExtensible(JSON.isRawJSON)",
        "let f=JSON.parse;Object.getPrototypeOf(f)===Function.prototype&&typeof f==='function'&&f.prototype===undefined",
        "Object.getOwnPropertyNames(JSON.parse).join()==='length,name'",
        "let f=JSON.parse;f.x=7;delete f.name;Object.defineProperty(f,'length',{value:9});f.x===7&&f.length===9&&!Object.hasOwn(f,'name')&&f('1')===1",
        "let f=JSON.stringify;Object.freeze(f);Object.isFrozen(f)&&f(7)==='7'",
        "let f=JSON.rawJSON;f.toJSON=function(){return 7};JSON.stringify(f)==='7'",
        "let f=JSON.isRawJSON;let p={x:7};Object.setPrototypeOf(f,p);f.x===7&&f(JSON.rawJSON('1'))",
        "let a=JSON.parse,b=JSON.parse;let m=new WeakMap();m.set(a,7);a===b&&m.get(b)===7",
    ] {
        assert_eq!(eval(source)?, Value::Boolean(true), "{source}");
    }
    Ok(())
}

#[test]
fn json_values_survive_gc_in_hooks_and_limits_are_fatal() -> Result<(), Error> {
    let limits = Limits {
        heap_entries: 180,
        ..Limits::default()
    };
    for source in [
        r#"let a=JSON.parse('{"a":[{"x":1},{"y":2}]}',function(k,v){for(let i=0;i<300;i++){let garbage={}}return v});JSON.stringify(a)==='{"a":[{"x":1},{"y":2}]}'"#,
        r#"let o={get a(){for(let i=0;i<300;i++){let garbage={}}return {x:1}},b:{y:2}};JSON.stringify(o,function(k,v){for(let i=0;i<300;i++){let garbage={}}return v})==='{"a":{"x":1},"b":{"y":2}}'"#,
        r#"let a=[];Object.defineProperty(a,0,{get(){let n=new String('x');n.toString=function(){for(let i=0;i<300;i++){let garbage={}}return 'x'};return n}});JSON.stringify({x:7},a)==='{"x":7}'"#,
        r#"for(let i=0;i<100;i++){try{JSON.stringify({get x(){throw {i}}})}catch(e){if(e.i!==i)throw 1}}JSON.stringify({x:1})==='{"x":1}'"#,
    ] {
        assert_eq!(
            Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
            Value::Boolean(true),
            "{source}"
        );
    }
    for source in [
        "let a={};let b=a;for(let i=0;i<60;i++){b.x={};b=b.x}JSON.stringify(a)",
        "let s='0';for(let i=0;i<60;i++){s='['+s+']'}JSON.parse(s)",
        "JSON.parse('{\"a\":1,\"b\":2}',function(k,v){if(k==='a')this.b=this;return v})",
        "JSON.stringify(1,function(){return JSON.stringify(1,arguments.callee)})",
    ] {
        assert!(matches!(eval(source), Err(Error::Limit { .. })), "{source}");
    }
    let program = compile("JSON.stringify([1,2,3,4,5])", Limits::default())?;
    assert!(matches!(
        Runtime::new(Limits {
            string_units: 10,
            ..Limits::default()
        })
        .run(&program, &mut SilentHost),
        Err(Error::Limit { .. })
    ));
    let limits = Limits {
        fuel: 300,
        ..Limits::default()
    };
    let program = compile("JSON.stringify(new Array(1000000))", limits)?;
    assert!(matches!(
        Runtime::new(limits).run(&program, &mut SilentHost),
        Err(Error::Limit { .. })
    ));
    let program = compile(
        "JSON.stringify({abcdefghijklmno:undefined})",
        Limits::default(),
    )?;
    assert_eq!(
        Runtime::new(Limits {
            string_units: 16,
            ..Limits::default()
        })
        .run(&program, &mut SilentHost)?,
        Value::string("{}")
    );
    let source = "let o={};let p=o;for(let i=0;i<35;i++){p.x={};p=p.x}p.toJSON=function(){return JSON.stringify(o)};JSON.stringify(o)";
    assert!(matches!(eval(source), Err(Error::Limit { .. })));
    Ok(())
}
