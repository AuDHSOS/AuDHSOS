// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use super::{Error, Limits, Runtime, SilentHost, Value, compile, eval};

#[test]
fn string_replace_dispatch_preserves_raw_arguments_and_order() {
    for source in [
        "let raw={toString(){throw 7}},replacement={toString(){throw 8}},p={[Symbol.replace](s,r){return this===p&&s===raw&&r===replacement&&arguments.length===2}};String.prototype.replace.call(raw,p,replacement)",
        "let n=0,p={get [Symbol.replace](){n++;throw 7}};try{String.prototype.replace.call(null,p)}catch(e){}n===0",
        "let log='',p={get [Symbol.replace](){log+='h'},toString(){log+='p';return 'a'}};String.prototype.replace.call({toString(){log+='s';return 'a'}},p,{toString(){log+='r';return 'x'}})==='x'&&log==='hspr'",
        "let n=0;let p={[Symbol.replace]:null,toString(){n++;return 'z'}};'a'.replace(p,{toString(){n++;return 'x'}})==='a'&&n===2",
        "let result={};'a'.replace({[Symbol.replace](){return result}},'x')===result",
        "String.prototype.replace.call(Symbol(),{[Symbol.replace](){return 7}},Symbol())===7",
        "Number.prototype[Symbol.replace]=()=>{throw 7};'a1b'.replace(1,'x')==='axb'",
        "let r=/a/g;r[Symbol.replace]=undefined;r.toString=()=> 'a';'aa'.replace(r,'x')==='xa'",
        "let args,receiver;'aba'.replace('a',function(){'use strict';receiver=this;args=arguments;return 7})==='7ba'&&receiver===undefined&&args.length===3&&args[0]==='a'&&args[1]===0&&args[2]==='aba'",
        "let n=0;'abc'.replace('x',()=>{n++})==='abc'&&n===0",
        "let e={},ok=false;try{'a'.replace({get [Symbol.replace](){throw e}},'x')}catch(x){ok=x===e}ok",
        "let e={},ok=false;try{'a'.replace('a',()=>({toString(){throw e}}))}catch(x){ok=x===e}ok",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "'a'.replace({[Symbol.replace]:7},'x')",
        "String.prototype.replace.call(undefined,'a','x')",
        "'a'.replace(Symbol(),'x')",
        "'a'.replace('x',Symbol())",
        "'a'.replace('a',()=>Symbol())",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn regex_replace_collects_all_matches_before_replacement_hooks() {
    for source in [
        "'aba'.replace(/a/g,'x')==='xbx'&&'aba'.replace(/a/,'x')==='xba'&&'aba'.replace(/z/,'x')==='aba'",
        "let log='',n=0,o={flags:'g',exec(){log+='e';return n++<2?{0:'a',length:1,index:n-1}:null}};RegExp.prototype[Symbol.replace].call(o,'aa',()=>{log+='r';return 'x'})==='xx'&&log==='eeerr'",
        "let log='',n=0,result={get 0(){log+='m';return 'a'},get length(){log+='l';return 2},get index(){log+='i';return 0},get 1(){log+='c';return {toString(){log+='t';return 'C'}}},get groups(){log+='g';return undefined}},o={get flags(){log+='f';return 'g'},exec(){log+='e';return n++===0?result:null}};let s=RegExp.prototype[Symbol.replace].call(o,{toString(){log+='s';return 'a'}},{toString(){log+='r';return '$1'}});s==='C'&&log==='srfemelmic tg'.replace(' ','')",
        "let n=0,result={0:'a',length:1,index:0},o={flags:'g',exec(){if(n++===0)return result;result[0]='b';return null}};RegExp.prototype[Symbol.replace].call(o,'a','$&')==='b'",
        "let n=0,result={0:'a',length:1,index:0},o={flags:'g',exec(){if(n++<2){result.index=n-1;return result}return null}};RegExp.prototype[Symbol.replace].call(o,'aa','$&')==='aa'",
        "let n=0,o={flags:'',exec(){n++;return {0:'a',length:1,index:0}}};RegExp.prototype[Symbol.replace].call(o,'aa','x')==='xa'&&n===1",
        "let n=0,r=/a/g;r.exec=function(){n++;this.exec=()=>null;return {0:'a',length:1,index:0}};'aa'.replace(r,'x')==='xa'&&n===1",
        "let r=/a/g;'aa'.replace(r,()=>{r.lastIndex=7;return 'x'})==='xx'&&r.lastIndex===7",
        "let o={flags:'g',exec(){return null},lastIndex:7};RegExp.prototype[Symbol.replace].call(o,'a','x')==='a'&&o.lastIndex===0",
        "let o={get flags(){throw 7}};let log='';try{RegExp.prototype[Symbol.replace].call(o,{toString(){log+='s';return 'a'}},{toString(){log+='r';return 'x'}})}catch(e){}log==='sr'",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn substitution_numeric_named_prefix_suffix_and_custom_match_strings() {
    for source in [
        "'abc'.replace('b',\"$$-$&-$`-$'-$0-$1-$01\")==='a$-b-a-c-$0-$1-$01c'",
        "'ab'.replace(/(a)(b)/,'$2$1-$01-$02-$03-$10-$20-$00-$0')==='ba-a-b-$03-a0-b0-$00-$0'",
        "'b'.replace(/(a)?b/,'[$1]')==='[]'",
        "let o={flags:'',exec(){return {0:'X',index:1,length:3,1:'A',2:undefined,groups:{x:7,y:undefined}}}};RegExp.prototype[Symbol.replace].call(o,'abc','$&:$1:$2:$<x>:$<y>:$<z>')==='aX:A::7::c'",
        "let n=0,o={flags:'',exec(){return {0:'a',index:0,length:1,groups:{get x(){return ++n}}}}};RegExp.prototype[Symbol.replace].call(o,'a','$<x>$<x>')==='12'&&n===2",
        "let o={flags:'',exec(){return {0:'a',index:0,length:2,1:'C'}}};RegExp.prototype[Symbol.replace].call(o,'a','$<x>-$<$1-$99')==='$<x>-$<C-$99'",
        "let o={flags:'',exec(){return {0:'a',index:0,length:2,1:'C',groups:{}}}};RegExp.prototype[Symbol.replace].call(o,'a','$<$1-$<x')==='$<C-$<x'",
        "let o={flags:'',exec(){return {0:'abcde',index:1,length:1}}};RegExp.prototype[Symbol.replace].call(o,'ab',\"$`|$&|$'\")==='aa|abcde|'",
        "let o={flags:'',exec(){return {0:'X',index:Infinity,length:1}}};RegExp.prototype[Symbol.replace].call(o,'ab','$&')==='abX'",
        "let o={flags:'',exec(){return {0:'X',index:-Infinity,length:1}}};RegExp.prototype[Symbol.replace].call(o,'ab','$&')==='Xb'",
        "let o={flags:'',exec(){return {0:'X',index:NaN,length:1}}};RegExp.prototype[Symbol.replace].call(o,'ab','$&')==='Xb'",
        "let o={flags:'',exec(){return {0:'X',index:1.9,length:1}}};RegExp.prototype[Symbol.replace].call(o,'abc','$&')==='aXc'",
        "let o={flags:'',exec(){return {0:'a',index:0,length:1,groups:'xy'}}};RegExp.prototype[Symbol.replace].call(o,'a','$<0>$<1>')==='xy'",
        "let o={flags:'',exec(){return {0:'a',index:0,length:3,1:{toString(){return 'A'}},2:null}}};RegExp.prototype[Symbol.replace].call(o,'a','$1$2')==='Anull'",
        "let o={flags:'',exec(){return {0:'a',index:0,length:1,groups:{'':'E','$1':'T'}}}};RegExp.prototype[Symbol.replace].call(o,'a','$<>-$<$1>')==='E-T'",
        "let named=Object.create({x:'P'}),o={flags:'',exec(){return {0:'a',index:0,length:1,groups:named}}};RegExp.prototype[Symbol.replace].call(o,'a','$<x>')==='P'",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn functional_replace_arguments_and_overlapping_results_still_run_hooks() {
    for source in [
        "let called=false,groups={},o={flags:'',exec(){return {0:'X',index:1.9,length:3,1:7,groups}}};RegExp.prototype[Symbol.replace].call(o,'abc',function(m,a,b,p,s,g){'use strict';called=this===undefined&&m==='X'&&a==='7'&&b===undefined&&p===1&&s==='abc'&&g===groups&&arguments.length===6;return 'R'})==='aRc'&&called",
        "let seen,o={flags:'',exec(){return {0:'a',index:0,length:0,groups:null}}};RegExp.prototype[Symbol.replace].call(o,'a',function(){seen=arguments;return 'R'})==='R'&&seen.length===4&&seen[3]===null",
        "let results=[{0:'bc',index:1,length:1},{0:'a',index:0,length:1}],n=0,calls=0,o={flags:'g',exec(){return n<2?results[n++]:null}};RegExp.prototype[Symbol.replace].call(o,'abcd',()=>{calls++;return 'R'})==='aRd'&&calls===2",
        "let results=[{0:'abcd',index:0,length:1},{0:'x',index:1,length:1}],n=0,calls=0,o={flags:'g',exec(){return n<2?results[n++]:null}};RegExp.prototype[Symbol.replace].call(o,'ab',()=>{calls++;return 'R'})==='R'&&calls===2",
        "let result={0:'a',index:0,length:1},n=0,o={flags:'g',exec(){return n++<2?result:null}},calls=0;RegExp.prototype[Symbol.replace].call(o,'ab',()=>{calls++;result[0]='';result.index=1;return 'R'})==='RRb'&&calls===2",
        "let o={flags:'',exec(){return {0:'a',index:0,length:1,groups:7}}};RegExp.prototype[Symbol.replace].call(o,'a',(m,p,s,g)=>g) === '7'",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn replace_abrupt_hooks_leave_side_effects_and_do_not_start_second_phase() {
    for source in [
        "let e={},n=0,o={flags:'g',exec(){if(n++===0)return {0:'a',index:0,length:1};throw e}},calls=0,ok=false;try{RegExp.prototype[Symbol.replace].call(o,'a',()=>calls++)}catch(x){ok=x===e}ok&&calls===0",
        "let e={},n=0,o={flags:'g',exec(){return {get 0(){throw e}}}},ok=false;try{RegExp.prototype[Symbol.replace].call(o,'a',()=>n++)}catch(x){ok=x===e}ok&&n===0",
        "let e={},o={flags:'',exec(){return {get length(){throw e},get 0(){throw 7}}}},ok=false;try{RegExp.prototype[Symbol.replace].call(o,'a','x')}catch(x){ok=x===e}ok",
        "let e={},o={flags:'',exec(){return {length:1,0:'a',get index(){throw e}}}},ok=false;try{RegExp.prototype[Symbol.replace].call(o,'a','x')}catch(x){ok=x===e}ok",
        "let e={},o={flags:'',exec(){return {length:2,0:'a',index:0,get 1(){throw e}}}},ok=false;try{RegExp.prototype[Symbol.replace].call(o,'a','x')}catch(x){ok=x===e}ok",
        "let e={},o={flags:'',exec(){return {length:1,0:'a',index:0,get groups(){throw e}}}},ok=false;try{RegExp.prototype[Symbol.replace].call(o,'a','x')}catch(x){ok=x===e}ok",
        "let e={},o={flags:'',exec(){return {length:1,0:'a',index:0,groups:{get x(){throw e}}}}},ok=false;try{RegExp.prototype[Symbol.replace].call(o,'a','$<x>')}catch(x){ok=x===e}ok",
        "let e={},o={flags:'',exec(){return {length:1,0:'a',index:0,groups:{x:{toString(){throw e}}}}}},ok=false;try{RegExp.prototype[Symbol.replace].call(o,'a','$<x>')}catch(x){ok=x===e}ok",
        "let e={},o={flags:'g',exec(){this.lastIndex={valueOf(){throw e}};return {0:''}}},ok=false;try{RegExp.prototype[Symbol.replace].call(o,'a','')}catch(x){ok=x===e}ok",
        "let e={},o={get flags(){throw e}},ok=false;try{RegExp.prototype[Symbol.replace].call(o,'a','')}catch(x){ok=x===e}ok",
        "let e={},o={flags:'',get exec(){throw e}},ok=false;try{RegExp.prototype[Symbol.replace].call(o,'a','')}catch(x){ok=x===e}ok",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "RegExp.prototype[Symbol.replace].call(null,'','x')",
        "RegExp.prototype[Symbol.replace].call({},'','x')",
        "let o={flags:'',exec(){return 7}};RegExp.prototype[Symbol.replace].call(o,'a','x')",
        "let o={flags:'',exec(){return {length:1,0:'a',index:0,groups:null}}};RegExp.prototype[Symbol.replace].call(o,'a','x')",
        "let o={flags:'',exec(){return {length:2,0:'a',index:0,1:Symbol()}}};RegExp.prototype[Symbol.replace].call(o,'a','x')",
        "RegExp.prototype[Symbol.replace].call(/a/,'a',()=>Symbol())",
        "Object.freeze(/a/g)[Symbol.replace]('a','x')",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn replace_gc_unicode_custom_progress_metadata_and_limits() -> Result<(), Error> {
    let limits = Limits {
        heap_entries: 180,
        ..Limits::default()
    };
    for source in [
        "let n=0,o={flags:'g',exec(){if(n++<10)return {0:'a',length:2,index:n-1,1:{n}};return null}};RegExp.prototype[Symbol.replace].call(o,'aaaaaaaaaa',(m,c)=>{for(let i=0;i<300;i++){let g={}}return 'x'})==='xxxxxxxxxx'",
        "let result={0:'a',length:3,index:0,1:{toString(){return 'C'}},get 2(){delete this[1];for(let i=0;i<300;i++){let g={}}return undefined}},o={flags:'',exec(){return result}};RegExp.prototype[Symbol.replace].call(o,'a','$1$2')==='C'",
        "let o={flags:'',exec(){return {0:'a',index:0,length:1,groups:{get x(){for(let i=0;i<300;i++){let g={}}return {toString(){return 'X'}}}}}}};RegExp.prototype[Symbol.replace].call(o,'a','$<x>')==='X'",
        "let n=0,seen=[],o={flags:'gu',exec(){seen.push(this.lastIndex);if(this.lastIndex>2)return null;return {0:'',length:1,index:this.lastIndex}}};RegExp.prototype[Symbol.replace].call(o,'😀','x')==='x😀x'&&seen.join()==='0,2,3'",
        "let n=0,seen=[],o={flags:'gv',exec(){seen.push(this.lastIndex);if(this.lastIndex>2)return null;return {0:'',length:1,index:this.lastIndex}}};RegExp.prototype[Symbol.replace].call(o,'😀','x')==='x😀x'&&seen.join()==='0,2,3'",
        "let f=RegExp.prototype[Symbol.replace],d=Object.getOwnPropertyDescriptor(RegExp.prototype,Symbol.replace);f.length===2&&f.name==='[Symbol.replace]'&&f.prototype===undefined&&d.writable&&!d.enumerable&&d.configurable&&String.prototype.replace.length===2",
        "let f=RegExp.prototype[Symbol.replace];delete f.name;f.extra=7;delete RegExp.prototype[Symbol.replace];f.call(/a/,'a','x')==='x'&&f.extra===7&&!Object.hasOwn(f,'name')",
    ] {
        assert_eq!(
            Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
            Value::Boolean(true),
            "{source}"
        );
    }
    for source in [
        "new String.prototype.replace()",
        "new RegExp.prototype[Symbol.replace]()",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })));
    }
    for (source, limits) in [
        (
            "let o={flags:'g',exec(){return {0:'a'}}};RegExp.prototype[Symbol.replace].call(o,'','')",
            Limits {
                fuel: 500,
                ..Limits::default()
            },
        ),
        (
            "let o={flags:'g',exec(){return {0:''}}};RegExp.prototype[Symbol.replace].call(o,'','')",
            Limits {
                fuel: 500,
                ..Limits::default()
            },
        ),
        (
            "let o={flags:'',exec(){return {0:'a',index:0,length:Infinity}}};RegExp.prototype[Symbol.replace].call(o,'a','')",
            Limits {
                properties: 64,
                ..Limits::default()
            },
        ),
        (
            "'aaaaaaaa'.replace(/a/g,'xxxxxxxx')",
            Limits {
                string_units: 16,
                ..Limits::default()
            },
        ),
    ] {
        assert!(
            matches!(
                Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost),
                Err(Error::Limit { .. })
            ),
            "{source}"
        );
    }
    let limits = Limits {
        properties: 64,
        ..Limits::default()
    };
    let source = "let result={0:'a'},o={flags:'g',exec(){return result}};RegExp.prototype[Symbol.replace].call(o,'a','')";
    assert!(matches!(
        Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost),
        Err(Error::Limit {
            resource: "replace matches"
        })
    ));
    let limits = Limits {
        stack: 32,
        ..Limits::default()
    };
    let source = "let o={flags:'',exec(){return {0:'a',index:0,length:32}}};RegExp.prototype[Symbol.replace].call(o,'a',()=>1)";
    assert!(matches!(
        Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost),
        Err(Error::Limit {
            resource: "replacement arguments"
        })
    ));
    let limits = Limits {
        fuel: 30_000,
        ..Limits::default()
    };
    let source = "let template='$<'.repeat(1000),o={flags:'',exec(){return {0:'a',index:0,length:1,groups:{}}}};RegExp.prototype[Symbol.replace].call(o,'a',template)===template";
    assert_eq!(
        Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
        Value::Boolean(true)
    );
    Ok(())
}
