// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The DER reader against arbitrary bytes: no input may panic, every value
//! the reader hands back is shorter than what it came from, and a reader
//! that returns a value has moved past it.
//!
//! The walk descends into constructed values, which is where a reader that
//! trusts a length loses its footing. It stops at `MAX_DEPTH`, because the
//! reader stops there too and a deeper walk would only exhaust this
//! program's own stack.


use audhsos_der::{MAX_DEPTH, Reader, from_generalized_time, from_utc_time};

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    walk(&mut Reader::new(bytes), 0);
    times(bytes);
    typed(bytes);
});

/// Reads every value at this level and descends into the constructed ones.
fn walk(reader: &mut Reader<'_>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    while !reader.is_empty() {
        let before = reader.rest().len();
        let Ok((tag, value)) = reader.read_any() else {
            return;
        };
        let after = reader.rest().len();
        assert!(after < before, "a value that consumed nothing");
        assert!(
            value.len() < before,
            "a value longer than the bytes it was read from"
        );
        assert!(!tag.is_high_form(), "a tag the reader must refuse");
        if tag.is_constructed() {
            walk(&mut Reader::new(value), depth + 1);
        }
    }
}

/// The two time forms against the whole input, because a walk reaches them
/// only for an input that happens to carry their tag.
fn times(bytes: &[u8]) {
    for parsed in [from_utc_time(bytes), from_generalized_time(bytes)] {
        let Ok(time) = parsed else {
            continue;
        };
        assert!((1..=12).contains(&time.month), "a month out of range");
        assert!((1..=31).contains(&time.day), "a day out of range");
        assert!(time.hour < 24, "an hour out of range");
        assert!(time.minute < 60, "a minute out of range");
        assert!(time.second < 60, "a second out of range");
    }
}

/// The readers that check a value's shape as well as its frame, each on a
/// reader of its own so that one refusal does not hide the next.
fn typed(bytes: &[u8]) {
    let _ = Reader::new(bytes).read_integer();
    let _ = Reader::new(bytes).read_boolean();
    let _ = Reader::new(bytes).read_null();
    let _ = Reader::new(bytes).read_octet_string();
    let _ = Reader::new(bytes).read_object_identifier();
    let _ = Reader::new(bytes).read_time();
    if let Ok(bits) = Reader::new(bytes).read_bit_string() {
        let _ = bits.whole_bytes();
    }
    if let Ok(mut inner) = Reader::new(bytes).read_sequence() {
        walk(&mut inner, 1);
    }
}
