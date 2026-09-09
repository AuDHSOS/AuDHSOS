// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use super::{Builtin, Value, integral, predicate};
use crate::{Error, Limits, Realm, Runtime, SilentHost, compile};

fn eval(source: &str) -> Result<Value, Error> {
    let limits = Limits::default();
    Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)
}

#[test]
fn predicates_never_coerce_and_distinguish_wrappers_and_globals() {
    for name in ["isFinite", "isInteger", "isNaN", "isSafeInteger"] {
        for value in [
            "undefined",
            "null",
            "true",
            "false",
            "''",
            "'42'",
            "'NaN'",
            "{}",
            "[]",
            "Symbol()",
            "new Number(1)",
            "new Number(NaN)",
            "function(){}",
        ] {
            let source = format!("Number.{name}({value})===false");
            assert_eq!(eval(&source), Ok(Value::Boolean(true)), "{source}");
        }
        let source = format!(
            "let n=0,o={{valueOf(){{n++;throw 7}},toString(){{n++;throw 7}},[Symbol.toPrimitive](){{n++;throw 7}}}};Number.{name}(o)===false&&n===0&&Number.{name}()===false"
        );
        assert_eq!(eval(&source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "Number.isFinite(0)&&Number.isFinite(-0)&&Number.isFinite(0.1)&&Number.isFinite(Number.MAX_VALUE)&&!Number.isFinite(Infinity)&&!Number.isFinite(-Infinity)&&!Number.isFinite(NaN)",
        "Number.isNaN(NaN)&&Number.isNaN(0/0)&&!Number.isNaN(0)&&!Number.isNaN(Infinity)",
        "Number.isInteger(0)&&Number.isInteger(-0)&&Number.isInteger(-1)&&Number.isInteger(Number.MAX_VALUE)&&!Number.isInteger(Number.MIN_VALUE)&&!Number.isInteger(1.5)&&!Number.isInteger(NaN)&&!Number.isInteger(Infinity)",
        "Number.isSafeInteger(Number.MAX_SAFE_INTEGER)&&Number.isSafeInteger(Number.MIN_SAFE_INTEGER)&&!Number.isSafeInteger(Number.MAX_SAFE_INTEGER+1)&&!Number.isSafeInteger(Number.MIN_SAFE_INTEGER-1)&&!Number.isSafeInteger(Number.MAX_VALUE)",
        "Number.isSafeInteger(0)&&Number.isSafeInteger(-0)&&!Number.isSafeInteger(1.1)&&!Number.isSafeInteger(Infinity)&&!Number.isSafeInteger(NaN)",
        "Number.isNaN!==isNaN&&Number.isFinite!==isFinite&&!Number.isNaN('x')&&isNaN('x')&&!Number.isFinite(null)&&isFinite(null)",
        "let t={};Number.isInteger.call(t,7)&&Number.isFinite.apply(null,[7])&&Number.isSafeInteger.bind({},7)()&&Reflect.apply(Number.isNaN,{},[NaN])",
        "class N extends Number{}N.isSafeInteger(7)&&N.isInteger===Number.isInteger",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn bit_classification_agrees_with_arithmetic_oracle_for_arbitrary_binary64() {
    let mut state = 0x9178_ace3_328f_4781u64;
    for bits in [
        0,
        1,
        1 << 63,
        0x3ff0_0000_0000_0000,
        0x3fe0_0000_0000_0000,
        0x433f_ffff_ffff_ffff,
        0x4340_0000_0000_0000,
        0x7fef_ffff_ffff_ffff,
        0x7ff0_0000_0000_0000,
        0x7ff8_0000_0000_0000,
    ]
    .into_iter()
    .chain((0..20_000).map(|_| {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        state
    })) {
        let n = f64::from_bits(bits);
        let integer = n.is_finite() && n.fract() == 0.0;
        assert_eq!(integral(n), integer, "{n:?}");
        for (kind, expected) in [
            (Builtin::NumberIsInteger, integer),
            (
                Builtin::NumberIsSafeInteger,
                integer && n.abs() <= 9_007_199_254_740_991.0,
            ),
            (Builtin::NumberIsFinite, n.is_finite()),
            (Builtin::NumberIsNaN, n.is_nan()),
        ] {
            assert_eq!(
                predicate(kind, &Value::Number(n)),
                Value::Boolean(expected),
                "{n:?}"
            );
        }
    }
    assert_eq!(
        predicate(Builtin::Print, &Value::Number(1.0)),
        Value::Boolean(false)
    );
}

#[test]
fn predicate_function_metadata_is_mutable_and_gc_rooted() -> Result<(), Error> {
    for name in ["isFinite", "isInteger", "isNaN", "isSafeInteger"] {
        let source = format!(
            "let f=Number.{name},d=Object.getOwnPropertyDescriptor(Number,'{name}'),n=Object.getOwnPropertyDescriptor(f,'name'),l=Object.getOwnPropertyDescriptor(f,'length');f.name==='{name}'&&f.length===1&&d.writable&&!d.enumerable&&d.configurable&&!n.writable&&!n.enumerable&&n.configurable&&!l.writable&&!l.enumerable&&l.configurable&&f.prototype===undefined&&Object.getPrototypeOf(f)===Function.prototype"
        );
        assert_eq!(eval(&source), Ok(Value::Boolean(true)), "{source}");
        let source = format!(
            "let f=Number.{name};delete f.name;f.extra=7;Number.{name}=42;let ok=Number.{name}===42;delete Number.{name};ok&&f.extra===7&&!Object.hasOwn(f,'name')&&!Object.hasOwn(Number,'{name}')"
        );
        assert_eq!(eval(&source), Ok(Value::Boolean(true)), "{source}");
        assert!(matches!(
            eval(&format!("new Number.{name}()")),
            Err(Error::Type { .. })
        ));
    }
    let mut host = SilentHost;
    let mut realm = Realm::new(
        Limits {
            heap_entries: 180,
            ..Limits::default()
        },
        &mut host,
    )?;
    realm.evaluate("let f=Number.isInteger;f.saved={n:7};delete Number.isInteger;")?;
    assert_eq!(realm.evaluate("for(let i=0;i<300;i++){let g={}}f(7)&&f.saved.n===7&&!Object.hasOwn(Number,'isInteger')")?,Value::Boolean(true));
    Ok(())
}
