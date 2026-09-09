// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use crate::{Error, Host, Limits, Realm, SilentHost, Value};
use std::{cell::RefCell, rc::Rc};

#[derive(Default)]
struct Reports {
    exceptions: Vec<Value>,
    rejections: Vec<Value>,
    output: Vec<Value>,
    fatal: bool,
}
struct ReportingHost(Rc<RefCell<Reports>>);
impl Host for ReportingHost {
    fn print(&mut self, args: &[Value]) -> Result<(), Error> {
        self.0.borrow_mut().output.extend_from_slice(args);
        Ok(())
    }
    fn report_exception(&mut self, reason: &Value) -> Result<(), Error> {
        let mut state = self.0.borrow_mut();
        state.exceptions.push(reason.clone());
        if state.fatal {
            Err(Error::Host)
        } else {
            Ok(())
        }
    }
    fn unhandled_rejection(&mut self, reason: &Value) -> Result<(), Error> {
        self.0.borrow_mut().rejections.push(reason.clone());
        Ok(())
    }
}

#[test]
fn microtasks_are_opt_in_callable_mutable_and_nonconstructible() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut r = Realm::new(Limits::default(), &mut host)?;
    assert_eq!(
        r.evaluate("typeof queueMicrotask")?,
        Value::string("undefined")
    );
    r.install_queue_microtask()?;
    assert_eq!(r.evaluate("queueMicrotask.name==='queueMicrotask'&&queueMicrotask.length===1&&!Object.hasOwn(queueMicrotask,'prototype')")?,Value::Boolean(true));
    assert_eq!(r.evaluate("let d=Object.getOwnPropertyDescriptor(globalThis,'queueMicrotask');d.writable&&d.enumerable&&d.configurable")?,Value::Boolean(true));
    for source in [
        "queueMicrotask()",
        "queueMicrotask(null)",
        "queueMicrotask({handleEvent(){}})",
        "queueMicrotask('print(1)')",
        "new queueMicrotask(()=>{})",
    ] {
        assert!(
            matches!(r.evaluate(source), Err(Error::Type { .. })),
            "{source}"
        );
    }
    r.evaluate("queueMicrotask.extra=7;let previous=queueMicrotask;")?;
    r.install_queue_microtask()?;
    assert_eq!(
        r.evaluate("previous!==queueMicrotask&&previous.extra===7")?,
        Value::Boolean(true)
    );
    r.evaluate(
        "Object.defineProperty(globalThis,'queueMicrotask',{configurable:false,writable:false})",
    )?;
    assert!(matches!(
        r.install_queue_microtask(),
        Err(Error::Type { .. })
    ));
    Ok(())
}

#[test]
fn microtasks_share_fifo_order_and_ignore_arguments_and_return_values() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut r = Realm::new(Limits::default(), &mut host)?;
    r.install_queue_microtask()?;
    assert_eq!(
        r.evaluate("let log='';queueMicrotask(()=>log+='m');log+='s';log")?,
        Value::string("s")
    );
    assert_eq!(r.evaluate("log")?, Value::string("sm"));
    r.evaluate("log='';Promise.resolve().then(()=>{log+='p';queueMicrotask(()=>log+='n')});queueMicrotask(()=>{log+='m';Promise.resolve().then(()=>log+='q')});Promise.reject().catch(()=>log+='r');queueMicrotask(()=>log+='t')")?;
    assert_eq!(r.evaluate("log")?, Value::string("pmrtnq"));
    r.evaluate("let receiver,args;queueMicrotask(function(){'use strict';receiver=this;args=arguments.length},1,2)")?;
    assert_eq!(
        r.evaluate("receiver===undefined&&args===0")?,
        Value::Boolean(true)
    );
    r.evaluate("let touched=0;queueMicrotask(()=>({get then(){touched++;throw 1}}));queueMicrotask(function(){receiver=this});")?;
    assert_eq!(
        r.evaluate("touched===0&&receiver===globalThis")?,
        Value::Boolean(true)
    );
    r.evaluate("log='';queueMicrotask(()=>{log+='a';queueMicrotask(()=>log+='b')});queueMicrotask(()=>log+='c');")?;
    assert_eq!(r.evaluate("log")?, Value::string("acb"));
    Ok(())
}

#[test]
fn reported_exceptions_are_not_rejections_and_do_not_stop_remaining_jobs() -> Result<(), Error> {
    let reports = Rc::new(RefCell::new(Reports::default()));
    let mut host = ReportingHost(reports.clone());
    let mut r = Realm::new(Limits::default(), &mut host)?;
    r.install_queue_microtask()?;
    let reason = r.evaluate("let reason={n:7};reason")?;
    r.evaluate("queueMicrotask(()=>{throw reason});queueMicrotask(()=>print(9));Promise.reject(3);queueMicrotask(async()=>{throw 4})")?;
    assert_eq!(reports.borrow().exceptions, vec![reason]);
    assert_eq!(reports.borrow().output, vec![Value::Number(9.0)]);
    assert_eq!(
        reports.borrow().rejections,
        vec![Value::Number(3.0), Value::Number(4.0)]
    );
    r.evaluate("queueMicrotask(()=>missingVariable)")?;
    let error = reports
        .borrow()
        .exceptions
        .last()
        .cloned()
        .ok_or(Error::InvalidBytecode)?;
    assert_eq!(
        r.get(&error, &Value::string("name"))?,
        Value::string("ReferenceError")
    );
    Ok(())
}

#[test]
fn default_reporting_fails_after_fifo_drain_and_keeps_the_realm_usable() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut r = Realm::new(Limits::default(), &mut host)?;
    r.install_queue_microtask()?;
    assert_eq!(r.evaluate("let log='';queueMicrotask(()=>{throw 7});queueMicrotask(()=>{log+='m';throw 8});Promise.resolve().then(()=>log+='p')"),Err(Error::Thrown{value:Value::Number(7.0)}));
    assert_eq!(r.evaluate("log")?, Value::string("mp"));
    assert_eq!(
        r.evaluate("queueMicrotask(()=>{log+='q';throw 9});throw 3"),
        Err(Error::Thrown {
            value: Value::Number(3.0)
        })
    );
    assert_eq!(r.evaluate("log")?, Value::string("mpq"));
    Ok(())
}

#[test]
fn host_enqueues_wait_for_checkpoint_and_queue_roots_survive_release() -> Result<(), Error> {
    let reports = Rc::new(RefCell::new(Reports::default()));
    let mut host = ReportingHost(reports.clone());
    let mut r = Realm::new(
        Limits {
            heap_entries: 100,
            ..Limits::default()
        },
        &mut host,
    )?;
    let callback = r.evaluate("(()=>{for(let i=0;i<200;i++){let t={}}print(7)})")?;
    r.queue_microtask(&callback)?;
    r.release(&callback)?;
    assert!(reports.borrow().output.is_empty());
    r.checkpoint()?;
    assert_eq!(reports.borrow().output, vec![Value::Number(7.0)]);
    r.checkpoint()?;
    assert_eq!(reports.borrow().output.len(), 1);
    let mut host2 = SilentHost;
    let mut other = Realm::new(Limits::default(), &mut host2)?;
    let foreign = other.evaluate("(()=>1)")?;
    assert!(matches!(
        r.queue_microtask(&foreign),
        Err(Error::Type { .. })
    ));
    assert!(matches!(
        r.queue_microtask(&Value::Null),
        Err(Error::Type { .. })
    ));
    Ok(())
}

#[test]
fn microtask_queue_fuel_and_reporting_failures_are_embedding_limits() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut r = Realm::new(
        Limits {
            jobs: 1,
            ..Limits::default()
        },
        &mut host,
    )?;
    let cb = r.evaluate("(()=>{})")?;
    r.queue_microtask(&cb)?;
    assert!(matches!(r.queue_microtask(&cb), Err(Error::Limit { .. })));
    assert!(r.checkpoint().is_err());
    let mut host = SilentHost;
    let mut r = Realm::new(
        Limits {
            fuel: 1000,
            ..Limits::default()
        },
        &mut host,
    )?;
    r.install_queue_microtask()?;
    assert!(matches!(
        r.evaluate("function f(){queueMicrotask(f)}queueMicrotask(f)"),
        Err(Error::Limit { .. })
    ));
    let reports = Rc::new(RefCell::new(Reports {
        fatal: true,
        ..Reports::default()
    }));
    let mut host = ReportingHost(reports.clone());
    let mut r = Realm::new(Limits::default(), &mut host)?;
    r.install_queue_microtask()?;
    assert_eq!(
        r.evaluate("queueMicrotask(()=>{throw 7});queueMicrotask(()=>print(1))"),
        Err(Error::Host)
    );
    assert!(reports.borrow().output.is_empty());
    assert!(r.evaluate("1").is_err());
    Ok(())
}

#[test]
fn concat_preserves_holes_species_and_spreadability() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut r = Realm::new(Limits::default(), &mut host)?;
    for source in [
        "let d=Object.getOwnPropertyDescriptor(Array.prototype.concat,'length');d.value===1&&!d.writable&&!d.enumerable&&d.configurable",
        "[1,,3].concat([4,,],5).join()==='1,,3,4,,5'",
        "let a=[,];let b=a.concat([,]);b.length===2&&!(0 in b)&&!(1 in b)",
        "let a=[1];a[Symbol.isConcatSpreadable]=false;[].concat(a)[0]===a",
        "[].concat({0:'a',2:'b',length:3,[Symbol.isConcatSpreadable]:true}).join()==='a,,b'",
        "Array.prototype.concat.call('a',1)[0] instanceof String",
        "class A extends Array{}new A(1,2).concat(3) instanceof A",
        "class A extends Array{static get [Symbol.species](){return Array}}!(new A(1,2).concat(3) instanceof A)",
        "let out;function C(n){out=this;this.n=n}let a=[1,2];a.constructor={[Symbol.species]:C};a.concat(3)===out&&out.length===3&&out[2]===3&&out.n===0",
    ] {
        assert_eq!(
            r.evaluate(&format!("{{{source}}}"))?,
            Value::Boolean(true),
            "{source}"
        );
    }
    for source in [
        "let a=[];a.constructor=3;a.concat()",
        "let a=[];a.constructor={[Symbol.species]:()=>{}};a.concat()",
        "Array.prototype.concat.call(null)",
    ] {
        assert!(
            matches!(
                r.evaluate(&format!("{{{source}}}")),
                Err(Error::Type { .. })
            ),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn concat_getters_species_order_and_resource_errors_are_observable() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut r = Realm::new(
        Limits {
            heap_entries: 150,
            ..Limits::default()
        },
        &mut host,
    )?;
    for (source, expected) in [
        (
            "let log='';let a=[1];a.constructor={get [Symbol.species](){log+='s';return function(n){log+='c';return {}}}};Object.defineProperty(a,Symbol.isConcatSpreadable,{get(){log+='p';return true}});let b={get [Symbol.isConcatSpreadable](){log+='q';return true},get length(){log+='l';return 1},get 0(){log+='v';return 2}};a.concat(b);log",
            Value::string("scpqlv"),
        ),
        (
            "let a=[1],out={};a.constructor={[Symbol.species]:function(){return out}};Object.defineProperty(out,'0',{set(){throw 1},configurable:true});a.concat()[0]",
            Value::Number(1.0),
        ),
        (
            "let a=[1],out;let proto={set length(v){this.n=v}};a.constructor={[Symbol.species]:function(){out=Object.create(proto);return out}};a.concat([,]);out.n",
            Value::Number(2.0),
        ),
        (
            "let a=[1];a.constructor=null;try{a.concat()}catch(e){e.name}",
            Value::string("TypeError"),
        ),
        (
            "let a=[1];a.constructor={[Symbol.species]:null};a.concat()[0]",
            Value::Number(1.0),
        ),
        (
            "let a=[];a.constructor=undefined;a.concat(7)[0]",
            Value::Number(7.0),
        ),
        (
            "let a=[];a.constructor={[Symbol.species]:function(){return Object.freeze({})}};try{a.concat(7)}catch(e){e.name}",
            Value::string("TypeError"),
        ),
        (
            "let a=[1,2];Object.defineProperty(a,0,{get(){delete a[1];for(let i=0;i<200;i++){let t={}}return {n:7}}});let b=a.concat([3]);b[0].n===7&&b.length===3&&!(1 in b)",
            Value::Boolean(true),
        ),
    ] {
        assert_eq!(r.evaluate(&format!("{{{source}}}"))?, expected, "{source}");
    }
    let mut host = SilentHost;
    let mut r = Realm::new(
        Limits {
            fuel: 2000,
            ..Limits::default()
        },
        &mut host,
    )?;
    assert!(matches!(
        r.evaluate("[].concat({length:1000000,[Symbol.isConcatSpreadable]:true})"),
        Err(Error::Limit { .. })
    ));
    let mut host = SilentHost;
    let mut r = Realm::new(
        Limits {
            fuel: 2000,
            ..Limits::default()
        },
        &mut host,
    )?;
    assert!(matches!(
        r.evaluate("[].concat({length:4294967296,[Symbol.isConcatSpreadable]:true})"),
        Err(Error::Limit { .. })
    ));
    Ok(())
}

#[test]
fn microtask_errors_keep_exact_values_and_survive_collection() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut r = Realm::new(
        Limits {
            heap_entries: 120,
            ..Limits::default()
        },
        &mut host,
    )?;
    r.install_queue_microtask()?;
    let Err(Error::Thrown { value }) = r.evaluate(
        "queueMicrotask(()=>{throw {n:7}});queueMicrotask(()=>{for(let i=0;i<300;i++){let t={}}})",
    ) else {
        return Err(Error::InvalidBytecode);
    };
    assert_eq!(r.get(&value, &Value::string("n"))?, Value::Number(7.0));
    r.evaluate("let result=0;let cb=()=>{result=9};queueMicrotask(cb);cb=null;")?;
    assert_eq!(r.evaluate("result")?, Value::Number(9.0));
    Ok(())
}

#[test]
fn report_hook_cannot_return_foreign_exception_values() -> Result<(), Error> {
    struct ForeignReport(Value);
    impl Host for ForeignReport {
        fn print(&mut self, _: &[Value]) -> Result<(), Error> {
            Ok(())
        }
        fn report_exception(&mut self, _: &Value) -> Result<(), Error> {
            Err(Error::Thrown {
                value: self.0.clone(),
            })
        }
    }
    let mut h = SilentHost;
    let mut source = Realm::new(Limits::default(), &mut h)?;
    let value = source.object()?;
    let mut host = ForeignReport(value);
    let mut r = Realm::new(Limits::default(), &mut host)?;
    r.install_queue_microtask()?;
    assert_eq!(
        r.evaluate("queueMicrotask(()=>{throw 7})"),
        Err(Error::Host)
    );
    assert!(r.checkpoint().is_err());
    Ok(())
}
