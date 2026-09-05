// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A second implementation of the field, written for the tests alone and
//! sharing nothing with the product code.
//!
//! It holds a value as four limbs of 64 bits rather than five of 51, and
//! reduces by folding `2^256` back in as thirty-eight, which is a different
//! arrangement of the same mathematics. Its purpose is to disagree with
//! `Fe` if either of the two is wrong.
#![expect(
    clippy::as_conversions,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a reference exists to be read and compared against the specification; the ordinary \
              operators keep it in that shape, and an overflow here is a failing test rather than \
              a fault in the product"
)]

/// The prime, as four limbs.
const PRIME: [u64; 4] = [
    0xFFFF_FFFF_FFFF_FFED,
    0xFFFF_FFFF_FFFF_FFFF,
    0xFFFF_FFFF_FFFF_FFFF,
    0x7FFF_FFFF_FFFF_FFFF,
];

/// A field element as four limbs, least significant first, always below
/// the prime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Reference([u64; 4]);

impl Reference {
    /// The element the bytes encode, with the highest bit ignored and the
    /// value reduced.
    pub(crate) fn from_bytes(bytes: &[u8; 32]) -> Reference {
        let (chunks, _) = bytes.as_chunks::<8>();
        let mut limbs = [0u64; 4];
        for (slot, chunk) in limbs.iter_mut().zip(chunks) {
            *slot = u64::from_le_bytes(*chunk);
        }
        limbs[3] &= 0x7FFF_FFFF_FFFF_FFFF;
        Reference(reduced(limbs))
    }

    /// The canonical little-endian encoding.
    pub(crate) fn to_bytes(self) -> [u8; 32] {
        let mut bytes = [0u8; 32];
        let (chunks, _) = bytes.as_chunks_mut::<8>();
        for (chunk, limb) in chunks.iter_mut().zip(self.0) {
            *chunk = limb.to_le_bytes();
        }
        bytes
    }

    /// The sum.
    pub(crate) fn add(self, other: Reference) -> Reference {
        let (sum, carry) = add_limbs(self.0, other.0);
        Reference(fold(sum, carry))
    }

    /// The difference.
    pub(crate) fn sub(self, other: Reference) -> Reference {
        let (difference, borrow) = sub_limbs(self.0, other.0);
        if borrow == 0 {
            Reference(difference)
        } else {
            let (wrapped, _) = add_limbs(difference, PRIME);
            Reference(reduced(wrapped))
        }
    }

    /// The product.
    pub(crate) fn mul(self, other: Reference) -> Reference {
        let mut wide = [0u64; 8];
        for (index, left) in self.0.iter().enumerate() {
            let mut carry = 0u64;
            for (slot, right) in wide.iter_mut().skip(index).zip(other.0) {
                let term =
                    u128::from(*left) * u128::from(right) + u128::from(*slot) + u128::from(carry);
                *slot = term as u64;
                carry = (term >> 64) as u64;
            }
            if let Some(slot) = wide.get_mut(index + 4) {
                *slot = slot.wrapping_add(carry);
            }
        }

        // `2^256` is thirty-eight modulo the prime, so the upper half folds
        // into the lower one with that factor. Two rounds bring it inside
        // four limbs.
        let mut value = [0u64; 4];
        value.copy_from_slice(&wide[..4]);
        let mut upper = [0u64; 4];
        upper.copy_from_slice(&wide[4..]);
        for _ in 0..3 {
            let (scaled, overflow) = mul_small(upper, 38);
            let (sum, carry) = add_limbs(value, scaled);
            value = sum;
            upper = [overflow.wrapping_add(carry), 0, 0, 0];
            if upper == [0; 4] {
                break;
            }
        }
        Reference(reduced(value))
    }
}

/// The sum of two four-limb values with the carry out.
fn add_limbs(a: [u64; 4], b: [u64; 4]) -> ([u64; 4], u64) {
    let mut result = [0u64; 4];
    let mut carry = 0u128;
    for (slot, (left, right)) in result.iter_mut().zip(a.iter().zip(b)) {
        let sum = u128::from(*left) + u128::from(right) + carry;
        *slot = sum as u64;
        carry = sum >> 64;
    }
    (result, carry as u64)
}

/// The difference of two four-limb values with the borrow out.
fn sub_limbs(a: [u64; 4], b: [u64; 4]) -> ([u64; 4], u64) {
    let mut result = [0u64; 4];
    let mut borrow = 0i128;
    for (slot, (left, right)) in result.iter_mut().zip(a.iter().zip(b)) {
        let difference = i128::from(*left) - i128::from(right) - borrow;
        *slot = difference as u64;
        borrow = i128::from(difference < 0);
    }
    (result, borrow as u64)
}

/// A four-limb value times a small factor, with the overflow.
fn mul_small(a: [u64; 4], factor: u64) -> ([u64; 4], u64) {
    let mut result = [0u64; 4];
    let mut carry = 0u128;
    for (slot, limb) in result.iter_mut().zip(a) {
        let term = u128::from(limb) * u128::from(factor) + carry;
        *slot = term as u64;
        carry = term >> 64;
    }
    (result, carry as u64)
}

/// Folds a carry out of the top back in and reduces below the prime.
fn fold(value: [u64; 4], carry: u64) -> [u64; 4] {
    let (scaled, _) = mul_small([carry, 0, 0, 0], 38);
    let (sum, again) = add_limbs(value, scaled);
    if again == 0 {
        reduced(sum)
    } else {
        fold(sum, again)
    }
}

/// The value reduced below the prime, subtracting at most twice.
fn reduced(mut value: [u64; 4]) -> [u64; 4] {
    for _ in 0..3 {
        let (difference, borrow) = sub_limbs(value, PRIME);
        if borrow == 0 {
            value = difference;
        } else {
            break;
        }
    }
    value
}
