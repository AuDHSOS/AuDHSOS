// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use super::{Error, Limits, Runtime, SilentHost, Value, compile, eval};

#[test]
fn string_dispatch_preserves_raw_receiver_and_order() {
    for method in ["match", "search"] {
        for text in [
            "let raw={toString(){throw 7}},p={[Symbol.METHOD](v){return v===raw&&this===p}};String.prototype.METHOD.call(raw,p)",
            "let n=0,p={get [Symbol.METHOD](){n++;throw 7}};try{String.prototype.METHOD.call(null,p)}catch(e){}n===0",
            "let log='',p={get [Symbol.METHOD](){log+='h';return undefined},toString(){log+='p';return 'a'}},raw={toString(){log+='s';return 'a'}};String.prototype.METHOD.call(raw,p);log==='hsp'",
            "let p={[Symbol.METHOD]:null,toString(){return 'a'}};String.prototype.METHOD.call('a',p)!==null",
            "let o={},p={[Symbol.METHOD](){return o}};String.prototype.METHOD.call('a',p)===o",
            "let p={[Symbol.METHOD](){return 17}};String.prototype.METHOD.call(Symbol(),p)===17",
            "let log='';Number.prototype[Symbol.METHOD]=()=>{throw 7};Number.prototype.toString=function(){throw 8};String.prototype.METHOD.call({toString(){log+='s';return '42'}},42);log==='s'",
            "let p={[Symbol.METHOD]:undefined,[Symbol.match]:undefined,get constructor(){throw 7},get source(){throw 8},get flags(){throw 9},toString(){return 'a'}};'a'.METHOD(p)!==null",
            "let original=globalThis.RegExp;globalThis.RegExp=function(){throw 7};String.prototype.METHOD.call('a','a')!==null",
            "let e={},ok=false;try{'a'.METHOD({get [Symbol.METHOD](){throw e}})}catch(x){ok=x===e}ok",
        ] {
            let source = text.replace("METHOD", method);
            assert_eq!(eval(&source), Ok(Value::Boolean(true)), "{source}");
        }
        for text in [
            "'a'.METHOD({[Symbol.METHOD]:7})",
            "String.prototype.METHOD.call(undefined,{})",
            "String.prototype.METHOD.call(Symbol(),{})",
            "'a'.METHOD(Symbol())",
        ] {
            let source = text.replace("METHOD", method);
            assert!(matches!(eval(&source), Err(Error::Type { .. })), "{source}");
        }
    }
    for source in [
        "let r=/a/;r[Symbol.match]=undefined;r.toString=()=> 'b';'b'.match(r)[0]==='b'",
        "let old=RegExp.prototype[Symbol.match],seen;RegExp.prototype[Symbol.match]=function(s){seen=this;return s};'abc'.match('a')==='abc'&&seen instanceof RegExp&&seen.source==='a'",
        "let old=RegExp.prototype[Symbol.search],seen;RegExp.prototype[Symbol.search]=function(s){seen=this;return s};'abc'.search('a')==='abc'&&seen instanceof RegExp&&seen.source==='a'",
        "delete RegExp.prototype[Symbol.match];let ok=false;try{'a'.match('a')}catch(e){ok=e instanceof TypeError}ok",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn symbol_match_uses_flags_snapshot_and_dynamic_exec() {
    for source in [
        "'ba'.match(/a/)[0]==='a'&&'ba'.search(/a/)===1&&'ba'.match(/z/)===null",
        "let marker={},o={flags:'',exec(s){return marker}};RegExp.prototype[Symbol.match].call(o,'a')===marker",
        "let log='',o={get flags(){log+='f';return {toString(){log+='t';return ''}}},get exec(){log+='e';return function(s){log+=s;return {}}}};RegExp.prototype[Symbol.match].call(o,{toString(){log+='s';return 'x'}});log==='sftex'",
        "let n=0,o={flags:'g',exec(){if(n++===0){this.flags='';this.exec=()=>null;return {0:{toString(){return 'a'}}}}}};RegExp.prototype[Symbol.match].call(o,'a').join()==='a'&&n===1&&o.lastIndex===0",
        "let n=0,o={flags:'g',exec(){return n++===0?{0:undefined}:null}};RegExp.prototype[Symbol.match].call(o,'a')[0]==='undefined'",
        "let n=0,o={flags:'g',exec(){return n++===0?{get 0(){o.exec=()=>null;return 'a'}}:null}};RegExp.prototype[Symbol.match].call(o,'a')[0]==='a'",
        "let r=/a/g;Object.defineProperty(r,'flags',{value:''});r[Symbol.match]('aa').index===0&&r.lastIndex===1",
        "let o={flags:'g',lastIndex:7,exec(){return null}};RegExp.prototype[Symbol.match].call(o,'')===null&&o.lastIndex===0",
        "let e={},o={get flags(){throw e}},ok=false;try{RegExp.prototype[Symbol.match].call(o,'a')}catch(x){ok=x===e}ok",
        "let e={},o={flags:'g',exec(){return {get 0(){throw e}}}},ok=false;try{RegExp.prototype[Symbol.match].call(o,'a')}catch(x){ok=x===e}ok&&o.lastIndex===0",
        "let e={},o={flags:'g',exec(){return {0:{toString(){throw e}}}}},ok=false;try{RegExp.prototype[Symbol.match].call(o,'a')}catch(x){ok=x===e}ok",
        "let r=/a/;r.exec=7;r[Symbol.match]('a')[0]==='a'",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "RegExp.prototype[Symbol.match].call(null,'a')",
        "RegExp.prototype[Symbol.match].call({},'a')",
        "let o={flags:Symbol()};RegExp.prototype[Symbol.match].call(o,'a')",
        "RegExp.prototype[Symbol.match].call({flags:'g',exec(){return 1}},'a')",
        "RegExp.prototype[Symbol.match].call({flags:'g',exec(){return {0:Symbol()}}},'a')",
        "let o={flags:'g',exec(){return null}};Object.freeze(o);RegExp.prototype[Symbol.match].call(o,'a')",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn empty_global_matches_advance_utf16_and_safe_integer_indices() {
    for source in [
        "'😀'.match(/(?:)/g).length===3",
        "let indices=[],o={flags:'gu',exec(){indices.push(this.lastIndex);return this.lastIndex<=3?{0:''}:null}};RegExp.prototype[Symbol.match].call(o,'😀x').length===3&&indices.join()==='0,2,3,4'",
        "let indices=[],o={flags:'gv',exec(){indices.push(this.lastIndex);return this.lastIndex<=3?{0:''}:null}};RegExp.prototype[Symbol.match].call(o,'😀x').length===3&&indices.join()==='0,2,3,4'",
        "let indices=[],o={flags:'gu',exec(){indices.push(this.lastIndex);return this.lastIndex<=3?{0:''}:null}};RegExp.prototype[Symbol.match].call(o,'\\ud800x\\udc00').length===4&&indices.join()==='0,1,2,3,4'",
        "let n=0,o={flags:'g',exec(){if(n++===0){this.lastIndex=Infinity;return {0:''}}return null}};RegExp.prototype[Symbol.match].call(o,'').length===1&&o.lastIndex===9007199254740992",
        "let n=0,o={flags:'gu',exec(){if(n++===0){this.lastIndex={valueOf(){return 0.9}};return {0:''}}return null}};RegExp.prototype[Symbol.match].call(o,'😀').length===1&&o.lastIndex===2",
        "let n=0,o={flags:'g',exec(){if(n++===0){this.lastIndex=-10;return {0:''}}return null}};RegExp.prototype[Symbol.match].call(o,'').length===1&&o.lastIndex===1",
        "let o={flags:'g',exec(){Object.defineProperty(this,'lastIndex',{writable:false});return {0:''}}};let ok=false;try{RegExp.prototype[Symbol.match].call(o,'')}catch(e){ok=e instanceof TypeError}ok",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn symbol_search_restores_samevalue_only_on_normal_completion() {
    for source in [
        "let r=/a/g;r.lastIndex=7;'ba'.search(r)===1&&r.lastIndex===7",
        "let r=/a/y;r.lastIndex=1;'ba'.search(r)===-1&&r.lastIndex===1",
        "let log='',saved={valueOf(){throw 7}},index=saved,result={get index(){log+='i';return saved}},o={get lastIndex(){log+='g';return index},set lastIndex(v){log+=v===saved?'r':'z';index=v},get exec(){log+='e';return function(s){log+='c';return result}}};RegExp.prototype[Symbol.search].call(o,{toString(){log+='s';return 'a'}})===saved&&log==='sgzecgri'&&index===saved",
        "let o={lastIndex:-0,exec(){return null}};RegExp.prototype[Symbol.search].call(o,'a')===-1&&Object.is(o.lastIndex,-0)",
        "let o={lastIndex:NaN,exec(){this.lastIndex=NaN;return null}};RegExp.prototype[Symbol.search].call(o,'a')===-1&&Number.isNaN(o.lastIndex)",
        "let e={},o={lastIndex:7,exec(){this.lastIndex=2;throw e}},ok=false;try{RegExp.prototype[Symbol.search].call(o,'a')}catch(x){ok=x===e}ok&&o.lastIndex===2",
        "let o={lastIndex:7,exec(){this.lastIndex=3;return 1}},ok=false;try{RegExp.prototype[Symbol.search].call(o,'a')}catch(e){ok=e instanceof TypeError}ok&&o.lastIndex===3",
        "let e={},o={lastIndex:7,exec(){this.lastIndex=3;return {get index(){throw e}}}},ok=false;try{RegExp.prototype[Symbol.search].call(o,'a')}catch(x){ok=x===e}ok&&o.lastIndex===7",
        "let o={get lastIndex(){return 0},exec(){return {index:'anything'}}};RegExp.prototype[Symbol.search].call(o,'a')==='anything'",
        "let n=0,o={get lastIndex(){n++;return 0},exec(){return null}};RegExp.prototype[Symbol.search].call(o,'a')===-1&&n===2",
        "let n=0,e={},o={get lastIndex(){if(++n===2)throw e;return 0},exec(){return null}},ok=false;try{RegExp.prototype[Symbol.search].call(o,'')}catch(x){ok=x===e}ok&&n===2",
        "let e={},value=7,n=0,o={get lastIndex(){return value},set lastIndex(v){if(v===7)throw e;value=v},exec(){return {get index(){n++;return 1}}}},ok=false;try{RegExp.prototype[Symbol.search].call(o,'')}catch(x){ok=x===e}ok&&value===0&&n===0",
        "let e={},n=0,o={get lastIndex(){return 7},set lastIndex(v){throw e},exec(){n++;return null}},ok=false;try{RegExp.prototype[Symbol.search].call(o,'')}catch(x){ok=x===e}ok&&n===0",
        "let r=/a/;r.exec=null;r[Symbol.search]('a')===0",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "RegExp.prototype[Symbol.search].call(null,'')",
        "RegExp.prototype[Symbol.search].call({},'')",
        "RegExp.prototype[Symbol.search].call({get lastIndex(){return -0},exec(){return null}},'')",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn match_search_metadata_gc_and_resource_limits() -> Result<(), Error> {
    for name in ["match", "search"] {
        let source = format!(
            "let f=RegExp.prototype[Symbol.{name}],d=Object.getOwnPropertyDescriptor(RegExp.prototype,Symbol.{name});let s=String.prototype.{name};f.name==='[Symbol.{name}]'&&f.length===1&&d.writable&&!d.enumerable&&d.configurable&&f.prototype===undefined&&s.name==='{name}'&&s.length===1&&s.prototype===undefined"
        );
        assert_eq!(eval(&source), Ok(Value::Boolean(true)), "{source}");
        assert!(matches!(
            eval(&format!("new RegExp.prototype[Symbol.{name}]()")),
            Err(Error::Type { .. })
        ));
        assert!(matches!(
            eval(&format!("new String.prototype.{name}()")),
            Err(Error::Type { .. })
        ));
    }
    let limits = Limits {
        heap_entries: 180,
        ..Limits::default()
    };
    for source in [
        "let n=0,o={flags:'g',exec(){for(let i=0;i<100;i++){let g={}}return n++<100?{0:'a'}:null}};RegExp.prototype[Symbol.match].call(o,'').length===100",
        "let original={},index=original,o={get lastIndex(){return index},set lastIndex(v){index=v},exec(){original=null;for(let i=0;i<300;i++){let g={}}return {index:7}}};RegExp.prototype[Symbol.search].call(o,'')===7&&typeof index==='object'",
        "let n=0,o={get lastIndex(){if(n++===1){for(let i=0;i<300;i++){let g={}}}return 0},exec(){return {index:{n:7}}}};RegExp.prototype[Symbol.search].call(o,'').n===7",
        "let p={[Symbol.match](s){for(let i=0;i<300;i++){let g={}}return s.n}},o={n:7};String.prototype.match.call(o,p)===7",
    ] {
        assert_eq!(
            Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
            Value::Boolean(true),
            "{source}"
        );
    }
    for (source, limits) in [
        (
            "let o={flags:'g',exec(){return {0:'a'}}};RegExp.prototype[Symbol.match].call(o,'')",
            Limits {
                fuel: 1000,
                ..Limits::default()
            },
        ),
        (
            "let o={flags:'g',exec(){return {0:''}}};RegExp.prototype[Symbol.match].call(o,'')",
            Limits {
                fuel: 1000,
                ..Limits::default()
            },
        ),
        (
            "'a'.match({[Symbol.match](s){return s.match(this)}})",
            Limits {
                fuel: 1000,
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
    let source = "let o={flags:'g',exec(){return {0:'a'}}};try{RegExp.prototype[Symbol.match].call(o,'')}catch(e){throw 'caught'}";
    assert!(matches!(
        Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost),
        Err(Error::Limit {
            resource: "RegExp match results"
        })
    ));
    Ok(())
}
