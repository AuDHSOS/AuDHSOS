// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use crate::{Error, Host, Limits, Realm, SilentHost, Value};
use std::{cell::RefCell, rc::Rc};

#[derive(Default)]
struct State {
    calls: Vec<(u32, Value, Vec<Value>)>,
    returned: Option<Value>,
    throw: bool,
    fatal: bool,
}
struct TestHost(Rc<RefCell<State>>);
impl Host for TestHost {
    fn print(&mut self, args: &[Value]) -> Result<(), Error> {
        self.0
            .borrow_mut()
            .calls
            .push((0, Value::Undefined, args.to_vec()));
        Ok(())
    }
    fn call(&mut self, id: u32, receiver: &Value, args: &[Value]) -> Result<Value, Error> {
        let mut state = self.0.borrow_mut();
        state.calls.push((id, receiver.clone(), args.to_vec()));
        if state.fatal {
            return Err(Error::Host);
        }
        let value = state
            .returned
            .clone()
            .unwrap_or_else(|| args.first().cloned().unwrap_or(Value::Undefined));
        if state.throw {
            Err(Error::Thrown { value })
        } else {
            Ok(value)
        }
    }
}

#[test]
fn installed_host_functions_have_identity_properties_and_exact_receivers() -> Result<(), Error> {
    let state = Rc::new(RefCell::new(State::default()));
    let mut host = TestHost(state.clone());
    let mut realm = Realm::new(Limits::default(), &mut host)?;
    let f = realm.host_function(7, "echo", 1)?;
    let g = realm.host_function(7, "echo", 1)?;
    assert_ne!(f, g);
    realm.set_global("echo", &f)?;
    assert_eq!(realm.evaluate("echo(4)")?, Value::Number(4.0));
    assert_eq!(
        state.borrow().calls.last().map(|(_, r, _)| r.clone()),
        Some(Value::Undefined)
    );
    assert_eq!(realm.evaluate("echo.call(3,8)")?, Value::Number(8.0));
    assert_eq!(
        state.borrow().calls.last().map(|(_, r, _)| r.clone()),
        Some(Value::Number(3.0))
    );
    assert_eq!(realm.evaluate("echo.apply(null,[9])")?, Value::Number(9.0));
    assert_eq!(realm.evaluate("echo.bind(true,5)()")?, Value::Number(5.0));
    assert_eq!(
        state.borrow().calls.last().map(|(_, r, _)| r.clone()),
        Some(Value::Boolean(true))
    );
    assert_eq!(realm.evaluate("echo.name==='echo'&&echo.length===1&&!Object.hasOwn(echo,'prototype')&&Object.getPrototypeOf(echo)===Function.prototype")?,Value::Boolean(true));
    assert_eq!(realm.evaluate("let d=Object.getOwnPropertyDescriptor(echo,'length');!d.writable&&!d.enumerable&&d.configurable")?,Value::Boolean(true));
    assert_eq!(
        realm.evaluate("echo.extra=4;echo.extra")?,
        Value::Number(4.0)
    );
    assert_eq!(
        realm.evaluate("String(echo)")?,
        Value::string("function echo() { [native code] }")
    );
    assert!(matches!(
        realm.evaluate("new echo()"),
        Err(Error::Type { .. })
    ));
    assert_eq!(
        realm.evaluate("Object.freeze(echo);echo.extra=9;echo.extra")?,
        Value::Number(4.0)
    );
    Ok(())
}

#[test]
fn later_host_turn_calls_retained_callbacks_and_drains_jobs() -> Result<(), Error> {
    let state = Rc::new(RefCell::new(State::default()));
    let mut host = TestHost(state.clone());
    let mut realm = Realm::new(
        Limits {
            heap_entries: 140,
            ..Limits::default()
        },
        &mut host,
    )?;
    let enqueue = realm.host_function(1, "enqueue", 1)?;
    realm.set_global("enqueue", &enqueue)?;
    realm.evaluate("let log='';enqueue(function(x){log+='c'+x;Promise.resolve().then(()=>log+='j');return this.n});undefined")?;
    let callback = state
        .borrow()
        .calls
        .last()
        .and_then(|(_, _, args)| args.first())
        .cloned()
        .ok_or(Error::InvalidBytecode)?;
    realm.evaluate("for(let i=0;i<300;i++){let x={}}")?;
    let receiver = realm.object()?;
    realm.set(&receiver, &Value::string("n"), &Value::Number(7.0))?;
    assert_eq!(
        realm.call(&callback, &receiver, &[Value::Number(2.0)])?,
        Value::Number(7.0)
    );
    assert_eq!(realm.get_global("log")?, Value::string("c2j"));
    let async_f = realm.evaluate("async function f(){await 1;return 9}f")?;
    let promise = realm.call(&async_f, &Value::Undefined, &[])?;
    realm.set_global("p", &promise)?;
    realm.evaluate("p.then(v=>{if(v!==9)throw 3})")?;
    Ok(())
}

#[test]
fn host_property_operations_execute_hooks_and_keep_pending_promises() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut realm = Realm::new(Limits::default(), &mut host)?;
    let obj = realm
        .evaluate("let n=0;({get x(){Promise.resolve().then(()=>n++);return 3},set x(v){n=v}})")?;
    assert_eq!(realm.get(&obj, &Value::string("x"))?, Value::Number(3.0));
    assert_eq!(realm.get_global("n")?, Value::Number(1.0));
    realm.set(&obj, &Value::string("x"), &Value::Number(8.0))?;
    assert_eq!(realm.get_global("n")?, Value::Number(8.0));
    let symbol = realm.evaluate("Symbol('key')")?;
    realm.set(&obj, &symbol, &Value::Number(9.0))?;
    assert_eq!(realm.get(&obj, &symbol)?, Value::Number(9.0));
    let cap=realm.evaluate("let cap=Promise.withResolvers();async function suspended(){let v=await cap.promise;n=v} suspended();cap")?;
    let resolve = realm.get(&cap, &Value::string("resolve"))?;
    realm.call(&resolve, &Value::Undefined, &[Value::Number(42.0)])?;
    assert_eq!(realm.get_global("n")?, Value::Number(42.0));
    Ok(())
}

#[test]
fn foreign_and_stale_values_are_rejected_without_heap_aliasing() -> Result<(), Error> {
    let mut a = SilentHost;
    let mut b = SilentHost;
    let mut first = Realm::new(Limits::default(), &mut a)?;
    let mut second = Realm::new(Limits::default(), &mut b)?;
    for source in [
        "({x:1})",
        "(()=>1)",
        "Symbol('x')",
        "(function(){}).bind(null)",
        "Promise.withResolvers().resolve",
    ] {
        let value = first.evaluate(source)?;
        assert!(
            matches!(
                second.set_global("foreign", &value),
                Err(Error::Type { .. })
            ),
            "{source}"
        );
        assert!(
            matches!(
                second.get(&value, &Value::string("x")),
                Err(Error::Type { .. })
            ),
            "{source}"
        );
        assert!(
            matches!(second.release(&value), Err(Error::Type { .. })),
            "{source}"
        );
        assert_eq!(second.evaluate("1")?, Value::Number(1.0));
    }
    let mut host = SilentHost;
    let mut realm = Realm::new(
        Limits {
            heap_entries: 100,
            ..Limits::default()
        },
        &mut host,
    )?;
    let old = realm.object()?;
    realm.release(&old)?;
    realm.evaluate("for(let i=0;i<300;i++){let x={}}")?;
    assert!(matches!(
        realm.get(&old, &Value::string("x")),
        Err(Error::Type { .. })
    ));
    assert_eq!(realm.evaluate("2")?, Value::Number(2.0));
    Ok(())
}

#[test]
fn host_errors_and_returned_values_follow_language_and_embedding_boundaries() -> Result<(), Error> {
    let state = Rc::new(RefCell::new(State::default()));
    let mut host = TestHost(state.clone());
    let mut realm = Realm::new(Limits::default(), &mut host)?;
    let f = realm.host_function(1, "host", 1)?;
    realm.set_global("host", &f)?;
    let object = realm.object()?;
    state.borrow_mut().returned = Some(object.clone());
    assert_eq!(realm.evaluate("host()")?, object);
    state.borrow_mut().throw = true;
    assert_eq!(realm.evaluate("try{host()}catch(e){e}")?, object);
    let throwing =
        realm.evaluate("(()=>{Promise.resolve().then(()=>globalThis.ran=7);throw 3})")?;
    assert_eq!(
        realm.call(&throwing, &Value::Undefined, &[]),
        Err(Error::Thrown {
            value: Value::Number(3.0)
        })
    );
    assert_eq!(realm.get_global("ran")?, Value::Number(7.0));
    state.borrow_mut().fatal = true;
    assert_eq!(realm.evaluate("try{host()}catch(e){}"), Err(Error::Host));
    assert!(realm.object().is_err());
    assert!(realm.get_global("host").is_err());
    assert!(realm.release(&f).is_err());
    Ok(())
}

#[test]
fn foreign_host_return_and_throw_poison_the_realm() -> Result<(), Error> {
    let mut foreign_host = SilentHost;
    let mut foreign_realm = Realm::new(Limits::default(), &mut foreign_host)?;
    let foreign = foreign_realm.object()?;
    for throw in [false, true] {
        let state = Rc::new(RefCell::new(State {
            returned: Some(foreign.clone()),
            throw,
            ..State::default()
        }));
        let mut host = TestHost(state);
        let mut realm = Realm::new(Limits::default(), &mut host)?;
        let f = realm.host_function(1, "bad", 0)?;
        realm.set_global("bad", &f)?;
        assert_eq!(realm.evaluate("try{bad()}catch(e){3}"), Err(Error::Host));
        assert!(realm.evaluate("1").is_err());
    }
    Ok(())
}

#[test]
fn host_functions_participate_in_gc_properties_and_weakmap_identity() -> Result<(), Error> {
    let state = Rc::new(RefCell::new(State::default()));
    let mut host = TestHost(state.clone());
    let mut realm = Realm::new(
        Limits {
            heap_entries: 130,
            ..Limits::default()
        },
        &mut host,
    )?;
    let f = realm.host_function(1, "echo", 1)?;
    realm.set_global("echo", &f)?;
    realm.evaluate("let map=new WeakMap();map.set(echo,{n:7});echo[Symbol('x')]={n:8};for(let i=0;i<300;i++){let x={}}")?;
    assert_eq!(
        realm.evaluate("map.get(echo).n+echo[Object.getOwnPropertySymbols(echo)[0]].n")?,
        Value::Number(15.0)
    );
    realm.evaluate("print(()=>7);undefined")?;
    let callback = state
        .borrow()
        .calls
        .last()
        .and_then(|(_, _, args)| args.first())
        .cloned()
        .ok_or(Error::InvalidBytecode)?;
    realm.evaluate("for(let i=0;i<300;i++){let x={}}")?;
    assert_eq!(
        realm.call(&callback, &Value::Undefined, &[])?,
        Value::Number(7.0)
    );
    Ok(())
}

#[test]
fn embedding_quotas_and_unimplemented_host_calls_are_explicit() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut realm = Realm::new(Limits::default(), &mut host)?;
    let f = realm.host_function(4, "missing", 0)?;
    assert_eq!(realm.call(&f, &Value::Undefined, &[]), Err(Error::Host));
    let mut host = SilentHost;
    let mut realm = Realm::new(
        Limits {
            stack: 8,
            ..Limits::default()
        },
        &mut host,
    )?;
    let f = realm.evaluate("(()=>1)")?;
    assert!(matches!(
        realm.call(&f, &Value::Undefined, &[const { Value::Undefined }; 7]),
        Err(Error::Limit { .. })
    ));
    let mut host = SilentHost;
    let mut realm = Realm::new(
        Limits {
            string_units: 64,
            ..Limits::default()
        },
        &mut host,
    )?;
    assert!(matches!(
        realm.set_global("long", &Value::string(&"x".repeat(65))),
        Err(Error::Limit { .. })
    ));
    Ok(())
}

#[test]
fn host_accessors_and_fallible_conversions_use_the_same_receiver_and_hooks() -> Result<(), Error> {
    let state = Rc::new(RefCell::new(State::default()));
    let mut host = TestHost(state.clone());
    let mut realm = Realm::new(Limits::default(), &mut host)?;
    let object = realm.object()?;
    let getter = realm.host_function(4, "get value", 0)?;
    let setter = realm.host_function(5, "set value", 1)?;
    let descriptor = realm.object()?;
    realm.set(&descriptor, &Value::string("get"), &getter)?;
    realm.set(&descriptor, &Value::string("set"), &setter)?;
    realm.set(
        &descriptor,
        &Value::string("configurable"),
        &Value::Boolean(true),
    )?;
    realm.define_property(&object, &Value::string("value"), &descriptor)?;
    state.borrow_mut().returned = Some(Value::Number(7.0));
    assert_eq!(
        realm.get(&object, &Value::string("value"))?,
        Value::Number(7.0)
    );
    assert_eq!(
        state
            .borrow()
            .calls
            .last()
            .map(|(id, r, _)| (*id, r.clone())),
        Some((4, object.clone()))
    );
    realm.set(&object, &Value::string("value"), &Value::Number(8.0))?;
    assert_eq!(
        state
            .borrow()
            .calls
            .last()
            .map(|(id, r, args)| (*id, r.clone(), args.clone())),
        Some((5, object.clone(), vec![Value::Number(8.0)]))
    );
    let convert = realm.evaluate("({toString(){return '27'},valueOf(){return 42}})")?;
    assert_eq!(realm.to_string(&convert)?, Value::string("27"));
    assert_eq!(realm.to_number(&convert)?, Value::Number(42.0));
    let symbol = realm.evaluate("Symbol('x')")?;
    assert!(matches!(realm.to_string(&symbol), Err(Error::Type { .. })));
    assert!(matches!(realm.to_number(&symbol), Err(Error::Type { .. })));
    let bad = realm.evaluate("({get value(){throw 9}})")?;
    assert_eq!(
        realm.define_property(&object, &Value::string("x"), &bad),
        Err(Error::Thrown {
            value: Value::Number(9.0)
        })
    );
    assert_eq!(realm.get(&object, &Value::string("x"))?, Value::Undefined);
    Ok(())
}

#[test]
fn host_function_reference_keys_and_gc_survive_throwing_getters() -> Result<(), Error> {
    let state = Rc::new(RefCell::new(State::default()));
    let mut host = TestHost(state);
    let mut realm = Realm::new(
        Limits {
            heap_entries: 140,
            ..Limits::default()
        },
        &mut host,
    )?;
    let object = realm.object()?;
    let key = realm.evaluate(
        "({[Symbol.toPrimitive](){for(let i=0;i<250;i++){let t={}}return Symbol('generated')}})",
    )?;
    realm.set(&object, &key, &Value::Number(4.0))?;
    realm.set_global("saved", &object)?;
    assert_eq!(
        realm.evaluate("saved[Object.getOwnPropertySymbols(saved)[0]]")?,
        Value::Number(4.0)
    );
    let callback = realm.evaluate("(function(){throw {n:7}})")?;
    let Err(Error::Thrown { value: reason }) = realm.call(&callback, &Value::Undefined, &[]) else {
        return Err(Error::InvalidBytecode);
    };
    realm.evaluate("for(let i=0;i<250;i++){let t={}}")?;
    assert_eq!(realm.get(&reason, &Value::string("n"))?, Value::Number(7.0));
    assert!(matches!(
        realm.call(&Value::Number(1.0), &Value::Undefined, &[]),
        Err(Error::Type { .. })
    ));
    assert_eq!(realm.evaluate("1")?, Value::Number(1.0));
    Ok(())
}

#[test]
fn stale_host_results_and_oversized_host_strings_are_fatal() -> Result<(), Error> {
    for throw in [false, true] {
        let state = Rc::new(RefCell::new(State::default()));
        let mut host = TestHost(state.clone());
        let mut realm = Realm::new(
            Limits {
                heap_entries: 110,
                ..Limits::default()
            },
            &mut host,
        )?;
        let f = realm.host_function(1, "bad", 0)?;
        let old = realm.object()?;
        realm.release(&old)?;
        realm.evaluate("for(let i=0;i<250;i++){let t={}}")?;
        state.borrow_mut().returned = Some(old);
        state.borrow_mut().throw = throw;
        assert_eq!(realm.call(&f, &Value::Undefined, &[]), Err(Error::Host));
    }
    let state = Rc::new(RefCell::new(State {
        returned: Some(Value::string(&"x".repeat(65))),
        ..State::default()
    }));
    let mut host = TestHost(state);
    let mut realm = Realm::new(
        Limits {
            string_units: 64,
            ..Limits::default()
        },
        &mut host,
    )?;
    let f = realm.host_function(1, "bad", 0)?;
    assert_eq!(realm.call(&f, &Value::Undefined, &[]), Err(Error::Host));
    Ok(())
}

#[test]
fn retained_quota_release_and_name_limits_are_enforced() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut realm = Realm::new(
        Limits {
            properties: 32,
            ..Limits::default()
        },
        &mut host,
    )?;
    for _ in 0..64 {
        let object = realm.object()?;
        realm.release(&object)?;
    }
    let object = realm.object()?;
    for _ in 0..64 {
        realm.release(&Value::Undefined)?;
        realm.set_global("same", &object)?;
        realm.get_global("same")?;
    }
    for _ in 0..31 {
        realm.object()?;
    }
    assert!(matches!(
        realm.object(),
        Err(Error::Limit {
            resource: "retained realm results"
        })
    ));
    for getter in [false, true] {
        let mut host = SilentHost;
        let mut realm = Realm::new(
            Limits {
                string_units: 32,
                ..Limits::default()
            },
            &mut host,
        )?;
        let name = "x".repeat(33);
        let result = if getter {
            realm.get_global(&name)
        } else {
            realm.host_function(1, &name, 0)
        };
        assert!(matches!(result, Err(Error::Limit { .. })));
    }
    Ok(())
}

#[test]
fn wrong_kinds_and_foreign_host_functions_cannot_alias_existing_nodes() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut realm = Realm::new(Limits::default(), &mut host)?;
    let function = realm.evaluate("(()=>3)")?;
    let Value::Function(crate::FunctionValue(crate::value::Callable::Script { handle, owner })) =
        function
    else {
        return Err(Error::InvalidBytecode);
    };
    // Only internal tests can forge a mismatched opaque type. The boundary
    // checks the tag as well as owner/generation before accepting it.
    let forged = Value::Object(crate::ObjectValue { handle, owner });
    assert!(matches!(
        realm.set_global("bad", &forged),
        Err(Error::Type { .. })
    ));
    assert_eq!(realm.evaluate("1")?, Value::Number(1.0));
    let mut foreign_host = SilentHost;
    let mut foreign = Realm::new(Limits::default(), &mut foreign_host)?;
    let function = foreign.host_function(5, "foreign", 0)?;
    assert!(matches!(
        realm.call(&function, &Value::Undefined, &[]),
        Err(Error::Type { .. })
    ));
    Ok(())
}
