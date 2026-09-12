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
/// Both buffers are written in either case. They are arrays of one length,
/// so a mismatch is a compile error rather than a silent exchange of the
/// common prefix; the Montgomery ladder that needs this function works on
/// fixed-size values anyway.
pub fn ct_swap<const N: usize>(choice: Choice, a: &mut [u8; N], b: &mut [u8; N]) {
    let mask = choice.mask_u8();
    for (left, right) in a.iter_mut().zip(b.iter_mut()) {
        let difference = mask & (*left ^ *right);
        *left ^= difference;
        *right ^= difference;
    }
}

/// Exchanges the contents of `a` and `b` when `choice` is true, and leaves
/// them untouched otherwise.
///
/// The word-wide [`ct_swap`], for the two working values of a modular
/// exponentiation whose exponent must not be observable. Both buffers are
/// written in either case.
pub fn ct_swap_u64<const N: usize>(choice: Choice, a: &mut [u64; N], b: &mut [u64; N]) {
    let mask = choice.mask_u64();
    for (left, right) in a.iter_mut().zip(b.iter_mut()) {
        let difference = mask & (*left ^ *right);
        *left ^= difference;
        *right ^= difference;
    }
}

/// Copies `source` over `destination` when `choice` is true, and leaves
/// `destination` unchanged otherwise. Both are arrays of one length, for
/// the reason given at [`ct_swap`].
pub fn ct_copy<const N: usize>(choice: Choice, destination: &mut [u8; N], source: &[u8; N]) {
    let mask = choice.mask_u8();
    for (target, byte) in destination.iter_mut().zip(source.iter()) {
        *target ^= mask & (*target ^ *byte);
    }
}
