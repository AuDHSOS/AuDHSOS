// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use crate::{Error, Host, Limits, Realm, Value};
use std::{cell::Cell, rc::Rc};

struct Clock {
    now: Rc<Cell<u128>>,
    reports: Rc<Cell<u32>>,
    deny: bool,
}
impl Host for Clock {
    fn timer_now(&mut self) -> Result<u128, Error> {
        Ok(self.now.get())
    }
    fn print(&mut self, _: &[Value]) -> Result<(), Error> {
        Ok(())
    }
    fn report_exception(&mut self, _: &Value) -> Result<(), Error> {
        self.reports.set(self.reports.get().saturating_add(1));
        Ok(())
    }
    fn ensure_can_compile_strings(&mut self) -> Result<(), Error> {
        if self.deny { Err(Error::Host) } else { Ok(()) }
    }
}
fn clock() -> Clock {
    Clock {
        now: Rc::new(Cell::new(0)),
        reports: Rc::new(Cell::new(0)),
        deny: false,
    }
}

#[test]
fn timer_deadlines_cancellation_interval_and_microtask_order() -> Result<(), Error> {
    let mut host = clock();
    let now = host.now.clone();
    let mut realm = Realm::new(Limits::default(), &mut host)?;
    realm.install_timers()?;
    realm.install_queue_microtask()?;
    realm.evaluate("let log='';let canceled=setTimeout(()=>log+='X',0);clearInterval(canceled);setTimeout(()=>{log+='a';queueMicrotask(()=>log+='m')},5);setTimeout(()=>log+='b',5);let n=0;let id=setInterval(()=>{log+='i';if(++n===2)clearTimeout(id)},2);")?;
    assert_eq!(realm.timer_wait()?, Some(2_000_000));
    assert!(!realm.run_timer()?);
    now.set(2_000_000);
    assert!(realm.run_timer()?);
    assert_eq!(realm.evaluate("log")?, Value::string("i"));
    now.set(4_000_000);
    assert!(realm.run_timer()?);
    assert_eq!(realm.evaluate("log")?, Value::string("ii"));
    now.set(5_000_000);
    assert!(realm.run_timer()?);
    assert_eq!(realm.evaluate("log")?, Value::string("iiam"));
    assert!(realm.run_timer()?);
    assert_eq!(realm.evaluate("log")?, Value::string("iiamb"));
    assert_eq!(realm.timer_wait()?, None);
    assert!(!realm.run_timer()?);
    Ok(())
}

#[test]
fn nesting_clamp_arguments_this_string_scripts_and_metadata() -> Result<(), Error> {
    let mut host = clock();
    let now = host.now.clone();
    let mut realm = Realm::new(Limits::default(), &mut host)?;
    realm.install_timers()?;
    realm.evaluate("let n=0;function f(){n++;if(n<8)setTimeout(f,0)}setTimeout(f,0)")?;
    for _ in 0..6 {
        assert!(realm.run_timer()?);
    }
    assert_eq!(realm.timer_wait()?, Some(4_000_000));
    now.set(4_000_000);
    assert!(realm.run_timer()?);
    assert_eq!(realm.timer_wait()?, Some(4_000_000));
    now.set(8_000_000);
    assert!(realm.run_timer()?);
    realm.evaluate("let ok=false;setTimeout(function(a,b){'use strict';ok=this===globalThis&&a.n===7&&b===8},0,{n:7},8)")?;
    assert!(realm.run_timer()?);
    assert_eq!(realm.evaluate("ok")?, Value::Boolean(true));
    realm.evaluate("let seen=0;function schedule(){let hidden=9;setTimeout('seen=typeof hidden;var timerGlobal=42',0)}schedule()")?;
    realm.run_timer()?;
    assert_eq!(
        realm.evaluate("seen==='undefined'&&timerGlobal===42")?,
        Value::Boolean(true)
    );
    for source in [
        "setTimeout.length===1&&setInterval.length===1&&clearTimeout.length===0&&clearInterval.name==='clearInterval'",
        "let d=Object.getOwnPropertyDescriptor(globalThis,'setTimeout');d.writable&&d.enumerable&&d.configurable",
        "let f=setTimeout;f.extra=7;delete f.name;f.extra===7&&!Object.hasOwn(f,'name')&&f.prototype===undefined",
        "let log='';let id=setTimeout({toString(){log+='s';return '0'}},{valueOf(){log+='d';return 0}});clearTimeout(id);log==='sd'",
        "let e={},ok=false;try{setTimeout({toString(){throw e}},{valueOf(){throw 7}})}catch(x){ok=x===e}ok",
    ] {
        assert_eq!(
            realm.evaluate(&format!("{{{source}}}"))?,
            Value::Boolean(true),
            "{source}"
        );
    }
    for source in [
        "setTimeout()",
        "setTimeout(Symbol())",
        "setTimeout(()=>1,Symbol())",
        "clearTimeout(Symbol())",
        "setTimeout.call({},()=>1)",
        "new setTimeout()",
    ] {
        assert!(
            matches!(realm.evaluate(source), Err(Error::Type { .. })),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn abort_timeouts_order_range_and_async_completion() -> Result<(), Error> {
    let mut host = clock();
    let now = host.now.clone();
    let mut realm = Realm::new(Limits::default(), &mut host)?;
    realm.install_events()?;
    realm.install_timers()?;
    realm.evaluate("let log='';let s=AbortSignal.timeout(0);s.onabort=()=>log+='a';let dependent=AbortSignal.any([s]);dependent.onabort=()=>log+='b';let first=AbortSignal.timeout(5);first.onabort=()=>log+='c';let second=AbortSignal.timeout(5);second.onabort=()=>log+='d';")?;
    assert_eq!(realm.evaluate("s.aborted")?, Value::Boolean(false));
    assert!(realm.run_timer()?);
    assert_eq!(realm.evaluate("log==='ab'&&s.aborted&&s.reason===dependent.reason&&s.reason instanceof DOMException&&s.reason.name==='TimeoutError'&&s.reason.code===23")?,Value::Boolean(true));
    now.set(5_000_000);
    assert!(realm.run_timer()?);
    assert!(realm.run_timer()?);
    assert_eq!(realm.evaluate("log")?, Value::string("abcd"));
    for source in [
        "AbortSignal.timeout()",
        "AbortSignal.timeout(NaN)",
        "AbortSignal.timeout(Infinity)",
        "AbortSignal.timeout(-1)",
        "AbortSignal.timeout(9007199254740992)",
        "AbortSignal.timeout(Symbol())",
    ] {
        assert!(
            matches!(realm.evaluate(source), Err(Error::Type { .. })),
            "{source}"
        );
    }
    realm.evaluate("let zero=AbortSignal.timeout(-0.5);let rounded=AbortSignal.timeout(1.9);let long=AbortSignal.timeout(Number.MAX_SAFE_INTEGER)")?;
    assert!(realm.run_timer()?);
    assert_eq!(realm.timer_wait()?, Some(1_000_000));
    now.set(6_000_000);
    assert!(realm.run_timer()?);
    assert_eq!(
        realm.evaluate("zero.aborted&&rounded.aborted&&!long.aborted")?,
        Value::Boolean(true)
    );
    Ok(())
}

#[test]
fn reported_errors_preserve_interval_and_gc_roots() -> Result<(), Error> {
    let mut host = clock();
    let reports = host.reports.clone();
    let mut realm = Realm::new(
        Limits {
            heap_entries: 180,
            ..Limits::default()
        },
        &mut host,
    )?;
    realm.install_timers()?;
    realm.install_events()?;
    realm.evaluate("let n=0;let id=setInterval(()=>{if(++n===2)clearTimeout(id);throw 7},0);setTimeout(()=>{for(let i=0;i<300;i++){let g={}}},0)")?;
    assert!(realm.run_timer()?);
    assert!(realm.run_timer()?);
    assert!(realm.run_timer()?);
    assert_eq!(reports.get(), 2);
    realm.evaluate(
        "let seen=0;setTimeout(v=>{seen=v.n},0,{n:42});for(let i=0;i<300;i++){let g={}}",
    )?;
    realm.run_timer()?;
    assert_eq!(realm.evaluate("seen")?, Value::Number(42.0));
    realm.evaluate("setTimeout('let =',0)")?;
    realm.run_timer()?;
    assert_eq!(reports.get(), 3);
    Ok(())
}

#[test]
fn timer_limits_and_clock_failures_are_fatal() -> Result<(), Error> {
    let mut silent = crate::SilentHost;
    let mut realm = Realm::new(Limits::default(), &mut silent)?;
    assert!(matches!(
        realm.install_timers(),
        Err(Error::Unsupported { .. })
    ));
    assert!(realm.evaluate("1").is_err());
    let mut host = clock();
    let now = host.now.clone();
    now.set(10);
    let mut realm = Realm::new(Limits::default(), &mut host)?;
    realm.install_timers()?;
    realm.evaluate("setTimeout(()=>1,1)")?;
    now.set(9);
    assert_eq!(realm.timer_wait(), Err(Error::Host));
    let mut host = clock();
    let mut realm = Realm::new(
        Limits {
            jobs: 2,
            ..Limits::default()
        },
        &mut host,
    )?;
    realm.install_timers()?;
    assert!(matches!(
        realm.evaluate("setTimeout(()=>1);setTimeout(()=>1);try{setTimeout(()=>1)}catch(e){}"),
        Err(Error::Limit { resource: "timers" })
    ));
    let mut host = clock();
    host.deny = true;
    let mut realm = Realm::new(Limits::default(), &mut host)?;
    realm.install_timers()?;
    realm.evaluate("setTimeout('42')")?;
    assert_eq!(realm.run_timer(), Err(Error::Host));
    let mut host = clock();
    let mut realm = Realm::new(
        Limits {
            fuel: 1000,
            ..Limits::default()
        },
        &mut host,
    )?;
    realm.install_timers()?;
    realm.evaluate("setTimeout(()=>{while(true){}})")?;
    assert!(matches!(
        realm.run_timer(),
        Err(Error::Limit {
            resource: "execution fuel"
        })
    ));
    Ok(())
}

#[test]
fn cancellation_during_tasks_and_checkpoints_preserves_order() -> Result<(), Error> {
    let mut host = clock();
    let now = host.now.clone();
    let mut realm = Realm::new(Limits::default(), &mut host)?;
    realm.install_timers()?;
    realm.install_queue_microtask()?;
    realm.evaluate("let log='';let first=setTimeout(()=>{log+='a';clearTimeout(second);setTimeout(()=>log+='c',0)},0);let second=setTimeout(()=>log+='b',0);")?;
    now.set(1_000_000);
    assert!(realm.run_timer()?);
    assert!(realm.run_timer()?);
    assert!(!realm.run_timer()?);
    assert_eq!(realm.evaluate("log")?, Value::string("ac"));
    realm
        .evaluate("let id=setInterval(()=>{log+='i';queueMicrotask(()=>clearInterval(id))},0);")?;
    assert!(realm.run_timer()?);
    assert!(!realm.run_timer()?);
    realm.evaluate("let n=0;function chain(){n++;if(n<7)queueMicrotask(()=>setTimeout(chain,0))}setTimeout(chain,0)")?;
    for _ in 0..7 {
        assert!(realm.run_timer()?);
    }
    assert_eq!(realm.timer_wait()?, None);
    realm.evaluate("let promiseCount=0;function promiseChain(){promiseCount++;if(promiseCount<10)Promise.resolve().then(()=>setTimeout(promiseChain,0))}setTimeout(promiseChain,0)")?;
    for _ in 0..10 {
        assert!(realm.run_timer()?);
    }
    assert_eq!(realm.timer_wait()?, None);
    realm.evaluate("setTimeout(()=>log+='z',4294967296);clearTimeout(-100);clearInterval();")?;
    assert!(realm.run_timer()?);
    assert_eq!(realm.evaluate("log")?, Value::string("aciz"));
    // Reinstalling is idempotent, not a source of replacement identities.
    realm.evaluate("let saved=setTimeout")?;
    realm.install_timers()?;
    assert_eq!(realm.evaluate("saved===setTimeout")?, Value::Boolean(true));
    Ok(())
}

#[test]
fn timer_callback_errors_do_not_skip_checkpoints_or_interval_cleanup() -> Result<(), Error> {
    struct Reject;
    impl Host for Reject {
        fn print(&mut self, _: &[Value]) -> Result<(), Error> {
            Ok(())
        }
        fn timer_now(&mut self) -> Result<u128, Error> {
            Ok(0)
        }
    }
    let mut host = Reject;
    let mut realm = Realm::new(Limits::default(), &mut host)?;
    realm.install_timers()?;
    realm.install_queue_microtask()?;
    realm.evaluate("let log='';setTimeout(()=>{queueMicrotask(()=>log+='m');throw 7},0)")?;
    assert_eq!(
        realm.run_timer(),
        Err(Error::Thrown {
            value: Value::Number(7.0)
        })
    );
    assert_eq!(realm.evaluate("log")?, Value::string("m"));
    assert_eq!(realm.timer_wait()?, None);
    let mut host = clock();
    let reports = host.reports.clone();
    let mut realm = Realm::new(Limits::default(), &mut host)?;
    realm.install_timers()?;
    realm.evaluate("let n=0;let id=setInterval('if(++n===2)clearTimeout(id);throw 7',0)")?;
    assert!(realm.run_timer()?);
    assert!(realm.run_timer()?);
    assert_eq!(reports.get(), 2);
    assert_eq!(realm.timer_wait()?, None);
    Ok(())
}

#[test]
fn timer_gc_cancellation_and_argument_quota_are_bounded() -> Result<(), Error> {
    let mut host = clock();
    let mut realm = Realm::new(
        Limits {
            heap_entries: 180,
            stack: 32,
            ..Limits::default()
        },
        &mut host,
    )?;
    realm.install_timers()?;
    realm.install_events()?;
    realm.evaluate(
        "let seen=0;AbortSignal.timeout(0).onabort=()=>{seen++};for(let i=0;i<300;i++){let g={}}",
    )?;
    realm.run_timer()?;
    assert_eq!(realm.evaluate("seen")?, Value::Number(1.0));
    realm.evaluate("for(let i=0;i<100;i++){let id=setTimeout(()=>{},0,{n:i});clearTimeout(id)}")?;
    assert_eq!(realm.timer_wait()?, None);
    assert!(matches!(
        realm.evaluate("for(let i=0;i<40;i++)setTimeout(()=>{},0,{})"),
        Err(Error::Limit {
            resource: "timer arguments"
        })
    ));
    let mut host = clock();
    let now = host.now.clone();
    now.set(u128::MAX);
    let mut realm = Realm::new(Limits::default(), &mut host)?;
    realm.install_timers()?;
    assert!(matches!(
        realm.evaluate("setTimeout(()=>{},1)"),
        Err(Error::Limit {
            resource: "timer deadline"
        })
    ));
    Ok(())
}
