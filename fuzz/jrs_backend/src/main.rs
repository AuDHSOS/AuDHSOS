// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The two backends must answer a source identically.
//!
//! A Program the register backend accepts is executed twice: once as compiled
//! and once with that backend withheld. A difference is a miscompile, which is
//! the failure mode a backend migration has to exclude — the register lowering
//! types every value statically, so a wrong type is a wrong answer rather than
//! a refusal.
#![forbid(unsafe_code)]

use jrs::{Error, Limits, Runtime, SilentHost, Value, compile};

fn same_value(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(left), Value::Number(right)) => {
            (left.is_nan() && right.is_nan()) || left.to_bits() == right.to_bits()
        }
        _ => left == right,
    }
}

/// How two outcomes are compared. A resource limit is not comparable: the two
/// backends charge fuel differently by design, and the register backend checks
/// it at loop back edges rather than at every instruction.
fn comparable(outcome: &Result<Value, Error>) -> bool {
    !matches!(outcome, Err(Error::Limit { .. }))
}

fn describe(outcome: &Result<Value, Error>) -> Option<Value> {
    match outcome {
        Ok(value) | Err(Error::Thrown { value }) => Some(value.clone()),
        Err(_) => None,
    }
}

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    let Ok(source) = core::str::from_utf8(bytes) else {
        return;
    };
    let limits = Limits {
        source_bytes: 4096,
        tokens: 2048,
        nesting: 32,
        instructions: 4096,
        fuel: 65_536,
        stack: 128,
        string_units: 4096,
        heap_entries: 256,
        feedback_vectors: 64,
        call_frames: 32,
        binding_slots: 512,
        properties: 128,
        jobs: 128,
        weak_entries: 128,
    };
    let Ok(program) = compile(source, limits) else {
        return;
    };
    if !program.uses_register_backend() {
        return;
    }
    let legacy = program.legacy_only();
    let actual = Runtime::new(limits).run(&program, &mut SilentHost);
    let expected = Runtime::new(limits).run(&legacy, &mut SilentHost);
    assert!(
        !matches!(actual, Err(Error::InvalidBytecode)),
        "register backend rejected its own bytecode: {source}"
    );
    if !comparable(&actual) || !comparable(&expected) {
        return;
    }
    assert_eq!(
        actual.is_ok(),
        expected.is_ok(),
        "backends disagree on completion: {source}"
    );
    match (describe(&actual), describe(&expected)) {
        (Some(actual), Some(expected)) => assert!(
            same_value(&actual, &expected),
            "backends disagree on the value of {source}: {actual:?} against {expected:?}"
        ),
        (None, None) => {}
        (actual, expected) => {
            panic!("backends disagree on the error of {source}: {actual:?} against {expected:?}")
        }
    }
});
