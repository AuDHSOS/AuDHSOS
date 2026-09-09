// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use super::{Error, Limits, Runtime, SilentHost, Value, compile, eval};
#[test]
fn character_methods_keep_code_units_and_code_points_distinct() {
    for source in [
        "'abc'.at(-1)==='c'&&'abc'.at(-4)===undefined&&'abc'.at(NaN)==='a'",
        "'abc'.charAt(-1)===''&&'abc'.charAt(1.9)==='b'&&'abc'.charAt(Infinity)===''",
        "'abc'.charCodeAt(1)===98&&isNaN('abc'.charCodeAt(99))&&'abc'.charCodeAt()===97",
        "'😀x'.codePointAt(0)===128512&&'😀x'.codePointAt(1)===56832&&'😀x'.codePointAt(3)===undefined",
        "'😀'.at(0)==='\\ud83d'&&'😀'.at(-1)==='\\ude00'",
        "'\\ud800'.codePointAt(0)===55296&&'\\udc00'.charCodeAt(0)===56320",
        "String.prototype.charAt.call(123,1)==='2'&&String.prototype.at.call(true,-1)==='e'",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}
#[test]
fn search_boundaries_overlap_and_regexp_hooks() {
    for source in [
        "'ababa'.indexOf('aba')===0&&'ababa'.indexOf('aba',1)===2&&'ababa'.lastIndexOf('aba')===2&&'ababa'.lastIndexOf('aba',1)===0",
        "'abc'.indexOf('',99)===3&&'abc'.lastIndexOf('',99)===3&&'abc'.lastIndexOf('',-1)===0",
        "'abc'.lastIndexOf('b',NaN)===1&&'abc'.indexOf('b',NaN)===1&&'abc'.indexOf('a',Infinity)===-1",
        "'abc'.includes('b')&&!'abc'.includes('b',2)&&'abc'.includes('',Infinity)",
        "'abc'.startsWith('bc',1)&&'abc'.endsWith('ab',2)&&!'abc'.endsWith('ab')&&'abc'.startsWith('',Infinity)",
        "'abc'.endsWith('',-Infinity)&&!'a'.endsWith('ab')",
        "'a\\ud800b'.indexOf('\\ud800')===1&&'😀'.indexOf('\\ude00')===1",
        "let r=/a/;r[Symbol.match]=false;r.toString=()=> 'a';'abc'.startsWith(r)",
        "let r=/a/;r[Symbol.match]=null;r.toString=()=> 'a';'abc'.includes(r)",
        "let r={get [Symbol.match](){throw 7}};let ok=false;try{'x'.includes(r)}catch(e){ok=e===7}ok",
        "let r={toString(){return 'b'},get [Symbol.match](){throw 7}};'abc'.indexOf(r)===1",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for method in ["includes", "startsWith", "endsWith"] {
        assert_eq!(
            eval(&format!(
                "let ok=false;try{{'x'.{method}({{[Symbol.match]:true}})}}catch(e){{ok=e instanceof TypeError}}ok"
            )),
            Ok(Value::Boolean(true))
        );
    }
}
#[test]
fn slicing_padding_repeating_concat_trim_and_well_formed() {
    for source in [
        "'abcdef'.slice(-4,-1)==='cde'&&'abc'.slice(2,1)===''&&'abc'.substring(2,1)==='b'",
        "'abc'.substring(-7,2)==='ab'&&'abc'.slice(-Infinity,Infinity)==='abc'",
        "'abc'.substring(NaN,undefined)==='abc'&&'abc'.slice(undefined,undefined)==='abc'",
        "'ab'.repeat(2.9)==='abab'&&'ab'.repeat(-0.5)===''&&''.repeat(1e100)===''",
        "'x'.padStart(4,'ab')==='abax'&&'x'.padEnd(4,'ab')==='xaba'&&'x'.padStart(3)==='  x'",
        "'x'.padStart(Infinity,'')==='x'&&'abc'.padEnd(2,{toString(){throw 7}})==='abc'",
        "'x'.concat(1,null,undefined)==='x1nullundefined'",
        "' \\t\\n\\u00a0x\\ufeff'.trim()==='x'&&' x '.trimStart()==='x '&&' x '.trimEnd()===' x'",
        "'\\u0085\\u180e'.trim()==='\\u0085\\u180e'",
        "String.prototype.trimLeft===String.prototype.trimStart&&String.prototype.trimRight===String.prototype.trimEnd",
        "'😀'.isWellFormed()&&!'\\ud800x'.isWellFormed()&&!'\\udc00'.isWellFormed()&&''.isWellFormed()",
        "'\\ud800x\\udc00😀'.toWellFormed()==='�x�😀'&&'abc'.toWellFormed()==='abc'",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for expr in [
        "'x'.repeat(-1)",
        "''.repeat(Infinity)",
        "'x'.repeat(-Infinity)",
    ] {
        assert!(matches!(eval(expr), Err(Error::Range { .. })), "{expr}");
    }
}
#[test]
fn methods_convert_in_order_and_preserve_gc_roots() -> Result<(), Error> {
    for source in [
        "let log='';let o={toString(){log+='s';return 'abc'}},v={toString(){log+='v';return 'b'},get [Symbol.match](){log+='m';return false}},p={valueOf(){log+='p';return 1}};String.prototype.includes.call(o,v,p)&&log==='smvp'",
        "let log='';String.prototype.substring.call({toString(){log+='s';return 'abc'}},{valueOf(){log+='a';return 2}},{valueOf(){log+='b';return 1}});log==='sab'",
        "let n=0;try{String.prototype.charAt.call(null,{valueOf(){n++}})}catch(e){}n===0",
        "let n=0;try{String.prototype.includes.call({toString(){throw 7}},{get [Symbol.match](){n++}})}catch(e){}n===0",
        "let n=0;try{'x'.startsWith({[Symbol.match]:true},{valueOf(){n++}})}catch(e){}n===0",
        "let p={valueOf(){for(let i=0;i<300;i++){let g={}}return 1}};String.prototype.charAt.call({toString(){return 'abc'}},p)==='b'",
        "let f={toString(){for(let i=0;i<300;i++){let g={}}return 'xy'}};'a'.concat(f,'b')==='axyb'",
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
    for method in [
        "at",
        "charAt",
        "charCodeAt",
        "codePointAt",
        "slice",
        "substring",
        "concat",
        "indexOf",
        "lastIndexOf",
        "repeat",
        "padEnd",
        "trim",
        "toWellFormed",
    ] {
        assert!(
            matches!(
                eval(&format!("String.prototype.{method}.call(Symbol(),0)")),
                Err(Error::Type { .. })
            ),
            "{method}"
        );
        assert_eq!(
            eval(&format!(
                "Object.isExtensible(String.prototype.{method})&&String.prototype.{method}.prototype===undefined"
            )),
            Ok(Value::Boolean(true))
        );
    }
    Ok(())
}
#[test]
fn string_work_and_output_are_bounded() -> Result<(), Error> {
    for source in [
        "'x'.repeat(1000000)",
        "'x'.padEnd(1000000)",
        "'x'.padStart(Infinity)",
        "'abcd'.repeat(1000).indexOf('bcdx'.repeat(10))",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(
            matches!(
                Runtime::new(Limits {
                    string_units: 512,
                    fuel: 1000,
                    ..Limits::default()
                })
                .run(&program, &mut SilentHost),
                Err(Error::Limit { .. })
            ),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn native_search_is_linear_on_repeated_prefixes_and_arguments_keep_brand() -> Result<(), Error> {
    for source in [
        "let text='a'.repeat(10000)+'b',needle='a'.repeat(1000)+'b';text.indexOf(needle)===9000&&text.lastIndexOf(needle)===9000",
        "let text='a'.repeat(10000)+'b',needle='a'.repeat(1000)+'b';text.split(needle)[0].length===9000",
        "let text='a'.repeat(10000)+'b',needle='a'.repeat(1000)+'b';text.replace(needle,'x').length===9001",
        "function f(){return String.prototype.trim.call(arguments)}f()==='[object Arguments]'",
        "function f(){'use strict';return String.prototype.trim.call(arguments)}f()==='[object Arguments]'",
        "function f(...a){arguments[Symbol.toStringTag]='Other';return Object.prototype.toString.call(arguments)}f()==='[object Other]'",
    ] {
        let limits = Limits {
            fuel: 150_000,
            ..Limits::default()
        };
        assert_eq!(
            Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
            Value::Boolean(true),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn string_edge_cases_limits_and_method_metadata() -> Result<(), Error> {
    for source in [
        "'x'.padStart(2,'')==='x'&&'x'.padEnd(NaN)==='x'&&'x'.padStart(-1)==='x'",
        "let n=0;try{'abc'.lastIndexOf('long',{valueOf(){n++;throw 7}})}catch(e){}n===1",
        "'abc'.startsWith('',-1)&&'abc'.endsWith('',NaN)&&!'abc'.startsWith('long')",
        "'abc'.slice(2,-Infinity)===''&&'abc'.substring(Infinity,0)==='abc'",
        "'abc'.at(-Infinity)===undefined&&'abc'.codePointAt(-1)===undefined&&''.charAt()===''",
        "'x'.padEnd(4,'\\ud800')==='x\\ud800\\ud800\\ud800'",
        "'abc'.concat()==='abc'&&''.repeat(NaN)===''&&'abc'.repeat(0)===''",
        "''.trim()===''&&'   '.trim()===''&&'abc'.trim()==='abc'",
        "let s='\\ud800\\ud800\\udc00\\udc00';s.toWellFormed()==='�\\ud800\\udc00�'",
        "let f=String.prototype.indexOf;f.x=7;delete f.length;f.x===7&&!Object.hasOwn(f,'length')&&f.call('abc','b')===1",
    ] {
        assert_eq!(eval(source)?, Value::Boolean(true), "{source}");
    }
    for source in [
        "String.prototype.includes.call(null,'')",
        "''.indexOf(Symbol())",
        "''.slice(Symbol())",
        "''.padEnd(3,Symbol())",
        "''.concat(Symbol())",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
    for source in [
        "'abcdef'.concat('abcdef')",
        "'x'.padStart(30,'abcdef')",
        "'abc'.repeat(10)",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(
            matches!(
                Runtime::new(Limits {
                    string_units: 10,
                    ..Limits::default()
                })
                .run(&program, &mut SilentHost),
                Err(Error::Limit { .. })
            ),
            "{source}"
        );
    }
    Ok(())
}
