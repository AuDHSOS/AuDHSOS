// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use crate::{Error, Host, Limits, Realm, SilentHost, Value};

fn check(source: &str) -> Result<(), Error> {
    let mut host = SilentHost;
    let mut realm = Realm::new(Limits::default(), &mut host)?;
    realm.install_events()?;
    assert_eq!(realm.evaluate(source)?, Value::Boolean(true), "{source}");
    Ok(())
}

#[test]
fn abort_interfaces_brands_descriptors_and_reason_identity() -> Result<(), Error> {
    for source in [
        "let c=new AbortController();c.signal===c.signal&&!c.signal.aborted&&c.signal.reason===undefined&&c.signal.onabort===null&&c.signal.throwIfAborted()===undefined",
        "let c=new AbortController();c.signal instanceof AbortSignal&&c.signal instanceof EventTarget&&Object.getPrototypeOf(AbortSignal)===EventTarget&&Object.getPrototypeOf(AbortSignal.prototype)===EventTarget.prototype",
        "Object.prototype.toString.call(new AbortController())==='[object AbortController]'&&Object.prototype.toString.call(new AbortController().signal)==='[object AbortSignal]'",
        "class C extends AbortController{}let c=new C();c instanceof C&&c instanceof AbortController&&c.signal instanceof AbortSignal",
        "let c=new AbortController(),r={};c.abort(r);c.abort(7);c.signal.aborted&&c.signal.reason===r",
        "let c=new AbortController();c.abort();c.signal.reason instanceof DOMException&&c.signal.reason.name==='AbortError'&&c.signal.reason.code===20",
        "let a=AbortSignal.abort(),b=AbortSignal.abort();a!==b&&a.reason!==b.reason&&a.aborted&&a.reason.name==='AbortError'",
        "let c=new AbortController();c.abort(null);let yes=false;try{c.signal.throwIfAborted()}catch(e){yes=e===null}yes&&c.signal.reason===null",
        "let r=Symbol();let s=AbortSignal.abort(r);let yes=false;try{s.throwIfAborted()}catch(e){yes=e===r}yes",
        "let d=Object.getOwnPropertyDescriptor(AbortController.prototype,'signal');d.enumerable&&d.configurable&&d.set===undefined&&d.get.name==='get signal'&&d.get.length===0",
        "AbortController.length===0&&AbortSignal.length===0&&AbortSignal.any.length===1&&AbortSignal.abort.length===0&&AbortController.prototype.abort.length===0&&AbortSignal.prototype.throwIfAborted.length===0",
        "AbortController.name==='AbortController'&&AbortSignal.name==='AbortSignal'&&AbortSignal.any.name==='any'&&AbortSignal.abort.name==='abort'&&AbortSignal.prototype.throwIfAborted.name==='throwIfAborted'",
        "let d=Object.getOwnPropertyDescriptor(AbortSignal,'prototype');!d.writable&&!d.enumerable&&!d.configurable",
    ] {
        check(source)?;
    }
    for source in [
        "AbortController()",
        "AbortSignal()",
        "new AbortSignal()",
        "AbortController.prototype.abort.call({})",
        "AbortSignal.prototype.throwIfAborted.call({})",
        "Object.getOwnPropertyDescriptor(AbortController.prototype,'signal').get.call({})",
        "Object.getOwnPropertyDescriptor(AbortSignal.prototype,'aborted').get.call({})",
        "Object.getOwnPropertyDescriptor(AbortSignal.prototype,'onabort').set.call({},null)",
        "class S extends AbortSignal{}new S()",
        "new AbortSignal.any([])",
    ] {
        let mut host = SilentHost;
        let mut r = Realm::new(Limits::default(), &mut host)?;
        r.install_events()?;
        assert!(
            matches!(r.evaluate(source), Err(Error::Type { .. })),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn abort_dispatch_and_handler_order_are_synchronous_and_trusted() -> Result<(), Error> {
    for source in [
        "let c=new AbortController(),s=c.signal,seen=0,event;s.addEventListener('abort',function(e){if(this===s&&e.target===s&&e.currentTarget===s&&e.type==='abort'&&e.isTrusted&&!e.bubbles&&!e.cancelable&&!e.composed&&s.aborted)seen++;event=e});c.abort();c.abort();seen===1&&event.currentTarget===null",
        "let c=new AbortController(),s=c.signal,log='';s.addEventListener('abort',()=>log+='a');s.onabort=()=>log+='b';s.addEventListener('abort',()=>log+='c');s.onabort=()=>log+='d';c.abort();log==='adc'",
        "let c=new AbortController(),s=c.signal,log='';s.onabort=()=>log+='a';s.addEventListener('abort',()=>log+='b');s.onabort=null;s.onabort=()=>log+='c';c.abort();log==='bc'",
        "let c=new AbortController(),s=c.signal,n=0;function f(){n++}s.onabort=f;s.removeEventListener('abort',f);s.addEventListener('abort',f);c.abort();n===2",
        "let s=new AbortController().signal;s.onabort={};let o=s.onabort;s.dispatchEvent(new Event('abort'));s.onabort===o",
        "let s=new AbortController().signal;s.onabort=()=>{};s.onabort=1;s.onabort===null",
        "let s=new AbortController().signal;let e=new Event('abort',{cancelable:true});s.onabort=()=>false;!s.dispatchEvent(e)&&e.defaultPrevented&&!s.aborted",
        "let c=new AbortController(),s=c.signal,n=0;s.onabort=()=>{n++;c.abort(2)};c.abort(1);s.reason===1&&n===1",
        "let c=new AbortController(),e;c.signal.onabort=x=>e=x;c.abort();let was=e.isTrusted;new EventTarget().dispatchEvent(e);was&&!e.isTrusted",
        "let c=new AbortController(),e;c.signal.onabort=x=>e=x;c.abort();e.initEvent('y');!e.isTrusted&&e.target===null",
        "let c=new AbortController(),n=0;c.signal.onabort=()=>{c.signal.onabort=null;n++};c.abort();c.signal.onabort===null&&n===1",
        "let c=new AbortController(),s=c.signal,log='';s.addEventListener('abort',()=>{s.onabort=()=>log+='x'});s.onabort=()=>log+='y';c.abort();log==='x'",
    ] {
        check(source)?;
    }
    Ok(())
}

#[test]
fn signal_listener_identity_and_abort_steps_precede_events() -> Result<(), Error> {
    for source in [
        "let c=new AbortController(),t=new EventTarget(),n=0;let f=()=>n++;t.addEventListener('x',f,{signal:c.signal});t.dispatchEvent(new Event('x'));c.signal.onabort=()=>t.dispatchEvent(new Event('x'));c.abort();t.dispatchEvent(new Event('x'));n===1",
        "let c=new AbortController(),t=new EventTarget(),n=0;let f=()=>n++;t.addEventListener('x',f);t.addEventListener('x',f,{signal:c.signal});c.abort();t.dispatchEvent(new Event('x'));n===1",
        "let a=new AbortController(),b=new AbortController(),t=new EventTarget(),n=0;let f=()=>n++;t.addEventListener('x',f,{signal:a.signal});t.addEventListener('x',f,{signal:b.signal});b.abort();t.dispatchEvent(new Event('x'));a.abort();t.dispatchEvent(new Event('x'));n===1",
        "let a=new AbortController(),b=new AbortController(),t=new EventTarget(),n=0;let f=()=>n++;t.addEventListener('x',f,{signal:a.signal});t.removeEventListener('x',f);t.addEventListener('x',f,{signal:b.signal});a.abort();t.dispatchEvent(new Event('x'));b.abort();t.dispatchEvent(new Event('x'));n===1",
        "let c=new AbortController(),t=new EventTarget(),n=0;let f=()=>{n++;t.addEventListener('x',f)};t.addEventListener('x',f,{once:true,signal:c.signal});t.dispatchEvent(new Event('x'));c.abort();t.dispatchEvent(new Event('x'));n===2",
        "let c=new AbortController(),t=new EventTarget(),n=0;t.addEventListener('x',()=>c.abort());t.addEventListener('x',()=>n++,{signal:c.signal});t.dispatchEvent(new Event('x'));n===0",
        "let t=new EventTarget(),n=0;let f=()=>n++;t.addEventListener('x',f,{signal:AbortSignal.abort()});t.dispatchEvent(new Event('x'));n===0",
        "let c=new AbortController(),t=new EventTarget(),log='';t.addEventListener('x',()=>{}, {get capture(){log+='c'},get once(){log+='o'},get passive(){log+='p'},get signal(){log+='s';return c.signal}});log==='cops'",
        "let c=new AbortController(),s=c.signal,n=0;s.addEventListener('abort',()=>n++,{signal:s});c.abort();n===0",
        "let c=new AbortController(),t=new EventTarget(),f=()=>{};t.removeEventListener('x',f,{get signal(){throw 7}});true",
    ] {
        check(source)?;
    }
    for signal in ["null", "{}", "1", "new AbortController()"] {
        check(&format!(
            "let t=new EventTarget();let yes=false;try{{t.addEventListener('x',null,{{signal:{signal}}})}}catch(e){{yes=e instanceof TypeError}}yes"
        ))?;
    }
    Ok(())
}

#[test]
fn any_flattens_dependencies_and_sets_all_reasons_before_callbacks() -> Result<(), Error> {
    for source in [
        "let s=AbortSignal.any([]);!s.aborted&&s.reason===undefined&&s instanceof AbortSignal",
        "let a=new AbortController(),b=new AbortController(),s=AbortSignal.any([a.signal,b.signal]),r={};b.abort(r);a.abort(1);s.aborted&&s.reason===r",
        "let a=AbortSignal.abort(1),b=AbortSignal.abort(2);AbortSignal.any([a,b]).reason===1&&AbortSignal.any([b,a]).reason===2",
        "let a=new AbortController(),s=AbortSignal.any([a.signal,a.signal]),n=0;s.onabort=()=>n++;a.abort();n===1",
        "let a=new AbortController(),b=new AbortController(),s=AbortSignal.any([a.signal,b.signal]),q=AbortSignal.any([s,a.signal]),log='';a.signal.onabort=()=>{if(!s.aborted||!q.aborted)throw 7;log+='a'};s.onabort=()=>log+='s';q.onabort=()=>log+='q';a.abort(1);log==='asq'&&q.reason===1",
        "let a=new AbortController(),s=AbortSignal.any([a.signal]),q=AbortSignal.any([s]),log='';a.signal.onabort=()=>{q.onabort=()=>log+='q';log+='a'};s.onabort=()=>log+='s';a.abort();log==='asq'",
        "let a=new AbortController(),b=new AbortController(),s=AbortSignal.any([a.signal,b.signal]);a.signal.onabort=()=>b.abort(2);a.abort(1);s.reason===1",
        "let a=new AbortController(),s=AbortSignal.any([a.signal]),later;a.signal.onabort=()=>later=AbortSignal.any([s]);a.abort(3);later.aborted&&later.reason===3",
        "let a=new AbortController(),s=AbortSignal.any([a.signal]),t=new EventTarget(),n=0;t.addEventListener('x',()=>n++,{signal:s});s.onabort=()=>t.dispatchEvent(new Event('x'));a.abort();n===0",
        "let a=new AbortController(),n=0;let seq={[Symbol.iterator](){return {next(){if(n++===0)return {value:a.signal};a.abort(7);return {done:true}}}}};AbortSignal.any(seq).reason===7",
    ] {
        check(source)?;
    }
    Ok(())
}

#[test]
fn sequence_conversion_is_complete_before_any_algorithm_and_does_not_close() -> Result<(), Error> {
    for source in [
        "let n=0;let seq={[Symbol.iterator](){return {next(){if(n++===0)return {value:AbortSignal.abort(1)};return {value:7}},return(){throw 8}}}};let yes=false;try{AbortSignal.any(seq)}catch(e){yes=e instanceof TypeError}yes&&n===2",
        "let closed=false;let seq={[Symbol.iterator](){return {next(){return {value:7}},return(){closed=true;return {}}}}};try{AbortSignal.any(seq)}catch(e){}!closed",
        "let read=false;let seq={[Symbol.iterator](){return {next(){return {done:true,get value(){read=true}}}}}};!AbortSignal.any(seq).aborted&&!read",
        "let n=0;let s=new AbortController().signal;let seq={[Symbol.iterator](){return {get next(){n++;let done=false;return function(){if(done)return {done:true};done=true;return {value:s}}}}}};AbortSignal.any(seq);n===1",
    ] {
        check(source)?;
    }
    for expression in [
        "AbortSignal.any()",
        "AbortSignal.any(null)",
        "AbortSignal.any('')",
        "AbortSignal.any(7)",
        "AbortSignal.any({})",
        "AbortSignal.any([{}])",
        "AbortSignal.any([null])",
    ] {
        check(&format!(
            "let yes=false;try{{{expression}}}catch(e){{yes=e instanceof TypeError}}yes"
        ))?;
    }
    Ok(())
}

#[test]
fn abort_dependencies_handlers_and_reason_survive_gc() -> Result<(), Error> {
    let limits = Limits {
        heap_entries: 210,
        ..Limits::default()
    };
    for source in [
        "let c=new AbortController(),n=0;function setup(){let s=AbortSignal.any([c.signal]);s.onabort=()=>n++}setup();for(let i=0;i<600;i++){let x={}}c.abort();n===1",
        "let c=new AbortController(),s=AbortSignal.any([c.signal]),n=0;s.onabort=()=>{for(let i=0;i<600;i++){let x={}}n++};c.signal.onabort=()=>{for(let i=0;i<600;i++){let x={}}};c.abort({x:7});n===1&&s.reason.x===7",
        "let c=new AbortController(),t=new EventTarget(),n=0;function setup(){let s=AbortSignal.any([c.signal]);t.addEventListener('x',()=>n++,{signal:s})}setup();for(let i=0;i<600;i++){let x={}}c.abort();t.dispatchEvent(new Event('x'));n===0",
        "let a=new AbortController(),b=new AbortController(),s=AbortSignal.any([a.signal,b.signal]);a=null;for(let i=0;i<600;i++){let x={}}let q=AbortSignal.any([s]);b.abort(7);q.reason===7",
        "let c=new AbortController(),t=new EventTarget();for(let i=0;i<600;i++){let f=()=>{};t.addEventListener('x',f,{signal:c.signal});t.removeEventListener('x',f)}c.abort();true",
        "let c=new AbortController(),n=0;for(let i=0;i<400;i++){let s=AbortSignal.any([c.signal]);s.onabort=()=>n++;s.onabort=null}c.abort();n===0",
        "let c=new AbortController(),n=0;c.signal.onabort=()=>{n++;for(let i=0;i<600;i++){let x={}}};c.abort();c.signal.reason.name==='AbortError'&&n===1",
    ] {
        let mut host = SilentHost;
        let mut r = Realm::new(limits, &mut host)?;
        r.install_events()?;
        assert_eq!(r.evaluate(source)?, Value::Boolean(true), "{source}");
    }
    Ok(())
}

#[test]
fn abort_listener_errors_report_after_all_dependents_and_limits_stop_work() -> Result<(), Error> {
    struct Reporting {
        count: usize,
    }
    impl Host for Reporting {
        fn print(&mut self, _: &[Value]) -> Result<(), Error> {
            Ok(())
        }
        fn report_exception(&mut self, _: &Value) -> Result<(), Error> {
            self.count += 1;
            Ok(())
        }
    }
    let mut host = Reporting { count: 0 };
    {
        let mut r = Realm::new(Limits::default(), &mut host)?;
        r.install_events()?;
        assert_eq!(r.evaluate("let c=new AbortController(),s=AbortSignal.any([c.signal]),n=0;c.signal.onabort=()=>{throw 7};s.onabort=()=>n++;c.abort();n")?,Value::Number(1.0));
    }
    assert_eq!(host.count, 1);
    for source in [
        "AbortSignal.any({[Symbol.iterator](){return {next(){return {value:new AbortController().signal}}}}})",
        "let c=new AbortController();for(let i=0;i<100;i++){AbortSignal.any([c.signal]).onabort=()=>{}}",
        "let c=new AbortController(),t=new EventTarget();for(let i=0;i<100;i++){t.addEventListener('x',()=>{},{signal:c.signal})}",
    ] {
        let mut host = SilentHost;
        let mut r = Realm::new(
            Limits {
                properties: 64,
                fuel: 20000,
                ..Limits::default()
            },
            &mut host,
        )?;
        r.install_events()?;
        assert!(
            matches!(r.evaluate(source), Err(Error::Limit { .. })),
            "{source}"
        );
    }
    Ok(())
}
