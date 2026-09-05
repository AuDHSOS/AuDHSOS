// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Arithmetic modulo the order of the prime-order subgroup of Edwards
//! curve 25519.
//!
//! A scalar is four limbs of 64 bits, always below the order. Reduction of
//! a wide value walks its bits from the top and doubles the accumulator,
//! which is slower than a Barrett step and short enough to check by
//! reading. Nothing here is on a secret path: verification reduces a hash
//! that is public, and the signing behind `test-signing` produces test
//! data.

/// Bytes of an encoded scalar.
pub const BYTES: usize = 32;

/// The order of the prime-order subgroup.
const ORDER: [u64; 4] = [
    0x5812_631a_5cf5_d3ed,
    0x14de_f9de_a2f7_9cd6,
    0x0000_0000_0000_0000,
    0x1000_0000_0000_0000,
];

/// A residue modulo the order, as four limbs, least significant first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Scalar([u64; 4]);

impl Scalar {
    /// Zero.
    pub const ZERO: Scalar = Scalar([0; 4]);

    /// The scalar the bytes encode, or `None` when the value is at or above
    /// the order. RFC 8032 requires a signature to carry the canonical
    /// encoding, so a value that is merely congruent is a forgery attempt,
    /// not a different spelling.
    #[must_use]
    pub fn from_canonical(bytes: &[u8; BYTES]) -> Option<Scalar> {
        let limbs = limbs_of(bytes);
        if is_below(limbs, ORDER) {
            Some(Scalar(limbs))
        } else {
            None
        }
    }

    /// The residue of a wide value, walking its bits from the top.
    #[must_use]
    pub fn from_wide(bytes: &[u8; 64]) -> Scalar {
        let mut result = Scalar::ZERO;
        for position in (0..512u32).rev() {
            result = result.double();
            result = result.add_bit(bit_of(bytes, position));
        }
        result
    }

    /// The residue of a 32-byte value that need not be canonical, which is
    /// what a clamped Ed25519 secret is.
    #[must_use]
    pub fn from_bytes_reduced(bytes: &[u8; BYTES]) -> Scalar {
        let mut wide = [0u8; 64];
        for (slot, byte) in wide.iter_mut().zip(bytes) {
            *slot = *byte;
        }
        Scalar::from_wide(&wide)
    }

    /// The canonical little-endian encoding.
    #[must_use]
    pub fn to_bytes(self) -> [u8; BYTES] {
        let mut bytes = [0u8; BYTES];
        let (chunks, _) = bytes.as_chunks_mut::<8>();
        for (chunk, limb) in chunks.iter_mut().zip(self.0) {
            *chunk = limb.to_le_bytes();
        }
        bytes
    }

    /// The sum.
    #[must_use]
    #[expect(
        clippy::should_implement_trait,
        reason = "the operator traits would invite `+` and `*` at every call site, where the workspace\
                  denies unchecked arithmetic; group and scalar operations are named after what \
                  they are"
    )]
    pub fn add(self, other: Scalar) -> Scalar {
        let (sum, carry) = add_limbs(self.0, other.0);
        Scalar(subtract_order(sum, carry))
    }

    /// The product, by doubling and adding along the bits of `other`.
    #[must_use]
    #[expect(
        clippy::should_implement_trait,
        reason = "the operator traits would invite `+` and `*` at every call site, where the workspace\
                  denies unchecked arithmetic; group and scalar operations are named after what \
                  they are"
    )]
    pub fn mul(self, other: Scalar) -> Scalar {
        let mut result = Scalar::ZERO;
        for position in (0..256u32).rev() {
            result = result.double();
            if other.bit(position) == 1 {
                result = result.add(self);
            }
        }
        result
    }

    /// Bit `position` of the scalar.
    #[must_use]
    pub fn bit(self, position: u32) -> u8 {
        let bytes = self.to_bytes();
        bit_of_slice(&bytes, position)
    }

    /// Whether the scalar is zero.
    #[must_use]
    pub fn is_zero(self) -> bool {
        self.0 == [0; 4]
    }

    /// Twice the scalar.
    fn double(self) -> Scalar {
        let (sum, carry) = add_limbs(self.0, self.0);
        Scalar(subtract_order(sum, carry))
    }

    /// The scalar with a bit added, which cannot overflow the order by more
    /// than one subtraction.
    fn add_bit(self, bit: u8) -> Scalar {
        let (sum, carry) = add_limbs(self.0, [u64::from(bit), 0, 0, 0]);
        Scalar(subtract_order(sum, carry))
    }
}

/// The limbs of a little-endian encoding.
fn limbs_of(bytes: &[u8; BYTES]) -> [u64; 4] {
    let (chunks, _) = bytes.as_chunks::<8>();
    let mut limbs = [0u64; 4];
    for (slot, chunk) in limbs.iter_mut().zip(chunks) {
        *slot = u64::from_le_bytes(*chunk);
    }
    limbs
}

/// Whether `a` is below `b`.
fn is_below(a: [u64; 4], b: [u64; 4]) -> bool {
    let (_, borrow) = sub_limbs(a, b);
    borrow == 1
}

/// The sum with the carry out.
fn add_limbs(a: [u64; 4], b: [u64; 4]) -> ([u64; 4], u64) {
    let mut result = [0u64; 4];
    let mut carry = 0u64;
    for (slot, (left, right)) in result.iter_mut().zip(a.iter().zip(b)) {
        let sum = u128::from(*left)
            .wrapping_add(u128::from(right))
            .wrapping_add(u128::from(carry));
        *slot = low64(sum);
        carry = low64(sum >> 64);
    }
    (result, carry)
}

/// The difference with the borrow out.
fn sub_limbs(a: [u64; 4], b: [u64; 4]) -> ([u64; 4], u64) {
    let mut result = [0u64; 4];
    let mut borrow = 0u64;
    for (slot, (left, right)) in result.iter_mut().zip(a.iter().zip(b)) {
        let (partial, first) = left.overflowing_sub(right);
        let (value, second) = partial.overflowing_sub(borrow);
        *slot = value;
        borrow = u64::from(first || second);
    }
    (result, borrow)
}

/// Subtracts the order while the value is at or above it. A carry out of
/// the addition means the value is above the order twice over.
fn subtract_order(value: [u64; 4], carry: u64) -> [u64; 4] {
    let mut result = value;
    let mut above = carry;
    for _ in 0..2 {
        let (difference, borrow) = sub_limbs(result, ORDER);
        if above == 1 || borrow == 0 {
            result = difference;
            above = 0;
        } else {
            break;
        }
    }
    result
}

/// Bit `position` of a fixed-size little-endian value.
fn bit_of(bytes: &[u8; 64], position: u32) -> u8 {
    bit_of_slice(bytes, position)
}

/// Bit `position` of a little-endian byte string.
fn bit_of_slice(bytes: &[u8], position: u32) -> u8 {
    let index = usize::try_from(position.wrapping_shr(3)).unwrap_or(usize::MAX);
    let within = position & 7;
    bytes
        .get(index)
        .map_or(0, |byte| byte.wrapping_shr(within) & 1)
}

/// The low sixty-four bits of a wide value.
#[expect(clippy::as_conversions, reason = "the mask keeps only the low 64 bits")]
const fn low64(value: u128) -> u64 {
    (value & 0xFFFF_FFFF_FFFF_FFFF) as u64
}
