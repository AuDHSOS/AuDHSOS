// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Comparison, selection, and exchange without a branch on the data.
//!
//! Invariant of every function here: the control flow depends on the
//! lengths of the arguments and on nothing else. Lengths are public; the
//! bytes are not.

use crate::choice::Choice;

/// Whether two byte strings are equal, without revealing where they differ.
///
/// The lengths are public and are compared normally: a caller that compares
/// a 16-byte tag with a 32-byte one has made a protocol error, not leaked a
/// secret. The bytes are folded into one accumulator, so the running time
/// depends on the length alone.
#[must_use]
pub fn ct_eq(a: &[u8], b: &[u8]) -> Choice {
    if a.len() != b.len() {
        return Choice::NO;
    }
    let mut difference = 0u8;
    for (left, right) in a.iter().zip(b.iter()) {
        difference |= left ^ right;
    }
    Choice::is_zero_u8(difference)
}

/// `a` when `choice` is true, `b` otherwise.
#[must_use]
pub const fn ct_select_u8(choice: Choice, a: u8, b: u8) -> u8 {
    b ^ (choice.mask_u8() & (a ^ b))
}

/// `a` when `choice` is true, `b` otherwise.
#[must_use]
pub const fn ct_select_u32(choice: Choice, a: u32, b: u32) -> u32 {
    b ^ (choice.mask_u32() & (a ^ b))
}

/// `a` when `choice` is true, `b` otherwise.
#[must_use]
pub const fn ct_select_u64(choice: Choice, a: u64, b: u64) -> u64 {
    b ^ (choice.mask_u64() & (a ^ b))
}

/// Exchanges the contents of `a` and `b` when `choice` is true, and leaves
/// them untouched otherwise.
///
/// Both buffers are written in either case. Bytes beyond the shorter of the
/// two are left alone; callers pass buffers of equal length, and the
/// Montgomery ladder that needs this function passes fixed-size ones.
pub fn ct_swap(choice: Choice, a: &mut [u8], b: &mut [u8]) {
    let mask = choice.mask_u8();
    for (left, right) in a.iter_mut().zip(b.iter_mut()) {
        let difference = mask & (*left ^ *right);
        *left ^= difference;
        *right ^= difference;
    }
}

/// Copies `source` over `destination` when `choice` is true, and leaves
/// `destination` unchanged otherwise. Bytes beyond the shorter buffer are
/// not touched.
pub fn ct_copy(choice: Choice, destination: &mut [u8], source: &[u8]) {
    let mask = choice.mask_u8();
    for (target, byte) in destination.iter_mut().zip(source.iter()) {
        *target ^= mask & (*target ^ *byte);
    }
}
