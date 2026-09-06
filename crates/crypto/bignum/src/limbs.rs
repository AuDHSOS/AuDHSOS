// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Arithmetic over limbs of sixty-four bits, least significant first.
//!
//! These are the three operations `crypto-ec` used to hold itself, in the
//! form that already took its modulus as an argument, plus the comparison
//! that the in-place shape of the other three needs. Every one of them
//! runs over the shorter of the slices it is given, so a caller that
//! passes slices of one length gets arithmetic of that width and a caller
//! that mixes widths gets the narrower one; no call panics and none
//! allocates.
//!
//! Nothing here is constant time. The final conditional subtraction of a
//! Montgomery product branches on the value it has just computed, which
//! is the point of the method. Every value that reaches this crate is
//! public, and the crate documentation says why that is not going to
//! change.

/// The sum `a + b`, written into `a`, with the carry out of the top limb.
#[must_use]
pub fn add_limbs(a: &mut [u64], b: &[u64]) -> u64 {
    let mut carry = 0u64;
    for (slot, right) in a.iter_mut().zip(b.iter().copied()) {
        let sum = u128::from(*slot)
            .wrapping_add(u128::from(right))
            .wrapping_add(u128::from(carry));
        *slot = low(sum);
        carry = high(sum);
    }
    carry
}

/// The difference `a - b`, written into `a`, with the borrow out of the
/// top limb.
#[must_use]
pub fn subtract(a: &mut [u64], b: &[u64]) -> u64 {
    let mut borrow = 0u64;
    for (slot, right) in a.iter_mut().zip(b.iter().copied()) {
        let (partial, first) = slot.overflowing_sub(right);
        let (value, second) = partial.overflowing_sub(borrow);
        *slot = value;
        borrow = u64::from(first || second);
    }
    borrow
}

/// Whether `a` is below `b`, both read as unsigned values of the same
/// width.
///
/// The scan runs upwards and keeps the last answer, so the most
/// significant limb the two differ in is the one that decides.
#[must_use]
pub fn is_less(a: &[u64], b: &[u64]) -> bool {
    let mut less = false;
    for (left, right) in a.iter().copied().zip(b.iter().copied()) {
        if left != right {
            less = left < right;
        }
    }
    less
}

/// The Montgomery product `a * b * 2^(-64*N)` modulo `modulus`, written
/// into `out`, by the coarsely integrated operand scanning method.
///
/// `N` is the length of `out`, and the four slices are meant to have that
/// same length. The running total is `N + 2` limbs wide: the lowest `N`
/// are `out`, and the two above it are named, because the width is not
/// known when the code is compiled and an array of `N + 2` cannot be
/// spelled.
pub fn montgomery(a: &[u64], b: &[u64], modulus: &[u64], n0inv: u64, out: &mut [u64]) {
    out.fill(0);
    let mut above = 0u64;

    for factor in b.iter().copied() {
        // out += a * factor
        let mut carry = 0u64;
        for (slot, limb) in out.iter_mut().zip(a.iter().copied()) {
            let sum = u128::from(*slot)
                .wrapping_add(u128::from(limb).wrapping_mul(u128::from(factor)))
                .wrapping_add(u128::from(carry));
            *slot = low(sum);
            carry = high(sum);
        }
        let sum = u128::from(above).wrapping_add(u128::from(carry));
        above = low(sum);
        let mut carry_out = high(sum);

        // out += m * modulus, with m chosen so the lowest limb becomes zero
        let m = out.first().copied().unwrap_or(0).wrapping_mul(n0inv);
        let mut carry = 0u64;
        for (slot, limb) in out.iter_mut().zip(modulus.iter().copied()) {
            let sum = u128::from(*slot)
                .wrapping_add(u128::from(m).wrapping_mul(u128::from(limb)))
                .wrapping_add(u128::from(carry));
            *slot = low(sum);
            carry = high(sum);
        }
        let sum = u128::from(above).wrapping_add(u128::from(carry));
        above = low(sum);
        carry_out = carry_out.wrapping_add(high(sum));

        // Divide by 2^64: the lowest limb is zero, so every limb moves
        // down one place and the two named limbs follow it.
        out.rotate_left(1);
        for slot in out.iter_mut().rev().take(1) {
            *slot = above;
        }
        above = carry_out;
    }

    if above != 0 || !is_less(out, modulus) {
        let _ = subtract(out, modulus);
    }
}

/// Twice `a`, written into `a`, with the bit shifted out of the top limb.
#[must_use]
pub(crate) fn shift_left_one(a: &mut [u64]) -> u64 {
    let mut carry = 0u64;
    for slot in a.iter_mut() {
        let out = slot.wrapping_shr(63);
        *slot = slot.wrapping_shl(1) | carry;
        carry = out;
    }
    carry
}

/// The low sixty-four bits of a wide value.
#[expect(clippy::as_conversions, reason = "the mask keeps only the low 64 bits")]
const fn low(value: u128) -> u64 {
    (value & 0xFFFF_FFFF_FFFF_FFFF) as u64
}

/// The high sixty-four bits of a wide value.
#[expect(clippy::as_conversions, reason = "the shift leaves 64 bits")]
const fn high(value: u128) -> u64 {
    (value >> 64) as u64
}
