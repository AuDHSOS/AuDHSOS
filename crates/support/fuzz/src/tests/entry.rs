// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::entry`, covering the entry glue items of the catalog
//! 6.6.53. They run under Miri, which is what makes the one `unsafe` of
//! the fuzzing infrastructure checked rather than argued.

use crate::entry::input;

#[test]
fn the_glue_passes_the_bytes_through_unchanged() {
    let bytes = [1u8, 2, 3, 4, 5];
    // SAFETY: the array lives for the whole test and nothing writes to it
    // while the slice is alive.
    let seen = unsafe { input(bytes.as_ptr(), bytes.len()) };
    assert_eq!(seen, &bytes);
}

#[test]
fn one_byte_is_a_slice_of_one() {
    let bytes = [0xFFu8];
    // SAFETY: as above.
    let seen = unsafe { input(bytes.as_ptr(), 1) };
    assert_eq!(seen, &[0xFF]);
}

#[test]
fn an_empty_input_never_touches_the_pointer() {
    let bytes = [7u8; 4];
    // SAFETY: a length of zero returns before it reads the pointer.
    let from_valid = unsafe { input(bytes.as_ptr(), 0) };
    // SAFETY: the same, which is why a null pointer is allowed here.
    let from_null = unsafe { input(core::ptr::null(), 0) };
    // SAFETY: a null pointer returns the empty slice whatever the length,
    // because no fuzzer can promise bytes behind it.
    let null_with_length = unsafe { input(core::ptr::null(), 16) };
    assert!(from_valid.is_empty());
    assert!(from_null.is_empty());
    assert!(null_with_length.is_empty());
}
