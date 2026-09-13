// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The evaluator against arbitrary bytes: no expression may panic, and
//! no expression may answer a value the engine cannot then write down.
//!
//! An evaluator is where an arithmetic overflow, a shift past the width
//! of a word and a conversion of a double outside the range of an
//! integer live, and every one of those is a panic in Rust rather than a
//! wrong answer. This target is what holds that none of them is reached.

use db_sqlite::eval::evaluate;
use db_sqlite::fp::{DIGITS, text};
use db_sqlite::number::{integer, real};
use db_sqlite::parse::expression;
use db_sqlite::value::{Collation, Value, compare};

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    // Every piece of text is read as a number and back without a panic,
    // whatever is in it.
    let read = real(bytes);
    let written = text(read.value, DIGITS);
    assert!(!written.is_empty(), "a double with no text");
    let _ = integer(bytes);

    let Ok((arena, root)) = expression(bytes) else {
        return;
    };
    let Ok(value) = evaluate(&arena, root, bytes) else {
        return;
    };
    // A value is equal to itself under every collation, which is the one
    // thing a comparison must always answer.
    for collation in [Collation::Binary, Collation::NoCase, Collation::Rtrim] {
        assert_eq!(
            compare(&value, &value, collation),
            core::cmp::Ordering::Equal,
            "a value that is not equal to itself"
        );
    }
    // A number is written down and read back as itself.
    match value {
        // A zero is written without its sign, so a negative zero is the
        // one double that does not come back as the bits it went out as.
        Value::Real(number) if number.is_finite() && number.abs() > 0.0 => {
            let written = text(number, DIGITS);
            let back = real(&written);
            assert!(back.complete(), "a double that does not read back");
            assert_eq!(
                back.value.to_bits(),
                number.to_bits(),
                "a double that reads back as another"
            );
        }
        Value::Int(number) => {
            let written = db_sqlite::number::integer_text(number);
            assert_eq!(integer(&written).value, number, "an integer that moved");
        }
        _ => {}
    }
});
