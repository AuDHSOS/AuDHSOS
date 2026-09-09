// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use crate::{Error, Host, Limits, Realm, SilentHost, Value, compile_script};
#[test]
fn compiled_scripts_preserve_phase_isolation_and_limit_contract() -> Result<(), Error> {
    let limits = Limits::default();
    let script = compile_script("var n=7;let x=1;n+x", limits)?;
    for _ in 0..2 {
        let mut host = SilentHost;
        let mut realm = Realm::new(limits, &mut host)?;
        assert_eq!(realm.evaluate_compiled(&script)?, Value::Number(8.0));
        assert!(matches!(
            realm.evaluate_compiled(&script),
            Err(Error::Syntax { .. })
        ));
        assert_eq!(realm.evaluate("n")?, Value::Number(7.0));
    }
    let mut host = SilentHost;
    let mut realm = Realm::new(
        Limits {
            fuel: 1000,
            ..limits
        },
        &mut host,
    )?;
    assert!(matches!(
        realm.evaluate_compiled(&script),
        Err(Error::Type { .. })
    ));
    assert_eq!(realm.evaluate("1")?, Value::Number(1.0));
    assert!(matches!(
        compile_script("let =", limits),
        Err(Error::Syntax { .. })
    ));
    Ok(())
}
#[test]
fn explicit_gc_and_string_print_are_real_host_capabilities() -> Result<(), Error> {
    struct Record(Vec<Value>);
    impl Host for Record {
        fn print(&mut self, args: &[Value]) -> Result<(), Error> {
            self.0.extend_from_slice(args);
            Ok(())
        }
    }
    let mut host = Record(Vec::new());
    {
        let mut realm = Realm::new(
            Limits {
                heap_entries: 100,
                ..Limits::default()
            },
            &mut host,
        )?;
        let print = realm.string_print_function()?;
        let gc = realm.gc_function()?;
        realm.set_global("print", &print)?;
        realm.set_global("gc", &gc)?;
        realm.evaluate("let o={x:7};for(let i=0;i<20;i++){let t={};gc()}print({toString(){gc();return String(o.x)}},'ignored');print()")?;
        assert!(matches!(
            realm.evaluate("print(Symbol())"),
            Err(Error::Type { .. })
        ));
        assert_eq!(realm.evaluate("o.x")?, Value::Number(7.0));
    }
    assert_eq!(host.0, vec![Value::string("7"), Value::string("undefined")]);
    Ok(())
}

#[test]
fn compiled_execution_poison_and_print_host_errors_remain_fatal() -> Result<(), Error> {
    struct Bad;
    impl Host for Bad {
        fn print(&mut self, _: &[Value]) -> Result<(), Error> {
            Err(Error::Host)
        }
    }
    let limits = Limits {
        fuel: 1000,
        ..Limits::default()
    };
    let script = compile_script("while(true){}", limits)?;
    let mut host = SilentHost;
    let mut r = Realm::new(limits, &mut host)?;
    assert!(matches!(
        r.evaluate_compiled(&script),
        Err(Error::Limit { .. })
    ));
    assert!(r.evaluate_compiled(&script).is_err());
    let mut host = Bad;
    let mut r = Realm::new(Limits::default(), &mut host)?;
    let p = r.string_print_function()?;
    r.set_global("print", &p)?;
    assert_eq!(r.evaluate("print(7)"), Err(Error::Host));
    assert!(
        !Error::Unsupported { feature: "test" }
            .to_string()
            .is_empty()
    );
    Ok(())
}
