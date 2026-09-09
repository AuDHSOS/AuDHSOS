// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use crate::{Error, Host, Limits, Runtime, Value, compile};

#[derive(Default)]
struct Output(Vec<String>);
impl Host for Output {
    fn print(&mut self, args: &[Value]) -> Result<(), Error> {
        self.0.push(
            args.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" "),
        );
        Ok(())
    }
}
fn output(source: &str) -> Result<Vec<String>, Error> {
    let limits = Limits::default();
    let mut host = Output::default();
    Runtime::new(limits).run(&compile(source, limits)?, &mut host)?;
    Ok(host.0)
}

#[test]
fn reactions_are_fifo_jobs_and_executors_are_synchronous() {
    for (source, expected) in [
        (
            "Promise.resolve(42).then(print);print('sync');",
            vec!["sync", "42"],
        ),
        (
            "new Promise((resolve,reject)=>{print('executor');resolve(42);reject(1);}).then(print);print('sync');",
            vec!["executor", "sync", "42"],
        ),
        (
            "let p=Promise.resolve(1);p.then(x=>{print('a');p.then(()=>print('c'));});p.then(()=>print('b'));",
            vec!["a", "b", "c"],
        ),
        ("Promise.resolve(40).then(x=>x+2).then(print);", vec!["42"]),
        (
            "Promise.reject(42).catch(print);print('sync');",
            vec!["sync", "42"],
        ),
        (
            "Promise.resolve(1).then(()=>{throw 42;}).catch(print);",
            vec!["42"],
        ),
        (
            "let p=Promise.resolve(42);print(Promise.resolve(p)===p);p.then(1).then(print);",
            vec!["true", "42"],
        ),
        ("Promise.reject(42).then().catch(print);", vec!["42"]),
        ("new Promise(()=>{throw 42;}).catch(print);", vec!["42"]),
        (
            "let r=Promise.withResolvers();r.promise.then(print);r.resolve(42);r.reject(1);",
            vec!["42"],
        ),
    ] {
        assert_eq!(
            output(source),
            Ok(expected.into_iter().map(str::to_owned).collect()),
            "{source}"
        );
    }
}

#[test]
fn thenable_assimilation_and_resolution_latches() {
    for (source, expected) in [
        (
            "let t={get then(){print('get');return function(resolve){print('then');resolve(42);};}};Promise.resolve(t).then(print);print('sync');",
            vec!["get", "sync", "then", "42"],
        ),
        (
            "Promise.resolve({then(r,j){r(42);j(1);throw 2;}}).then(print,()=>print('bad'));",
            vec!["42"],
        ),
        (
            "Promise.resolve({get then(){throw 42;}}).catch(print);",
            vec!["42"],
        ),
        (
            "let a=Promise.withResolvers();let b=Promise.withResolvers();a.resolve(b.promise);a.reject(1);a.promise.then(print);b.resolve(42);",
            vec!["42"],
        ),
        (
            "let r=Promise.withResolvers();r.promise.catch(e=>print(e.name));r.resolve(r.promise);",
            vec!["TypeError"],
        ),
        (
            "let p=Promise.resolve(1);let q=p.then(()=>q);q.catch(e=>print(e.name));",
            vec!["TypeError"],
        ),
        (
            "Promise.resolve({then:42}).then(x=>print(x.then));",
            vec!["42"],
        ),
    ] {
        assert_eq!(
            output(source),
            Ok(expected.into_iter().map(str::to_owned).collect()),
            "{source}"
        );
    }
}

#[test]
fn async_functions_suspend_on_await_and_return_promises() {
    for (source, expected) in [
        (
            "async function f(){print('start');let x=await 40;print(x+2);return x+2;}f().then(print);print('sync');",
            vec!["start", "sync", "42", "42"],
        ),
        (
            "async function f(){return 42;}f().then(print);print('sync');",
            vec!["sync", "42"],
        ),
        (
            "async function f(){throw 42;}f().catch(print);print('sync');",
            vec!["sync", "42"],
        ),
        (
            "(async x=>{return (await x)+2;})(40).then(print);",
            vec!["42"],
        ),
        (
            "(async function(x){return await x;})(42).then(print);",
            vec!["42"],
        ),
        (
            "async function f(){return (await 20)+(await 22);}f().then(print);",
            vec!["42"],
        ),
        (
            "async function f(){try{return await Promise.reject(1);}catch(e){return e+41;}}f().then(print);",
            vec!["42"],
        ),
        (
            "async function f(){try{return await 1;}finally{return await 42;}}f().then(print);",
            vec!["42"],
        ),
        (
            "let r=Promise.withResolvers();async function f(){return await r.promise;}f().then(print);print('sync');r.resolve(42);",
            vec!["sync", "42"],
        ),
        (
            "async function f(a=(()=>{throw 42;})()){}f().catch(print);",
            vec!["42"],
        ),
        (
            "async function f(){let s=0;for(let x of [10,20,12]){s+=await x;}return s;}f().then(print);",
            vec!["42"],
        ),
    ] {
        assert_eq!(
            output(source),
            Ok(expected.into_iter().map(str::to_owned).collect()),
            "{source}"
        );
    }
}

#[test]
fn promise_usage_errors_and_limits_are_explicit() {
    for source in [
        "Promise(()=>{})",
        "new Promise(1)",
        "Promise.prototype.then.call({})",
    ] {
        assert!(
            matches!(output(source), Err(Error::Type { .. })),
            "{source}"
        );
    }
    for source in [
        "await 1",
        "async\nfunction f(){await 1;}",
        "async function f(){function g(){await 1;}}",
    ] {
        assert!(compile(source, Limits::default()).is_err(), "{source}");
    }
    let limits = Limits {
        fuel: 500,
        ..Limits::default()
    };
    let mut host = Output::default();
    let source = "function again(){Promise.resolve().then(again);}again();";
    let result = compile(source, limits).and_then(|p| Runtime::new(limits).run(&p, &mut host));
    assert!(matches!(result, Err(Error::Limit { .. })));
}

#[test]
fn promise_gc_preserves_pending_reactions_and_suspended_execution() -> Result<(), Error> {
    let limits = Limits {
        heap_entries: 80,
        ..Limits::default()
    };
    for source in [
        "let r=Promise.withResolvers();async function f(){let o={x:42};await r.promise;return o.x;}f().then(print);for(let i=0;i<300;i++){let o={};o.o=o;}r.resolve(1);",
        "async function f(){try{return {x:42};}finally{await 1;for(let i=0;i<300;i++){let o={};o.o=o;}}}f().then(o=>print(o.x));",
        "async function f(){let g;{let x=42;g=()=>x;}await 1;for(let i=0;i<300;i++){let o={};o.o=o;}return g();}f().then(print);",
        "let r=Promise.withResolvers();let o={x:42};r.promise.then(x=>print(x.x));r.resolve(o);o=null;for(let i=0;i<300;i++){let o={};o.o=o;}",
    ] {
        let mut host = Output::default();
        Runtime::new(limits).run(&compile(source, limits)?, &mut host)?;
        assert_eq!(host.0, vec!["42"], "{source}");
    }
    Ok(())
}

#[test]
fn rejection_checkpoint_and_async_native_boundaries() {
    assert_eq!(
        output("Promise.reject(42);"),
        Err(Error::Thrown {
            value: Value::Number(42.0)
        })
    );
    assert_eq!(
        output("Promise.reject(42).catch(print);"),
        Ok(vec!["42".to_owned()])
    );
    for (source, expected) in [
        (
            "let o={x:42,async f(){return await this.x;}};o.f().then(print);",
            vec!["42"],
        ),
        (
            "[1].forEach(async x=>{print('start');print(await x);});print('sync');",
            vec!["start", "sync", "1"],
        ),
        (
            "let o={get x(){async function f(){return await 42;}return f();}};o.x.then(print);",
            vec!["42"],
        ),
        (
            "async function f(){await 1;try{throw 2;}finally{await 3;}}f().catch(print);",
            vec!["2"],
        ),
        (
            "async function f(){for(let i=0;i<3;i++){try{if(i==1)continue;if(i==2)break;}finally{print(await i);}}}f();",
            vec!["0", "1", "2"],
        ),
        (
            "async function f(){return await /a/.test('a');}f().then(print);",
            vec!["true"],
        ),
    ] {
        assert_eq!(
            output(source),
            Ok(expected.into_iter().map(str::to_owned).collect()),
            "{source}"
        );
    }
    for source in [
        "async function f(a=await 1){}",
        "async function f(){function g(a=await 1){}}",
        "async (a=await 1)=>a",
    ] {
        assert!(compile(source, Limits::default()).is_err(), "{source}");
    }
}

#[test]
fn job_reaction_and_suspension_limits_are_enforced() -> Result<(), Error> {
    for (source, limits) in [
        (
            "let p=Promise.resolve(1);p.then(()=>1);p.then(()=>2);",
            Limits {
                jobs: 1,
                ..Limits::default()
            },
        ),
        (
            "let p=new Promise(()=>{});p.then(()=>1);p.then(()=>2);",
            Limits {
                jobs: 1,
                ..Limits::default()
            },
        ),
        (
            "let p=new Promise(()=>{});async function f(){await p;}for(let i=0;i<20;i++)f();",
            Limits {
                call_frames: 4,
                ..Limits::default()
            },
        ),
    ] {
        let mut host = Output::default();
        assert!(
            matches!(
                Runtime::new(limits).run(&compile(source, limits)?, &mut host),
                Err(Error::Limit { .. })
            ),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn unreachable_pending_async_cycles_release_frame_quotas() -> Result<(), Error> {
    let limits = Limits {
        heap_entries: 64,
        call_frames: 6,
        ..Limits::default()
    };
    let source =
        "async function f(){await new Promise(()=>{});}for(let i=0;i<100;i++)f();print(42);";
    let mut host = Output::default();
    Runtime::new(limits).run(&compile(source, limits)?, &mut host)?;
    assert_eq!(host.0, vec!["42"]);
    Ok(())
}

#[test]
fn finally_and_combinators_observe_jobs_and_preserve_values() {
    for (source, expected) in [
        (
            "Promise.resolve(42).finally(()=>print('finally')).then(print);print('sync');",
            vec!["sync", "finally", "42"],
        ),
        (
            "Promise.reject(42).finally(()=>1).catch(print);",
            vec!["42"],
        ),
        (
            "Promise.resolve(1).finally(()=>Promise.resolve(2)).then(print);",
            vec!["1"],
        ),
        (
            "Promise.resolve(1).finally(()=>{throw 42;}).catch(print);",
            vec!["42"],
        ),
        ("Promise.resolve(42).finally(1).then(print);", vec!["42"]),
        (
            "Promise.all([Promise.resolve(1),2,Promise.resolve(3)]).then(a=>print(a.join()));",
            vec!["1,2,3"],
        ),
        (
            "let a=Promise.withResolvers();let b=Promise.withResolvers();Promise.all([a.promise,b.promise]).then(a=>print(a.join()));b.resolve(2);a.resolve(1);",
            vec!["1,2"],
        ),
        (
            "Promise.all([]).then(a=>print(a.length));print('sync');",
            vec!["sync", "0"],
        ),
        (
            "Promise.all([Promise.reject(42),1]).catch(print);",
            vec!["42"],
        ),
        (
            "Promise.allSettled([1,Promise.reject(42)]).then(a=>print(a[0].status,a[0].value,a[1].status,a[1].reason));",
            vec!["fulfilled 1 rejected 42"],
        ),
        (
            "Promise.race([Promise.resolve(42),Promise.reject(1)]).then(print);",
            vec!["42"],
        ),
        (
            "Promise.race([]).then(()=>print('bad'));print('sync');",
            vec!["sync"],
        ),
        (
            "Promise.all(null).catch(e=>print(e.name));",
            vec!["TypeError"],
        ),
    ] {
        assert_eq!(
            output(source),
            Ok(expected.into_iter().map(str::to_owned).collect()),
            "{source}"
        );
    }
}

#[test]
fn await_preserves_operands_this_and_repeated_suspension() {
    for (source, expected) in [
        (
            "async function f(){let o={x:40};return o.x+(await 2);}f().then(print);",
            vec!["42"],
        ),
        (
            "async function f(){return {x:40}.x + (await 1) + (await 1);}f().then(print);",
            vec!["42"],
        ),
        (
            "let o={x:42,async f(){await 0;return (()=>this.x)();}};o.f().then(print);",
            vec!["42"],
        ),
        (
            "async function f(){let s=0;for(let i=0;i<50;i++)s+=await 1;return s;}f().then(print);",
            vec!["50"],
        ),
        (
            "async function f(){try{throw 42;}finally{await 0;}}f().catch(print);",
            vec!["42"],
        ),
        (
            "async function f(){try{await Promise.reject(1);}finally{await Promise.reject(42);}}f().catch(print);",
            vec!["42"],
        ),
        (
            "async function f(){await 0;missing;}f().catch(e=>print(e.name));",
            vec!["ReferenceError"],
        ),
        (
            "Promise.reject(1).finally(()=>Promise.reject(42)).catch(print);",
            vec!["42"],
        ),
        (
            "let r=Promise.withResolvers();print(r.resolve===r.resolve,r.resolve===r.reject);r.reject(42);r.resolve(1);r.promise.catch(print);",
            vec!["true false", "42"],
        ),
        (
            "let a=[];for(let i=0;i<3;i++){a.push(async()=>await i);}Promise.all(a.map(f=>f())).then(a=>print(a.join()));",
            vec!["0,1,2"],
        ),
    ] {
        assert_eq!(
            output(source),
            Ok(expected.into_iter().map(str::to_owned).collect()),
            "{source}"
        );
    }
    assert!(compile("async function f(a,a){}", Limits::default()).is_err());
    assert!(matches!(
        output("new (async function(){})"),
        Err(Error::Type { .. })
    ));
}

#[test]
fn host_checkpoint_policy_and_callback_failures_are_observable() -> Result<(), Error> {
    struct HostPolicy(Vec<Value>);
    impl Host for HostPolicy {
        fn print(&mut self, _: &[Value]) -> Result<(), Error> {
            Err(Error::Host)
        }
        fn unhandled_rejection(&mut self, reason: &Value) -> Result<(), Error> {
            self.0.push(reason.clone());
            Ok(())
        }
    }
    let limits = Limits::default();
    let mut host = HostPolicy(Vec::new());
    let mut runtime = Runtime::new(limits);
    runtime.run(&compile("Promise.reject(42);", limits)?, &mut host)?;
    assert_eq!(host.0, vec![Value::Number(42.0)]);
    assert_eq!(
        runtime.run(
            &compile("Promise.resolve(1).then(print).catch(()=>0)", limits)?,
            &mut host
        ),
        Err(Error::Host)
    );
    assert!(runtime.run(&compile("let p=new Promise(()=>{});async function f(){return 1+(await p);}for(let i=0;i<100;i++)f();",limits)?,&mut host).is_ok());
    assert_eq!(
        output("let p=Promise.reject(42);Promise.resolve().then(()=>p.catch(print));"),
        Ok(vec!["42".to_owned()])
    );
    Ok(())
}
