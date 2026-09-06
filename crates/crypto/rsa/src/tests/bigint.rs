// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Just enough arbitrary-precision arithmetic to build a forgery.
//!
//! The forgery of catalog 6.6.55 against an exponent of three is not a
//! signature anyone made: it is the integer cube root of a value whose
//! high bytes are a well-formed PKCS #1 block with a short padding and a
//! long tail of rubbish. Finding that root needs multiplication and
//! comparison over numbers wider than a `u128` and nothing else, so that
//! is all this is.
#![expect(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    reason = "test scaffolding, written to be read against the definition; an overflow here is a \
              failing test rather than a fault in the product"
)]

use core::cmp::Ordering;

/// A non-negative integer as limbs of sixty-four bits, least significant
/// first, with no leading zero limb.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Big(Vec<u64>);

impl Big {
    /// Zero.
    pub(crate) fn zero() -> Big {
        Big(Vec::new())
    }

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

    /// The big-endian encoding, right-aligned in `len` bytes.
    pub(crate) fn to_be_bytes(&self, len: usize) -> Vec<u8> {
        let mut bytes = vec![0u8; len];
        for (index, slot) in bytes.iter_mut().rev().enumerate() {
            let limb = self.0.get(index / 8).copied().unwrap_or(0);
            *slot = ((limb >> ((index % 8) * 8)) & 0xff) as u8;
        }
        bytes
    }

    /// The product, by the quadratic algorithm.
    pub(crate) fn mul(&self, other: &Big) -> Big {
        if self.0.is_empty() || other.0.is_empty() {
            return Big::zero();
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

    /// The value with bit `position` set.
    pub(crate) fn with_bit(&self, position: usize) -> Big {
        let mut limbs = self.0.clone();
        limbs.resize(limbs.len().max(position / 64 + 1), 0);
        limbs[position / 64] |= 1u64 << (position % 64);
        Big(trimmed(limbs))
    }

    /// How the value compares with another.
    pub(crate) fn compare(&self, other: &Big) -> Ordering {
        self.0
            .len()
            .cmp(&other.0.len())
            .then_with(|| self.0.iter().rev().cmp(other.0.iter().rev()))
    }

    /// The largest value whose cube is at most this one, found one bit at
    /// a time from `bits` down.
    pub(crate) fn cube_root_floor(&self, bits: usize) -> Big {
        let mut root = Big::zero();
        for position in (0..bits).rev() {
            let candidate = root.with_bit(position);
            let cube = candidate.mul(&candidate).mul(&candidate);
            if cube.compare(self) != Ordering::Greater {
                root = candidate;
            }
        }
        root
    }
}

/// The limbs with any leading zero limb removed.
fn trimmed(mut limbs: Vec<u64>) -> Vec<u64> {
    while limbs.last() == Some(&0) {
        limbs.pop();
    }
    limbs
}
