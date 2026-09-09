// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use crate::{Error, Host, Limits, Realm, SilentHost, Value};

fn realm(host: &mut impl Host, limits: Limits) -> Result<Realm<'_>, Error> {
    let mut r = Realm::new(limits, host)?;
    let f = r.eval_script_function()?;
    r.set_global("evalScript", &f)?;
    let gc = r.gc_function()?;
    r.set_global("gc", &gc)?;
    Ok(r)
}
#[test]
fn nested_scripts_have_global_scope_and_return_script_completions() -> Result<(), Error> {
    for source in [
        "evalScript('1+2')===3&&evalScript('')===undefined&&evalScript('var x')===undefined",
        "let x=1;function f(){let x=2;return evalScript('x')}f()===1",
        "function f(){let x=2;evalScript('var x=7;let y=9');return x}f()===2&&x===7&&y===9",
        "function f(){'use strict';return evalScript('this')}f()===globalThis",
        "let o={m(){return evalScript('this')}};o.m()===globalThis",
        "let o={m(){return evalScript('()=>this')}};o.m()()===globalThis",
        "evalScript('var n=7;function f(){return n}');f()===7&&Object.getOwnPropertyDescriptor(globalThis,'n').configurable===false",
        "evalScript('function f(){return y};let y=3');evalScript('y=4');f()===4",
        "let read=evalScript('let x=7;()=>x');gc();read()===7",
        "let f=evalScript;f.call({x:1},'2+3')===5",
        "evalScript({toString(){return '6+1'}})===7&&evalScript(7)===7",
        "let n=0;evalScript({toString(){n++;return '1'}});n===1",
        "evalScript('if(true){3}');evalScript('try{7}finally{}')===7",
        "evalScript('var x=3;');evalScript('var x;');x===3",
        "evalScript('var x=3;function f(){return 1}function f(){return 2}');f()===2",
    ] {
        let mut host = SilentHost;
        let mut r = realm(&mut host, Limits::default())?;
        assert_eq!(r.evaluate(source)?, Value::Boolean(true), "{source}");
    }
    Ok(())
}
#[test]
fn nested_errors_unwind_once_and_keep_prior_script_effects() -> Result<(), Error> {
    for source in [
        "let log='';try{evalScript('throw 7')}catch(e){log+=e}finally{log+='f'}log==='7f'",
        "let log='';try{evalScript(\"try{throw 7}finally{log+='i'}\")}catch(e){log+=e}finally{log+='o'}log==='i7o'",
        "let log='';function f(){try{return evalScript('throw 7')}catch(e){log+=e;return 3}finally{log+='f'}}f()===3&&log==='7f'",
        "let e={x:7},caught;try{evalScript('throw e')}catch(v){caught=v}caught===e",
        "let yes=false;try{evalScript('var notCreated; let =')}catch(e){yes=e instanceof SyntaxError}yes&&typeof notCreated==='undefined'",
        "let x=7,yes=false;try{evalScript('var notCreated;let x')}catch(e){yes=e instanceof SyntaxError}yes&&typeof notCreated==='undefined'&&x===7",
        "let yes=false;try{evalScript('var published=7;throw 9;let pending')}catch(e){yes=e===9}let tdz=false;try{pending}catch(e){tdz=e instanceof ReferenceError}yes&&published===7&&tdz",
        "let yes=false;try{evalScript('return 7')}catch(e){yes=e instanceof SyntaxError}yes",
        "let yes=false;try{evalScript('()=>new.target')}catch(e){yes=e instanceof SyntaxError}yes",
        "function C(){let yes=false;try{evalScript('new.target')}catch(e){yes=e instanceof SyntaxError}this.yes=yes}new C().yes",
        "evalScript('function C(){return (()=>new.target)()}new C()')===C",
        "let yes=false;try{evalScript(Symbol())}catch(e){yes=e instanceof TypeError}yes",
        "let yes=false;try{evalScript({toString(){throw 7}})}catch(e){yes=e===7}yes",
        "let yes=false;Object.defineProperty(globalThis,'blocked',{value:1});Object.defineProperty(globalThis,'restricted',{value:2});try{evalScript('function blocked(){}let restricted')}catch(e){yes=e instanceof SyntaxError}yes",
        "let yes=false;let lexical;Object.preventExtensions(globalThis);try{evalScript('var fresh;var lexical')}catch(e){yes=e instanceof SyntaxError}yes",
        "for(let i=0;i<100;i++){try{evalScript('throw 7')}catch(e){if(e!==7)throw 9}}evalScript('3')===3",
    ] {
        let mut host = SilentHost;
        let mut r = realm(&mut host, Limits::default())?;
        assert_eq!(r.evaluate(source)?, Value::Boolean(true), "{source}");
    }
    Ok(())
}
#[test]
fn nested_scripts_preserve_stack_roots_and_do_not_drain_jobs_early() -> Result<(), Error> {
    for source in [
        "let log='';evalScript(\"Promise.resolve().then(()=>log+='j');log+='i'\");log+='o';if(log!=='io')throw 7;Promise.resolve().then(()=>{if(log!=='ioj')throw 8});true",
        "function f(){let local={x:7};let r=evalScript('gc();({y:3})');gc();return local.x+r.y}f()===10",
        "function f(){let local={x:7};return local.x+evalScript('gc();3')}f()===10",
        "let f=evalScript('(()=>{let v={x:7};return ()=>v})()');for(let i=0;i<300;i++){let o={}}f().x===7",
        "let result=0;evalScript('async function f(){let x={n:7};await 0;gc();return x}f().then(x=>result=x.n)');Promise.resolve().then(()=>{});true",
        "let t={x:3,m(){return this.x+evalScript('gc();7')}};t.m()===10",
        "let a={x:1};let b={get x(){return evalScript('gc();2')}};function sum(x,y,z){return x.x+y+z.x}sum(a,b.x,evalScript('gc();({x:3})'))===6",
        "let text='evalScript(\"gc();7\")';evalScript(text)===7",
        "let e=evalScript('(()=>{let x={n:7};gc();return x})()');evalScript('gc()');e.n===7",
        "let n=0;function f(){let arr=[1];try{for(let x of arr){evalScript('throw 7')}}catch(e){n=e}return arr[0]}f()===1&&n===7",
    ] {
        let mut host = SilentHost;
        let mut r = realm(
            &mut host,
            Limits {
                heap_entries: 130,
                ..Limits::default()
            },
        )?;
        assert_eq!(r.evaluate(source)?, Value::Boolean(true), "{source}");
    }
    let mut host = SilentHost;
    let mut r = realm(&mut host, Limits::default())?;
    r.evaluate(
        "let result=0;evalScript('async function f(){await 0;return 7}f().then(x=>result=x)')",
    )?;
    assert_eq!(r.evaluate("result")?, Value::Number(7.0));
    Ok(())
}
#[test]
fn recursive_eval_and_combined_frame_binding_budgets_are_bounded() -> Result<(), Error> {
    for (source, limits) in [
        (
            "evalScript('while(true){}')",
            Limits {
                fuel: 1000,
                ..Limits::default()
            },
        ),
        (
            "var code='evalScript(code)';evalScript(code)",
            Limits::default(),
        ),
        (
            "var code='evalScript(code)';evalScript(code)",
            Limits {
                call_frames: 2,
                ..Limits::default()
            },
        ),
        (
            "function f(){let a,b,c,d;return evalScript('{let a,b,c,d;1}')}f()",
            Limits {
                binding_slots: 6,
                ..Limits::default()
            },
        ),
    ] {
        let mut host = SilentHost;
        let mut r = realm(&mut host, limits)?;
        assert!(
            matches!(r.evaluate(source), Err(Error::Limit { .. })),
            "{source}"
        );
        assert!(r.evaluate("1").is_err());
    }
    let mut host = SilentHost;
    let mut r = realm(&mut host, Limits::default())?;
    assert!(matches!(
        r.evaluate(r"evalScript('\ud800')"),
        Err(Error::Unsupported { .. })
    ));
    Ok(())
}

#[test]
fn nested_source_failure_host_errors_and_binding_publication() -> Result<(), Error> {
    struct Fail;
    impl Host for Fail {
        fn print(&mut self, _: &[Value]) -> Result<(), Error> {
            Err(Error::Host)
        }
    }
    let mut host = Fail;
    let mut r = realm(&mut host, Limits::default())?;
    assert_eq!(
        r.evaluate("let caught=false;try{evalScript('print(7)')}catch(e){caught=true}"),
        Err(Error::Host)
    );
    assert!(r.evaluate("1").is_err());
    let mut host = SilentHost;
    let mut r = realm(
        &mut host,
        Limits {
            source_bytes: 64,
            ..Limits::default()
        },
    )?;
    let f = r.get_global("evalScript")?;
    assert!(matches!(
        r.call(&f, &Value::Null, &[Value::string(&" ".repeat(65))]),
        Err(Error::Limit { .. })
    ));
    for source in [
        "let n=0;let yes=false;try{evalScript('n=1;var a;let a')}catch(e){yes=e instanceof SyntaxError}yes&&n===0&&typeof a==='undefined'",
        "let e={x:7},yes=false;try{evalScript('gc();throw e')}catch(v){gc();yes=v===e}yes",
        "let saved=evalScript('function f(){return ()=>7}f()');evalScript('function f(){return 2}');saved()===7&&f()===2",
        "let count=0;let o={toString(){return evalScript(\"count++;'7'\")}};evalScript(o)===7&&count===1",
    ] {
        let mut host = SilentHost;
        let mut r = realm(&mut host, Limits::default())?;
        assert_eq!(r.evaluate(source)?, Value::Boolean(true), "{source}");
    }
    Ok(())
}

#[test]
fn new_target_arrows_require_enclosing_non_arrow_function() {
    for source in [
        "()=>new.target",
        "async()=>new.target",
        "(x=new.target)=>x",
        "()=>()=>new.target",
        "({[new.target](){}})",
    ] {
        assert!(
            matches!(
                crate::compile(source, Limits::default()),
                Err(Error::Syntax { .. })
            ),
            "{source}"
        );
    }
    for source in [
        "function f(x=()=>new.target){return x()}new f()===f",
        "function f(){return (()=>()=>new.target)()()}new f()===f",
        "let o={m(x=new.target){return x}};o.m()===undefined",
    ] {
        assert_eq!(super::eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}
