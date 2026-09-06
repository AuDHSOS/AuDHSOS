// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A second implementation of wide arithmetic, written for the tests
//! alone and sharing nothing with the product code.
//!
//! It is schoolbook throughout: multiplication is the quadratic
//! algorithm over base `2^64`, and reduction is binary long division, one
//! bit at a time, with no Montgomery form anywhere. That is a different
//! arrangement of the same mathematics, which is what makes it worth
//! having: it disagrees with `Modulus` if either of the two is wrong.
#![expect(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    reason = "a reference exists to be read and compared against the definition; the ordinary \
              operators and plain indexing keep it in that shape, and an overflow here is a \
              failing test rather than a fault in the product"
)]

/// A non-negative integer as limbs of sixty-four bits, least significant
/// first, with no leading zero limb.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Big(Vec<u64>);

impl Big {
    /// The value a big-endian encoding carries.
    pub(crate) fn from_be_bytes(bytes: &[u8]) -> Big {
        let mut limbs = Vec::new();
        let mut limb = 0u64;
        let mut shift = 0u32;
        for byte in bytes.iter().rev() {
            limb |= u64::from(*byte) << shift;
            shift += 8;
            if shift == 64 {
                limbs.push(limb);
                limb = 0;
                shift = 0;
            }
        }
        if shift != 0 {
            limbs.push(limb);
        }
        Big(trimmed(limbs))
    }

    /// The value of a small integer.
    pub(crate) fn from_u64(value: u64) -> Big {
        Big(trimmed(vec![value]))
    }

    /// The big-endian encoding, right-aligned in `len` bytes.
    pub(crate) fn to_be_bytes(&self, len: usize) -> Vec<u8> {
        let mut bytes = vec![0u8; len];
        for (index, slot) in bytes.iter_mut().rev().enumerate() {
            let limb = self.0.get(index / 8).copied().unwrap_or(0);
            *slot = ((limb >> ((index % 8) * 8)) & 0xff) as u8;
        }
        bytes
    }

    /// Whether the value is zero.
    pub(crate) fn is_zero(&self) -> bool {
        self.0.is_empty()
    }

    /// How many bits the value occupies.
    pub(crate) fn bit_len(&self) -> usize {
        match self.0.last() {
            None => 0,
            Some(top) => self.0.len() * 64 - top.leading_zeros() as usize,
        }
    }

    /// Bit `position`, counted from the least significant.
    pub(crate) fn bit(&self, position: usize) -> bool {
        (self.0.get(position / 64).copied().unwrap_or(0) >> (position % 64)) & 1 == 1
    }

    /// The product, by the quadratic algorithm.
    pub(crate) fn mul(&self, other: &Big) -> Big {
        if self.is_zero() || other.is_zero() {
            return Big(Vec::new());
        }
        let mut product = vec![0u64; self.0.len() + other.0.len()];
        for (index, left) in self.0.iter().enumerate() {
            let mut carry = 0u128;
            for (offset, right) in other.0.iter().enumerate() {
                let sum = u128::from(*left) * u128::from(*right)
                    + u128::from(product[index + offset])
                    + carry;
                product[index + offset] = (sum & u128::from(u64::MAX)) as u64;
                carry = sum >> 64;
            }
            let mut position = index + other.0.len();
            while carry != 0 {
                let sum = u128::from(product[position]) + carry;
                product[position] = (sum & u128::from(u64::MAX)) as u64;
                carry = sum >> 64;
                position += 1;
            }
        }
        Big(trimmed(product))
    }

    /// The value shifted left by `bits`, which is a multiplication by a
    /// power of two.
    pub(crate) fn shl(&self, bits: usize) -> Big {
        if self.is_zero() {
            return Big(Vec::new());
        }
        let mut limbs = vec![0u64; bits / 64];
        limbs.extend_from_slice(&self.0);
        let within = (bits % 64) as u32;
        if within != 0 {
            let mut carry = 0u64;
            for slot in &mut limbs {
                let out = *slot >> (64 - within);
                *slot = (*slot << within) | carry;
                carry = out;
            }
            if carry != 0 {
                limbs.push(carry);
            }
        }
        Big(trimmed(limbs))
    }

    /// The remainder modulo `modulus`, by binary long division.
    pub(crate) fn rem(&self, modulus: &Big) -> Big {
        assert!(!modulus.is_zero(), "the reference does not divide by zero");
        let mut remainder = vec![0u64; modulus.0.len() + 1];
        for position in (0..self.bit_len()).rev() {
            let mut carry = u64::from(self.bit(position));
            for slot in &mut remainder {
                let out = *slot >> 63;
                *slot = (*slot << 1) | carry;
                carry = out;
            }
            if !below(&remainder, &modulus.0) {
                take_away(&mut remainder, &modulus.0);
            }
        }
        Big(trimmed(remainder))
    }

    /// `self^exponent` modulo `modulus`, with the exponent given
    /// big-endian.
    pub(crate) fn pow_mod(&self, exponent: &[u8], modulus: &Big) -> Big {
        let power = Big::from_be_bytes(exponent);
        let mut result = Big::from_u64(1).rem(modulus);
        for position in (0..power.bit_len()).rev() {
            result = result.mul(&result).rem(modulus);
            if power.bit(position) {
                result = result.mul(self).rem(modulus);
            }
        }
        result
    }
}

/// The limbs with any leading zero limb removed.
fn trimmed(mut limbs: Vec<u64>) -> Vec<u64> {
    while limbs.last() == Some(&0) {
        limbs.pop();
    }
    limbs
}

/// Whether `a` is below `b`, with `b` read as zero-extended to the width
/// of `a`.
fn below(a: &[u64], b: &[u64]) -> bool {
    let mut less = false;
    for (index, left) in a.iter().enumerate() {
        let right = b.get(index).copied().unwrap_or(0);
        if *left != right {
            less = *left < right;
        }
    }
    less
}

/// `a -= b`, with `b` read as zero-extended to the width of `a`.
fn take_away(a: &mut [u64], b: &[u64]) {
    let mut borrow = 0u64;
    for (index, slot) in a.iter_mut().enumerate() {
        let right = b.get(index).copied().unwrap_or(0);
        let (partial, first) = slot.overflowing_sub(right);
        let (value, second) = partial.overflowing_sub(borrow);
        *slot = value;
        borrow = u64::from(first || second);
    }
}
