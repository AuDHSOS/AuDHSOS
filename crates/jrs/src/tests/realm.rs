// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use crate::{Error, Host, Limits, Realm, SilentHost, Value};

#[test]
fn realm_scripts_share_lexicals_globals_and_closures() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut realm = Realm::new(Limits::default(), &mut host)?;
    realm.evaluate("let x=1;const y=2;var z=3;function f(){return x+y+z}")?;
    assert_eq!(realm.evaluate("f()")?, Value::Number(6.0));
    assert_eq!(
        realm.evaluate("x=4;globalThis.z=5;f()")?,
        Value::Number(11.0)
    );
    assert_eq!(realm.evaluate("var z;z")?, Value::Number(5.0));
    assert_eq!(
        realm.evaluate("globalThis.x===undefined&&globalThis.y===undefined&&globalThis.z===5")?,
        Value::Boolean(true)
    );
    realm.evaluate("function f(){return 7}")?;
    assert_eq!(realm.evaluate("f()")?, Value::Number(7.0));
    assert!(matches!(realm.evaluate("y=3"), Err(Error::Type { .. })));
    assert_eq!(realm.evaluate("y")?, Value::Number(2.0));
    Ok(())
}

#[test]
fn cross_script_conflicts_are_checked_before_any_script_effects() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut realm = Realm::new(Limits::default(), &mut host)?;
    realm.evaluate("let x=1;var y=2;globalThis.z=3;")?;
    for source in [
        "var x",
        "let x",
        "function x(){}",
        "let y",
        "let Infinity",
        "let added=3;let x=2",
    ] {
        assert!(
            matches!(realm.evaluate(source), Err(Error::Syntax { .. })),
            "{source}"
        );
    }
    assert_eq!(realm.evaluate("typeof added")?, Value::string("undefined"));
    realm.evaluate("let z=4")?;
    assert_eq!(realm.evaluate("z+globalThis.z")?, Value::Number(7.0));
    assert!(matches!(
        realm.evaluate("throw 7;let uninit=3"),
        Err(Error::Thrown { .. })
    ));
    assert!(matches!(
        realm.evaluate("typeof uninit"),
        Err(Error::Reference { .. })
    ));
    assert!(matches!(
        realm.evaluate("let uninit=4"),
        Err(Error::Syntax { .. })
    ));
    Ok(())
}

#[test]
fn separate_script_parsing_prevents_hoisting_across_boundaries() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut realm = Realm::new(Limits::default(), &mut host)?;
    assert!(matches!(
        realm.evaluate("later()"),
        Err(Error::Reference { .. })
    ));
    realm.evaluate("function later(){return 9}")?;
    assert_eq!(realm.evaluate("later()")?, Value::Number(9.0));
    realm.evaluate("var x=7")?;
    assert!(realm.evaluate("let = ;").is_err());
    assert_eq!(realm.evaluate("x")?, Value::Number(7.0));
    realm.evaluate("'use strict';")?;
    assert_eq!(
        realm.evaluate("implicit=4;globalThis.implicit")?,
        Value::Number(4.0)
    );
    assert!(matches!(
        realm.evaluate("'use strict';missing=3"),
        Err(Error::Reference { .. })
    ));
    Ok(())
}

#[test]
fn realm_jobs_checkpoint_and_pending_async_state_cross_scripts() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut realm = Realm::new(Limits::default(), &mut host)?;
    realm.evaluate("let log='';Promise.resolve().then(()=>log+='a');log+='s';")?;
    assert_eq!(realm.evaluate("log")?, Value::string("sa"));
    realm.evaluate(
        "let r=Promise.withResolvers();async function f(){let x=await r.promise;log+=x}f();",
    )?;
    realm.evaluate("r.resolve('b');")?;
    assert_eq!(realm.evaluate("log")?, Value::string("sab"));
    assert!(matches!(
        realm.evaluate("Promise.resolve().then(()=>log+='e');throw 1;"),
        Err(Error::Thrown { .. })
    ));
    assert_eq!(realm.evaluate("log")?, Value::string("sabe"));
    Ok(())
}

#[test]
fn realm_globals_observe_properties_and_lexical_shadowing() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut realm = Realm::new(Limits::default(), &mut host)?;
    realm.evaluate("let old=Number;Number=()=>7;")?;
    assert_eq!(realm.evaluate("Number('3')")?, Value::Number(7.0));
    realm.evaluate("Number=old;")?;
    assert_eq!(realm.evaluate("Number('3')")?, Value::Number(3.0));
    realm.evaluate(
        "Object.defineProperty(globalThis,'access',{get(){return 8},configurable:true});",
    )?;
    assert_eq!(realm.evaluate("access")?, Value::Number(8.0));
    realm.evaluate("var access;")?;
    assert_eq!(realm.evaluate("access")?, Value::Number(8.0));
    realm.evaluate("function access(){return 9}")?;
    assert_eq!(realm.evaluate("access()")?, Value::Number(9.0));
    assert_eq!(
        realm.evaluate("let sum=0;for(var v of [1,2])sum+=v;for(var k in {a:1})sum++;sum")?,
        Value::Number(4.0)
    );
    assert_eq!(realm.evaluate("v+k")?, Value::string("2a"));
    Ok(())
}

#[test]
fn realm_gc_roots_include_prior_scripts_and_returned_identities() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut realm = Realm::new(
        Limits {
            heap_entries: 130,
            ..Limits::default()
        },
        &mut host,
    )?;
    let value = realm.evaluate("let obj={x:7};obj")?;
    realm.evaluate("function f(){return obj.x};for(let i=0;i<300;i++){let t={}}")?;
    assert_eq!(realm.evaluate("obj")?, value);
    assert_eq!(realm.evaluate("f()")?, Value::Number(7.0));
    realm.evaluate("let s=Symbol.for('x');class A{m(){return 9}}let a=new A();")?;
    assert_eq!(
        realm.evaluate("for(let i=0;i<300;i++){let t={}}s===Symbol.for('x')&&a.m()===9")?,
        Value::Boolean(true)
    );
    Ok(())
}

#[test]
fn realm_resource_failures_poison_but_language_exceptions_do_not() -> Result<(), Error> {
    struct Broken;
    impl Host for Broken {
        fn print(&mut self, _: &[Value]) -> Result<(), Error> {
            Err(Error::Host)
        }
    }
    let mut host = SilentHost;
    let mut realm = Realm::new(
        Limits {
            fuel: 1000,
            ..Limits::default()
        },
        &mut host,
    )?;
    assert!(matches!(
        realm.evaluate("while(true){}"),
        Err(Error::Limit { .. })
    ));
    assert!(matches!(
        realm.evaluate("1"),
        Err(Error::Limit {
            resource: "realm is poisoned"
        })
    ));
    let mut broken = Broken;
    let mut realm = Realm::new(Limits::default(), &mut broken)?;
    assert_eq!(realm.evaluate("print(1)"), Err(Error::Host));
    assert!(realm.evaluate("1").is_err());
    Ok(())
}

#[test]
fn realm_deleted_globals_redeclarations_and_property_restrictions() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut realm = Realm::new(Limits::default(), &mut host)?;
    realm.evaluate("implicit=3;var declared=4;let lexical=5;")?;
    assert_eq!(
        realm.evaluate("delete implicit&&!delete declared&&!delete lexical&&delete unknown")?,
        Value::Boolean(true)
    );
    assert_eq!(
        realm.evaluate("typeof implicit")?,
        Value::string("undefined")
    );
    realm.evaluate("Object.defineProperty(globalThis,'restricted',{value:1});")?;
    assert!(matches!(
        realm.evaluate("function restricted(){}"),
        Err(Error::Type { .. })
    ));
    realm.evaluate("var restricted;")?;
    assert_eq!(realm.evaluate("restricted")?, Value::Number(1.0));
    realm.evaluate("Object.preventExtensions(globalThis)")?;
    assert!(matches!(
        realm.evaluate("var newName"),
        Err(Error::Type { .. })
    ));
    assert_eq!(realm.evaluate("let hidden=8;hidden")?, Value::Number(8.0));
    assert!(matches!(
        realm.evaluate("let hidden=9;"),
        Err(Error::Syntax { .. })
    ));
    assert_eq!(realm.evaluate("declared")?, Value::Number(4.0));
    Ok(())
}

#[test]
fn realm_closures_see_later_lexicals_and_preserve_local_environments() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut realm = Realm::new(Limits::default(), &mut host)?;
    realm.evaluate("function f(){return later}function make(x){return ()=>++x}let inc=make(3)")?;
    assert!(matches!(
        realm.evaluate("f()"),
        Err(Error::Reference { .. })
    ));
    realm.evaluate("let later=7")?;
    assert_eq!(realm.evaluate("f()+inc()+inc()")?, Value::Number(16.0));
    realm.evaluate("let old=f")?;
    realm.evaluate("function f(){return 9}")?;
    assert_eq!(realm.evaluate("old()+f()")?, Value::Number(16.0));
    assert_eq!(
        realm.evaluate("let total=0;for(var [a,b] of [[2,3]])total+=a+b;total")?,
        Value::Number(5.0)
    );
    assert_eq!(realm.evaluate("a+b")?, Value::Number(5.0));
    Ok(())
}

#[test]
fn realm_binding_and_retained_value_quotas_are_cumulative() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut realm = Realm::new(
        Limits {
            binding_slots: 2,
            ..Limits::default()
        },
        &mut host,
    )?;
    realm.evaluate("let a=1")?;
    realm.evaluate("let b=2")?;
    assert!(matches!(
        realm.evaluate("let c=3"),
        Err(Error::Limit { .. })
    ));
    let mut host = SilentHost;
    let mut realm = Realm::new(
        Limits {
            source_bytes: 16,
            ..Limits::default()
        },
        &mut host,
    )?;
    assert!(matches!(
        realm.evaluate("1+2+3+4+5+6+7+8+9"),
        Err(Error::Limit { .. })
    ));
    assert!(matches!(
        realm.evaluate("1"),
        Err(Error::Limit {
            resource: "realm is poisoned"
        })
    ));
    Ok(())
}

#[test]
fn realm_unhandled_rejection_results_do_not_destroy_later_script_state() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut realm = Realm::new(Limits::default(), &mut host)?;
    realm.evaluate("let reason={x:7};let value=3")?;
    let Err(Error::Thrown { value: reason }) = realm.evaluate("Promise.reject(reason);undefined")
    else {
        return Err(Error::InvalidBytecode);
    };
    assert_eq!(realm.evaluate("reason")?, reason);
    assert_eq!(realm.evaluate("value")?, Value::Number(3.0));
    Ok(())
}
