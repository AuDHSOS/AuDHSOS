// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::{Binary, Error, Execution, Limits, SilentHost, Value, binary};

const OPERATORS: &[Binary] = &[
    Binary::Add,
    Binary::Sub,
    Binary::Mul,
    Binary::Div,
    Binary::Rem,
    Binary::Lt,
    Binary::Le,
    Binary::Gt,
    Binary::Ge,
    Binary::Eq,
    Binary::Ne,
    Binary::StrictEq,
    Binary::StrictNe,
];

fn same_result(actual: &Value, expected: &Value) -> bool {
    match (actual, expected) {
        (Value::Number(a), Value::Number(b)) => {
            (a.is_nan() && b.is_nan()) || a.to_bits() == b.to_bits()
        }
        _ => actual == expected,
    }
}

fn compare(execution: &mut Execution<'_>, a: f64, b: f64) -> Result<(), Error> {
    for op in OPERATORS {
        let expected = binary(*op, &Value::Number(a), &Value::Number(b), execution.limits)?;
        execution.stack.clear();
        execution.stack.extend([
            Value::string("sentinel"),
            Value::Number(a),
            Value::Number(b),
        ]);
        let fuel = execution.fuel;
        let roots = execution.native_roots.len();
        assert!(execution.numeric_binary_in_place(*op)?);
        assert_eq!(execution.stack.len(), 2);
        assert_eq!(execution.stack.first(), Some(&Value::string("sentinel")));
        assert!(
            same_result(execution.stack.last().unwrap(), &expected),
            "{a:?} {op:?} {b:?}"
        );
        assert_eq!(
            execution.fuel, fuel,
            "dispatch, not arithmetic, charges opcode fuel"
        );
        assert_eq!(execution.native_roots.len(), roots);
    }
    Ok(())
}

#[test]
fn fast_path_matches_generic_binary_for_specials_and_generated_bit_patterns() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut e = Execution::new(&mut host, Limits::default());
    let specials = [
        0.0,
        -0.0,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        1.0,
        -1.0,
        f64::MIN_POSITIVE,
        f64::from_bits(1),
        -f64::from_bits(1),
        f64::MAX,
        -f64::MAX,
        9_007_199_254_740_991.0,
        9_007_199_254_740_992.0,
        1.0 + f64::EPSILON,
    ];
    for a in specials {
        for b in specials {
            compare(&mut e, a, b)?;
        }
    }
    let mut bits = 0x004a_5253_u64;
    for _ in 0..10_000 {
        bits = bits.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let a = f64::from_bits(bits);
        bits = bits.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let b = f64::from_bits(bits);
        compare(&mut e, a, b)?;
    }
    Ok(())
}

#[test]
fn fallback_does_not_change_operands_and_invariant_errors_are_structured() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut e = Execution::new(&mut host, Limits::default());
    for op in [
        Binary::Pow,
        Binary::BitAnd,
        Binary::Shl,
        Binary::In,
        Binary::InstanceOf,
    ] {
        e.stack = vec![Value::Number(1.0), Value::Number(2.0)];
        let old = e.stack.clone();
        assert!(!e.numeric_binary_in_place(op)?);
        assert_eq!(e.stack, old);
    }
    for (a, b) in [
        (Value::string("2"), Value::Number(3.0)),
        (Value::Number(2.0), Value::string("3")),
        (Value::Null, Value::Boolean(true)),
        (Value::Undefined, Value::Number(3.0)),
    ] {
        e.stack = vec![a, b];
        let old = e.stack.clone();
        assert!(!e.numeric_binary_in_place(Binary::Add)?);
        assert_eq!(e.stack, old);
    }
    e.stack.clear();
    assert_eq!(
        e.numeric_binary_in_place(Binary::Add),
        Err(Error::InvalidBytecode)
    );
    e.stack.push(Value::Number(7.0));
    assert_eq!(
        e.numeric_binary_in_place(Binary::Lt),
        Err(Error::InvalidBytecode)
    );
    assert_eq!(e.stack, vec![Value::Number(7.0)]);
    Ok(())
}

#[test]
fn dispatched_numeric_work_keeps_exact_fuel_stack_and_hook_semantics() -> Result<(), Error> {
    use crate::{Runtime, compile};
    let source = "1+2";
    let program = compile(source, Limits::default())?;
    let required = u64::try_from(program.instruction_count()).unwrap();
    let limits = Limits {
        fuel: required,
        stack: 2,
        ..Limits::default()
    };
    assert_eq!(
        Runtime::new(limits).run(&program, &mut SilentHost)?,
        Value::Number(3.0)
    );
    for limits in [
        Limits {
            fuel: required - 1,
            ..limits
        },
        Limits { stack: 1, ..limits },
    ] {
        assert!(matches!(
            Runtime::new(limits).run(&program, &mut SilentHost),
            Err(Error::Limit { .. })
        ));
    }
    for source in [
        "let log='';let a={valueOf(){log+='a';return 2}},b={valueOf(){log+='b';return 3}};a+b===5&&log==='ab'",
        "let count=0;let x={valueOf(){count++;return 2}};x*3===6&&count===1",
        "'2'+3==='23'&&2+'3'==='23'&&'2'*3===6&&true+null===1",
        "let n=0;try{({valueOf(){throw 7}})+({valueOf(){n++;return 3}})}catch(e){}n===0",
        "1/-0===-Infinity&&1/(-0*1)===-Infinity&&!(NaN===NaN)&&NaN!==NaN",
        "let f=(a,b)=>a+b;f(1,2)===3&&f('x',2)==='x2'&&f({valueOf(){return 3}},4)===7",
        "async function f(){let x=1+(await 2);if(x!==3)throw 7}f();true",
    ] {
        let program = compile(source, Limits::default())?;
        assert_eq!(
            Runtime::new(Limits::default()).run(&program, &mut SilentHost)?,
            Value::Boolean(true),
            "{source}"
        );
    }
    Ok(())
}
