// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::{Error, Value, eval};

#[test]
fn math_sin_special_values_coercion_and_approximation() {
    for source in [
        "Math.sin()",
        "Math.sin(NaN)",
        "Math.sin(Infinity)",
        "Math.sin(-Infinity)",
    ] {
        assert!(
            matches!(eval(source), Ok(Value::Number(value)) if value.is_nan()),
            "{source}"
        );
    }
    for (source, bits) in [
        ("Math.sin(0)", 0.0f64.to_bits()),
        ("Math.sin(-0)", (-0.0f64).to_bits()),
    ] {
        assert!(
            matches!(eval(source), Ok(Value::Number(value)) if value.to_bits() == bits),
            "{source}"
        );
    }
    for source in [
        "Math.abs(Math.sin(Math.PI/6)-0.5)<1e-15",
        "Math.abs(Math.sin(Math.PI/2)-1)<1e-15",
        "Math.abs(Math.sin(2*Math.PI))<1e-15",
        "Math.sin('0.5')===Math.sin(0.5)",
        "let n=0;Math.sin({valueOf(){n++;return 0}})===0&&n===1",
        "let log='';Math.sin({valueOf(){log+='v';return {}},toString(){log+='s';return '0'}})===0&&log==='vs'",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    assert!(matches!(
        eval("Math.sin(Symbol())"),
        Err(Error::Type { .. })
    ));
}

#[test]
fn math_sin_is_an_ordinary_mutable_nonconstructor_function() {
    for source in [
        "Math.sin.length===1&&Math.sin.name==='sin'&&Object.isExtensible(Math.sin)",
        "let d=Object.getOwnPropertyDescriptor(Math,'sin');d.writable&&!d.enumerable&&d.configurable",
        "Math.sin.prototype===undefined&&Object.getPrototypeOf(Math.sin)===Function.prototype",
        "let f=Math.sin;f.x=7;delete f.name;f.x===7&&f(0)===0",
        "Object.getOwnPropertyNames(Math.sin).join()==='length,name'",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    assert!(matches!(eval("new Math.sin(0)"), Err(Error::Type { .. })));
}
