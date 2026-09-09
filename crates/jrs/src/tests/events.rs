// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use crate::{Error, Host, Limits, Realm, SilentHost, Value};
use std::{cell::RefCell, rc::Rc};

fn check(source: &str) -> Result<(), Error> {
    let mut host = SilentHost;
    let mut r = Realm::new(Limits::default(), &mut host)?;
    r.install_events()?;
    assert_eq!(r.evaluate(source)?, Value::Boolean(true), "{source}");
    Ok(())
}

#[test]
fn standalone_events_properties_brands_and_subclasses() -> Result<(), Error> {
    for source in [
        "let e=new Event('x',{bubbles:true,cancelable:true,composed:true});e.type==='x'&&e.bubbles&&e.cancelable&&e.composed&&!e.isTrusted&&e.target===null&&e.currentTarget===null&&e.eventPhase===0",
        "let x={},e=new CustomEvent('x',{detail:x});e instanceof Event&&e instanceof CustomEvent&&e.detail===x",
        "new CustomEvent('x').detail===null",
        "class T extends EventTarget{}let t=new T();t instanceof T&&t instanceof EventTarget",
        "class E extends Event{}new E('x') instanceof E",
        "Object.prototype.toString.call(new Event('x'))==='[object Event]'&&Object.prototype.toString.call(new EventTarget())==='[object EventTarget]'",
        "Event.NONE===0&&Event.AT_TARGET===2&&Event.prototype.BUBBLING_PHASE===3",
        "let d=Object.getOwnPropertyDescriptor(new Event('x'),'isTrusted');!d.configurable&&d.enumerable&&d.set===undefined",
        "let e=new Event('x'),f=new Event('y');Object.getOwnPropertyDescriptor(e,'isTrusted').get===Object.getOwnPropertyDescriptor(f,'isTrusted').get",
        "let d=Object.getOwnPropertyDescriptor(Event,'prototype');!d.enumerable&&!d.writable&&!d.configurable",
        "let e=new Event('\\ud800');e.type==='\\ud800'",
    ] {
        check(source)?;
    }
    let mut host = SilentHost;
    let mut r = Realm::new(Limits::default(), &mut host)?;
    assert_eq!(
        r.evaluate("typeof EventTarget")?,
        Value::string("undefined")
    );
    r.install_events()?;
    for source in [
        "Event('x')",
        "EventTarget()",
        "new Event()",
        "new Event('x',3)",
        "Event.prototype.preventDefault.call({})",
        "EventTarget.prototype.addEventListener.call({},'x',()=>{})",
        "new EventTarget().dispatchEvent({})",
        "new EventTarget().addEventListener('x')",
        "new EventTarget().addEventListener('x',3)",
    ] {
        assert!(
            matches!(r.evaluate(source), Err(Error::Type { .. })),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn target_dispatch_observes_receivers_paths_and_no_parent_propagation() -> Result<(), Error> {
    for source in [
        "let t=new EventTarget(),e=new Event('x'),count=0;t.addEventListener('x',function(v){if(this===t&&v===e&&e.target===t&&e.currentTarget===t&&e.eventPhase===2&&e.composedPath()[0]===t)count++});let result=t.dispatchEvent(e);result&&count===1&&e.target===t&&e.currentTarget===null&&e.eventPhase===0&&e.composedPath().length===0",
        "let t=new EventTarget(),e=new Event('x'),log='';t.addEventListener('x',()=>log+='b');t.addEventListener('x',()=>log+='c',true);t.dispatchEvent(e);log==='cb'",
        "let t=new EventTarget(),log='';let listener={handleEvent(e){if(this===listener&&e.type==='x')log+='o'}};t.addEventListener('x',listener);t.dispatchEvent(new Event('x'));listener.handleEvent=()=>log+='n';t.dispatchEvent(new Event('x'));log==='on'",
        "let t=new EventTarget(),n=0,f=()=>n++;t.addEventListener('x',f);t.addEventListener('x',f,{passive:true,once:true});t.dispatchEvent(new Event('x'));t.dispatchEvent(new Event('x'));n===2",
        "let t=new EventTarget(),n=0,f=()=>n++;t.addEventListener('x',f,true);t.addEventListener('x',f,false);t.removeEventListener('x',f);t.dispatchEvent(new Event('x'));n===1",
    ] {
        check(source)?;
    }
    Ok(())
}

#[test]
fn listener_snapshot_removal_readdition_once_and_nested_dispatch() -> Result<(), Error> {
    for (source, expected) in [
        (
            "let t=new EventTarget(),log='';let b=()=>log+='b';t.addEventListener('x',()=>{log+='a';t.removeEventListener('x',b);t.addEventListener('x',b)});t.addEventListener('x',b);t.dispatchEvent(new Event('x'));log",
            "a",
        ),
        (
            "let t=new EventTarget(),log='';t.addEventListener('x',()=>{log+='a';t.addEventListener('x',()=>log+='b')});t.dispatchEvent(new Event('x'));log",
            "a",
        ),
        (
            "let t=new EventTarget(),log='';t.addEventListener('x',()=>{log+='c';t.addEventListener('x',()=>log+='b')},true);t.dispatchEvent(new Event('x'));log",
            "cb",
        ),
        (
            "let t=new EventTarget(),n=0;t.addEventListener('x',()=>{n++;t.dispatchEvent(new Event('x'))},{once:true});t.dispatchEvent(new Event('x'));String(n)",
            "1",
        ),
        (
            "let t=new EventTarget(),n=0;function f(){n++;if(n===1)t.addEventListener('x',f,{once:true});if(n<3)t.dispatchEvent(new Event('x'))}t.addEventListener('x',f,{once:true});t.dispatchEvent(new Event('x'));String(n)",
            "2",
        ),
    ] {
        let mut host = SilentHost;
        let mut r = Realm::new(Limits::default(), &mut host)?;
        r.install_events()?;
        assert_eq!(r.evaluate(source)?, Value::string(expected), "{source}");
    }
    Ok(())
}

#[test]
fn cancelation_passive_propagation_and_reinitialization() -> Result<(), Error> {
    for source in [
        "let t=new EventTarget(),e=new Event('x',{cancelable:true});t.addEventListener('x',e=>e.preventDefault(),{passive:true});t.dispatchEvent(e)&&!e.defaultPrevented",
        "let t=new EventTarget(),e=new Event('x',{cancelable:true});t.addEventListener('x',e=>e.returnValue=false,{passive:true});t.dispatchEvent(e)&&e.returnValue",
        "let t=new EventTarget(),e=new Event('x',{cancelable:true});t.addEventListener('x',e=>e.preventDefault());!t.dispatchEvent(e)&&e.defaultPrevented&&!e.returnValue",
        "let e=new Event('x');e.preventDefault();!e.defaultPrevented",
        "let t=new EventTarget(),e=new Event('x'),n=0;t.addEventListener('x',e=>{n++;e.stopPropagation()},true);t.addEventListener('x',()=>n++,true);t.addEventListener('x',()=>n++);t.dispatchEvent(e);n===2&&!e.cancelBubble",
        "let t=new EventTarget(),e=new Event('x'),n=0;t.addEventListener('x',e=>{n++;e.stopImmediatePropagation()},{once:true});t.addEventListener('x',()=>n++);t.dispatchEvent(e);t.dispatchEvent(e);n===2",
        "let t=new EventTarget(),e=new Event('x'),n=0;t.addEventListener('x',()=>n++);e.cancelBubble=true;e.cancelBubble=false;t.dispatchEvent(e);n===0&&!e.cancelBubble",
        "let e=new Event('x',{cancelable:true,composed:true});e.preventDefault();e.initEvent('y',true,false);e.type==='y'&&e.bubbles&&!e.cancelable&&!e.defaultPrevented&&e.composed",
        "let t=new EventTarget(),e=new Event('x');t.addEventListener('x',e=>e.initEvent('y'));t.dispatchEvent(e);e.type==='x';",
    ] {
        check(source)?;
    }
    Ok(())
}

#[test]
fn option_getters_and_callback_validation_order() -> Result<(), Error> {
    for (source, expected) in [
        (
            "let log='';let t=new EventTarget();t.addEventListener('x',null,{get capture(){log+='c'},get once(){log+='o'},get passive(){log+='p'},get signal(){log+='s'}});log",
            "cops",
        ),
        (
            "let log='';let t=new EventTarget();t.removeEventListener('x',null,{get capture(){log+='c'},get passive(){log+='p'}});log",
            "c",
        ),
        (
            "let log='';let t=new EventTarget();try{t.addEventListener({toString(){log+='t';return 'x'}},1,{get capture(){log+='c'}})}catch(e){}log",
            "t",
        ),
        (
            "let log='';new CustomEvent({toString(){log+='t';return 'x'}},{get bubbles(){log+='b'},get cancelable(){log+='c'},get composed(){log+='m'},get detail(){log+='d'}});log",
            "tbcmd",
        ),
    ] {
        let mut h = SilentHost;
        let mut r = Realm::new(Limits::default(), &mut h)?;
        r.install_events()?;
        assert_eq!(r.evaluate(source)?, Value::string(expected));
    }
    Ok(())
}

#[derive(Default)]
struct Reports {
    values: Vec<Value>,
    fatal: bool,
}
struct Reporting(Rc<RefCell<Reports>>);
impl Host for Reporting {
    fn print(&mut self, _: &[Value]) -> Result<(), Error> {
        Ok(())
    }
    fn event_timestamp(&mut self) -> f64 {
        12.5
    }
    fn report_exception(&mut self, value: &Value) -> Result<(), Error> {
        let mut s = self.0.borrow_mut();
        s.values.push(value.clone());
        if s.fatal { Err(Error::Host) } else { Ok(()) }
    }
}
#[test]
fn listener_exceptions_are_reported_not_thrown_by_dispatch() -> Result<(), Error> {
    let reports = Rc::new(RefCell::new(Reports::default()));
    let mut h = Reporting(reports.clone());
    let mut r = Realm::new(Limits::default(), &mut h)?;
    r.install_events()?;
    assert_eq!(r.evaluate("let t=new EventTarget(),n=0,e=new Event('x');t.addEventListener('x',()=>{throw 7});t.addEventListener('x',()=>n++);let caught=false;try{t.dispatchEvent(e)}catch(e){caught=true}n===1&&!caught&&e.currentTarget===null&&e.timeStamp===12.5")?,Value::Boolean(true));
    assert_eq!(reports.borrow().values, vec![Value::Number(7.0)]);
    reports.borrow_mut().fatal = true;
    assert_eq!(r.evaluate("t.dispatchEvent(e)"), Err(Error::Host));
    assert!(r.evaluate("1").is_err());
    let mut h = SilentHost;
    let mut r = Realm::new(Limits::default(), &mut h)?;
    r.install_events()?;
    assert_eq!(r.evaluate("let n=0,t=new EventTarget();t.addEventListener('x',()=>{throw 7});t.addEventListener('x',()=>n++);try{t.dispatchEvent(new Event('x'))}catch(e){n=99}"),Err(Error::Thrown{value:Value::Number(7.0)}));
    assert_eq!(r.evaluate("n")?, Value::Number(1.0));
    Ok(())
}

#[test]
fn event_listener_and_detail_roots_survive_gc_and_once_removal() -> Result<(), Error> {
    let mut h = SilentHost;
    let mut r = Realm::new(
        Limits {
            heap_entries: 150,
            ..Limits::default()
        },
        &mut h,
    )?;
    r.install_events()?;
    assert_eq!(r.evaluate("let n=0,t=new EventTarget();t.addEventListener('x',e=>{for(let i=0;i<200;i++){let a={}}n=e.detail.n},{once:true});let e=new CustomEvent('x',{detail:{n:7}});for(let i=0;i<200;i++){let a={}}t.dispatchEvent(e);n===7&&e.target===t")?,Value::Boolean(true));
    let mut h = SilentHost;
    let mut r = Realm::new(
        Limits {
            fuel: 3000,
            ..Limits::default()
        },
        &mut h,
    )?;
    r.install_events()?;
    assert!(matches!(r.evaluate("let t=new EventTarget();t.addEventListener('x',()=>t.dispatchEvent(new Event('x')));t.dispatchEvent(new Event('x'))"),Err(Error::Limit{..})));
    Ok(())
}

#[test]
fn redispatch_rejects_same_event_and_domexception_has_real_brand() -> Result<(), Error> {
    for source in [
        "let t=new EventTarget(),e=new Event('x'),ok=false;t.addEventListener('x',()=>{try{t.dispatchEvent(e)}catch(x){ok=x instanceof DOMException&&x instanceof Error&&x.name==='InvalidStateError'&&x.code===11}});t.dispatchEvent(e);ok&&e.eventPhase===0",
        "let e=new DOMException('bad','InvalidStateError');e.name==='InvalidStateError'&&e.message==='bad'&&e.code===11&&String(e)==='InvalidStateError: bad'",
        "new DOMException().name==='Error'&&new DOMException().message===''&&new DOMException('','not-known').code===0",
        "DOMException.INVALID_STATE_ERR===11&&DOMException.prototype.DATA_CLONE_ERR===25&&DOMException.DOMSTRING_SIZE_ERR===2",
        "class E extends DOMException{}new E('x','SyntaxError') instanceof E&&new E('x','SyntaxError').code===12",
        "let e=new CustomEvent('x',{detail:7,composed:true,cancelable:true});e.preventDefault();e.initCustomEvent('y',true,false,9);e.type==='y'&&e.bubbles&&!e.cancelable&&!e.defaultPrevented&&e.composed&&e.detail===9",
        "let e=new CustomEvent('x',{detail:7}),t=new EventTarget();t.addEventListener('x',()=>e.initCustomEvent('y',true,true,9));t.dispatchEvent(e);e.type==='x'&&e.detail===7",
    ] {
        check(source)?;
    }
    let mut host = SilentHost;
    let mut r = Realm::new(Limits::default(), &mut host)?;
    r.install_events()?;
    for source in [
        "DOMException()",
        "DOMException.prototype.name",
        "Object.getOwnPropertyDescriptor(DOMException.prototype,'code').get.call({})",
        "CustomEvent.prototype.initCustomEvent.call(new Event('x'),'y')",
    ] {
        assert!(
            matches!(r.evaluate(source), Err(Error::Type { .. })),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn events_negative_paths_listener_errors_and_quotas() -> Result<(), Error> {
    struct BadClock;
    impl Host for BadClock {
        fn print(&mut self, _: &[Value]) -> Result<(), Error> {
            Ok(())
        }
        fn event_timestamp(&mut self) -> f64 {
            f64::NAN
        }
    }
    let reports = Rc::new(RefCell::new(Reports::default()));
    let mut h = Reporting(reports.clone());
    let mut r = Realm::new(Limits::default(), &mut h)?;
    r.install_events()?;
    r.evaluate("let t=new EventTarget();t.addEventListener('x',{get handleEvent(){throw 9}});t.dispatchEvent(new Event('x'))")?;
    assert_eq!(reports.borrow().values, vec![Value::Number(9.0)]);
    for source in [
        "new EventTarget().dispatchEvent()",
        "new EventTarget().removeEventListener('x',7)",
        "new EventTarget().addEventListener('x',null,{signal:null})",
        "new EventTarget().addEventListener('x',()=>{},{signal:{}})",
        "Object.getOwnPropertyDescriptor(CustomEvent.prototype,'detail').get.call(new Event('x'))",
        "Event.prototype.type",
        "Object.getOwnPropertyDescriptor(Event.prototype,'returnValue').set.call({})",
        "new Event('x').initEvent()",
        "DOMException.prototype.code",
    ] {
        assert!(
            matches!(r.evaluate(source), Err(Error::Type { .. })),
            "{source}"
        );
    }
    check(
        "let t=new EventTarget(),e=new Event('x',{cancelable:true});t.dispatchEvent(e);e.returnValue=false;e.returnValue=true;!e.returnValue&&e.defaultPrevented",
    )?;
    let mut h = SilentHost;
    let mut r = Realm::new(
        Limits {
            properties: 40,
            ..Limits::default()
        },
        &mut h,
    )?;
    r.install_events()?;
    assert!(matches!(
        r.evaluate("let t=new EventTarget();for(let i=0;i<41;i++)t.addEventListener('x',()=>{})"),
        Err(Error::Limit {
            resource: "event listeners"
        })
    ));
    let mut h = BadClock;
    let mut r = Realm::new(Limits::default(), &mut h)?;
    r.install_events()?;
    assert_eq!(r.evaluate("new Event('x')"), Err(Error::Host));
    Ok(())
}
