// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The format language against arbitrary bytes: no format may panic, no
//! format may answer more bytes than a value holds, and a format with no
//! conversion in it answers itself.
//!
//! A format reader is where a width, a precision and a count of copies
//! meet arithmetic on the bytes they describe, and every one of those is
//! a panic in Rust rather than a wrong answer.

use db_sqlite::format::format;
use db_sqlite::func::MAX_LENGTH;
use db_sqlite::value::Value;

/// The values the format is put to, picked by the first byte. Each is
/// short, so that a width comes from the format and not from here.
fn arguments(pick: u8) -> Vec<Value> {
    let all = [
        Value::Null,
        Value::Int(0),
        Value::Int(-3),
        Value::Int(i64::MIN),
        Value::Real(2.5),
        Value::Real(-0.0625),
        Value::Real(f64::INFINITY),
        Value::Text(b"a'b\\c".to_vec()),
        Value::Text(alloc_text()),
        Value::Blob(b"\x00\x01\x41".to_vec()),
    ];
    let count = usize::from(pick % 4);
    let mut out = Vec::new();
    for at in 0..count {
        let index = (usize::from(pick / 4) + at) % all.len();
        out.push(all.get(index).cloned().unwrap_or(Value::Null));
    }
    out
}

/// Text of more than one character, so that `!` has characters to count.
fn alloc_text() -> Vec<u8> {
    "kä\u{1d11e}z".as_bytes().to_vec()
}

/// Whether the format asks for a field wider than a fuzz run may write.
/// The refusal past a thousand million bytes is what the recorded corpus
/// holds the engine to; here it would only take the memory.
fn wide(format: &[u8]) -> bool {
    let mut run: u64 = 0;
    for byte in format {
        if byte.is_ascii_digit() {
            run = run.saturating_mul(10).saturating_add(u64::from(byte - b'0'));
            if run > 100_000 {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    let Some((pick, rest)) = bytes.split_first() else {
        return;
    };
    if wide(rest) {
        return;
    }
    let mut args = vec![Value::Text(rest.to_vec())];
    args.extend(arguments(*pick));
    let Ok(answer) = format(&args) else {
        return;
    };
    assert!(
        matches!(answer, Value::Text(_) | Value::Null),
        "a format that answers neither text nor nothing"
    );
    let written = answer.text().unwrap_or_default();
    assert!(
        written.len() < MAX_LENGTH,
        "a format that answers more than a value holds"
    );
    // The format is read up to the first nought, and a format with no
    // conversion in it is copied out as it stands.
    let plain = rest.split(|byte| *byte == 0).next().unwrap_or_default();
    if !plain.is_empty() && !plain.contains(&b'%') {
        assert_eq!(written, plain, "a format with no conversion that moved");
    }
});
