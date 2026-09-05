// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Arithmetic in the field of `2^255 - 19`.
//!
//! An element is five limbs of 51 bits over `u64`, so a product of two
//! limbs fits in 128 bits and the reduction is a fold of the top limb back
//! into the bottom one with a factor of nineteen. Limbs are allowed to grow
//! past 51 bits between operations; every multiplication carries them back
//! down, and the canonical encoding reduces fully.
//!
//! Invariants: no operation branches on the value of an element, and none
//! indexes with one. `to_bytes` is the only fully reduced representation,
//! and it is the only place where the comparison with the prime happens.

use crypto_ct::{Choice, ct_select_u64};

/// Bytes of an encoded element.
pub const BYTES: usize = 32;

/// The width of one limb.
const MASK: u64 = (1 << 51) - 1;

/// An element of the field, as five limbs of 51 bits, least significant
/// first.
#[derive(Clone, Copy, Debug, Default)]
pub struct Fe([u64; 5]);

impl Fe {
    /// The additive identity.
    pub const ZERO: Fe = Fe([0; 5]);
    /// The multiplicative identity.
    pub const ONE: Fe = Fe([1, 0, 0, 0, 0]);

    /// The element with the given limbs, for the constants of the curves.
    #[must_use]
    pub const fn from_limbs(limbs: [u64; 5]) -> Fe {
        Fe(limbs)
    }

    /// The element `value`, for small constants.
    #[must_use]
    pub const fn from_u64(value: u64) -> Fe {
        Fe([value, 0, 0, 0, 0])
    }

    /// The element the bytes encode, little-endian. The highest bit is
    /// ignored, as every use of this encoding prescribes, and a value at or
    /// above the prime wraps rather than being rejected; callers that must
    /// reject it compare against [`Fe::to_bytes`].
    #[must_use]
    pub fn from_bytes(bytes: &[u8; BYTES]) -> Fe {
        let (chunks, _) = bytes.as_chunks::<8>();
        let mut words = [0u64; 4];
        for (slot, chunk) in words.iter_mut().zip(chunks) {
            *slot = u64::from_le_bytes(*chunk);
        }
        let [w0, w1, w2, w3] = words;
        Fe([
            w0 & MASK,
            ((w0 >> 51) | (w1 << 13)) & MASK,
            ((w1 >> 38) | (w2 << 26)) & MASK,
            ((w2 >> 25) | (w3 << 39)) & MASK,
            (w3 >> 12) & MASK,
        ])
    }

    /// The canonical little-endian encoding: fully reduced, highest bit
    /// clear.
    #[must_use]
    pub fn to_bytes(self) -> [u8; BYTES] {
        let mut h = self.carried().0;

        // Whether the value is at or above the prime: adding nineteen
        // carries out of the top limb exactly then.
        let mut q = h[0].wrapping_add(19) >> 51;
        q = h[1].wrapping_add(q) >> 51;
        q = h[2].wrapping_add(q) >> 51;
        q = h[3].wrapping_add(q) >> 51;
        q = h[4].wrapping_add(q) >> 51;

        h[0] = h[0].wrapping_add(q.wrapping_mul(19));
        h[1] = h[1].wrapping_add(h[0] >> 51);
        h[0] &= MASK;
        h[2] = h[2].wrapping_add(h[1] >> 51);
        h[1] &= MASK;
        h[3] = h[3].wrapping_add(h[2] >> 51);
        h[2] &= MASK;
        h[4] = h[4].wrapping_add(h[3] >> 51);
        h[3] &= MASK;
        h[4] &= MASK;

        let words = [
            h[0] | (h[1] << 51),
            (h[1] >> 13) | (h[2] << 38),
            (h[2] >> 26) | (h[3] << 25),
            (h[3] >> 39) | (h[4] << 12),
        ];
        let mut bytes = [0u8; BYTES];
        let (chunks, _) = bytes.as_chunks_mut::<8>();
        for (chunk, word) in chunks.iter_mut().zip(words) {
            *chunk = word.to_le_bytes();
        }
        bytes
    }

    /// The sum.
    #[must_use]
    #[expect(
        clippy::should_implement_trait,
        reason = "the operator traits would invite `+` and `*` at every call site, where the workspace\
                  denies unchecked arithmetic; field operations are named after what they are"
    )]
    pub fn add(self, other: Fe) -> Fe {
        let mut result = [0u64; 5];
        for (slot, (left, right)) in result.iter_mut().zip(self.0.iter().zip(other.0)) {
            *slot = left.wrapping_add(right);
        }
        Fe(result).carried()
    }

    /// The difference.
    #[must_use]
    #[expect(
        clippy::should_implement_trait,
        reason = "the operator traits would invite `+` and `*` at every call site, where the workspace\
                  denies unchecked arithmetic; field operations are named after what they are"
    )]
    pub fn sub(self, other: Fe) -> Fe {
        // Twice the prime, so that the subtraction cannot go negative.
        const TWICE: [u64; 5] = [
            0x000f_ffff_ffff_ffda,
            0x000f_ffff_ffff_fffe,
            0x000f_ffff_ffff_fffe,
            0x000f_ffff_ffff_fffe,
            0x000f_ffff_ffff_fffe,
        ];
        let mut result = [0u64; 5];
        for (slot, ((left, right), twice)) in
            result.iter_mut().zip(self.0.iter().zip(other.0).zip(TWICE))
        {
            *slot = left.wrapping_add(twice).wrapping_sub(right);
        }
        Fe(result).carried()
    }

    /// The negation.
    #[must_use]
    pub fn negate(self) -> Fe {
        Fe::ZERO.sub(self)
    }

    /// The product.
    #[must_use]
    #[expect(
        clippy::should_implement_trait,
        reason = "the operator traits would invite `+` and `*` at every call site, where the workspace\
                  denies unchecked arithmetic; field operations are named after what they are"
    )]
    pub fn mul(self, other: Fe) -> Fe {
        let [f0, f1, f2, f3, f4] = self.0;
        let [g0, g1, g2, g3, g4] = other.0;
        let g1_19 = g1.wrapping_mul(19);
        let g2_19 = g2.wrapping_mul(19);
        let g3_19 = g3.wrapping_mul(19);
        let g4_19 = g4.wrapping_mul(19);

        Fe::from_wide([
            product(f0, g0)
                .wrapping_add(product(f1, g4_19))
                .wrapping_add(product(f2, g3_19))
                .wrapping_add(product(f3, g2_19))
                .wrapping_add(product(f4, g1_19)),
            product(f0, g1)
                .wrapping_add(product(f1, g0))
                .wrapping_add(product(f2, g4_19))
                .wrapping_add(product(f3, g3_19))
                .wrapping_add(product(f4, g2_19)),
            product(f0, g2)
                .wrapping_add(product(f1, g1))
                .wrapping_add(product(f2, g0))
                .wrapping_add(product(f3, g4_19))
                .wrapping_add(product(f4, g3_19)),
            product(f0, g3)
                .wrapping_add(product(f1, g2))
                .wrapping_add(product(f2, g1))
                .wrapping_add(product(f3, g0))
                .wrapping_add(product(f4, g4_19)),
            product(f0, g4)
                .wrapping_add(product(f1, g3))
                .wrapping_add(product(f2, g2))
                .wrapping_add(product(f3, g1))
                .wrapping_add(product(f4, g0)),
        ])
    }

    /// The square. The same terms as [`Fe::mul`] with itself, with the
    /// pairs that appear twice folded together.
    #[must_use]
    pub fn square(self) -> Fe {
        let [f0, f1, f2, f3, f4] = self.0;
        let f0_2 = f0.wrapping_mul(2);
        let f1_2 = f1.wrapping_mul(2);
        let f2_2 = f2.wrapping_mul(2);
        let f3_2 = f3.wrapping_mul(2);
        let f3_19 = f3.wrapping_mul(19);
        let f4_19 = f4.wrapping_mul(19);

        Fe::from_wide([
            product(f0, f0)
                .wrapping_add(product(f1_2, f4_19))
                .wrapping_add(product(f2_2, f3_19)),
            product(f0_2, f1)
                .wrapping_add(product(f2_2, f4_19))
                .wrapping_add(product(f3, f3_19)),
            product(f0_2, f2)
                .wrapping_add(product(f1, f1))
                .wrapping_add(product(f3_2, f4_19)),
            product(f0_2, f3)
                .wrapping_add(product(f1_2, f2))
                .wrapping_add(product(f4, f4_19)),
            product(f0_2, f4)
                .wrapping_add(product(f1_2, f3))
                .wrapping_add(product(f2, f2)),
        ])
    }

    /// The product with `a24`, the constant of the Montgomery ladder,
    /// which RFC 7748 defines as `(486662 - 2) / 4`.
    #[must_use]
    pub fn mul_a24(self) -> Fe {
        let mut wide = [0u128; 5];
        for (slot, limb) in wide.iter_mut().zip(self.0) {
            *slot = product(limb, 121_665);
        }
        Fe::from_wide(wide)
    }

    /// The square applied `count` times.
    #[must_use]
    pub fn square_times(self, count: u32) -> Fe {
        let mut result = self;
        for _ in 0..count {
            result = result.square();
        }
        result
    }

    /// The multiplicative inverse, as the power `p - 2`. The inverse of
    /// zero is zero, which the callers of this function all check for
    /// separately.
    #[must_use]
    pub fn invert(self) -> Fe {
        let (t, z11) = self.power_chain();
        // The chain leaves `self^(2^250 - 1)`; five more squarings and a
        // multiplication by `self^11` give `self^(2^255 - 21)`.
        t.square_times(5).mul(z11)
    }

    /// The power `(p - 5) / 8`, which is `2^252 - 3`, for the square root
    /// that decompresses an Edwards point.
    #[must_use]
    pub fn pow22523(self) -> Fe {
        let (t, _) = self.power_chain();
        t.square_times(2).mul(self)
    }

    /// Whether the element is zero, in its canonical encoding.
    #[must_use]
    pub fn is_zero(self) -> Choice {
        let bytes = self.to_bytes();
        let mut difference = 0u8;
        for byte in bytes {
            difference |= byte;
        }
        Choice::is_zero_u8(difference)
    }

    /// The lowest bit of the canonical encoding, which is the sign
    /// convention of Ed25519.
    #[must_use]
    pub fn is_negative(self) -> Choice {
        let [low, ..] = self.to_bytes();
        Choice::from_lsb(low)
    }

    /// `a` when `choice` is true, `b` otherwise.
    #[must_use]
    pub fn select(choice: Choice, a: Fe, b: Fe) -> Fe {
        let mut result = [0u64; 5];
        for (slot, (left, right)) in result.iter_mut().zip(a.0.iter().zip(b.0)) {
            *slot = ct_select_u64(choice, *left, right);
        }
        Fe(result)
    }

    /// Exchanges `a` and `b` when `choice` is true.
    pub fn swap(choice: Choice, a: &mut Fe, b: &mut Fe) {
        let first = Fe::select(choice, *b, *a);
        let second = Fe::select(choice, *a, *b);
        *a = first;
        *b = second;
    }

    /// The common part of the two exponentiations: `self^(2^250 - 1)`, and
    /// `self^11` on the way, which the inversion needs again.
    #[expect(
        clippy::similar_names,
        reason = "each name is the exponent it holds, 2^n - 2^m, which is how the chain is \
                  written down everywhere it is described"
    )]
    fn power_chain(self) -> (Fe, Fe) {
        let z2 = self.square();
        let z9 = z2.square_times(2).mul(self);
        let z11 = z9.mul(z2);
        let z2_5_0 = z11.square().mul(z9);
        let z2_10_0 = z2_5_0.square_times(5).mul(z2_5_0);
        let z2_20_0 = z2_10_0.square_times(10).mul(z2_10_0);
        let z2_40_0 = z2_20_0.square_times(20).mul(z2_20_0);
        let z2_50_0 = z2_40_0.square_times(10).mul(z2_10_0);
        let z2_100_0 = z2_50_0.square_times(50).mul(z2_50_0);
        let z2_200_0 = z2_100_0.square_times(100).mul(z2_100_0);
        let z2_250_0 = z2_200_0.square_times(50).mul(z2_50_0);
        (z2_250_0, z11)
    }

    /// Carries the limbs back into their width.
    const fn carried(self) -> Fe {
        let mut h = self.0;
        let mut carry = h[0] >> 51;
        h[0] &= MASK;
        h[1] = h[1].wrapping_add(carry);
        carry = h[1] >> 51;
        h[1] &= MASK;
        h[2] = h[2].wrapping_add(carry);
        carry = h[2] >> 51;
        h[2] &= MASK;
        h[3] = h[3].wrapping_add(carry);
        carry = h[3] >> 51;
        h[3] &= MASK;
        h[4] = h[4].wrapping_add(carry);
        carry = h[4] >> 51;
        h[4] &= MASK;
        h[0] = h[0].wrapping_add(carry.wrapping_mul(19));
        h[1] = h[1].wrapping_add(h[0] >> 51);
        h[0] &= MASK;
        Fe(h)
    }

    /// Carries a set of wide accumulators down into limbs.
    fn from_wide(wide: [u128; 5]) -> Fe {
        let mut h = wide;
        let mut carry = low64(h[0] >> 51);
        let r0 = low64(h[0]) & MASK;
        h[1] = h[1].wrapping_add(u128::from(carry));
        carry = low64(h[1] >> 51);
        let r1 = low64(h[1]) & MASK;
        h[2] = h[2].wrapping_add(u128::from(carry));
        carry = low64(h[2] >> 51);
        let r2 = low64(h[2]) & MASK;
        h[3] = h[3].wrapping_add(u128::from(carry));
        carry = low64(h[3] >> 51);
        let r3 = low64(h[3]) & MASK;
        h[4] = h[4].wrapping_add(u128::from(carry));
        carry = low64(h[4] >> 51);
        let r4 = low64(h[4]) & MASK;

        let mut limbs = [r0.wrapping_add(carry.wrapping_mul(19)), r1, r2, r3, r4];
        limbs[1] = limbs[1].wrapping_add(limbs[0] >> 51);
        limbs[0] &= MASK;
        Fe(limbs)
    }
}

/// The product of two limbs, which needs more than sixty-four bits.
#[expect(clippy::as_conversions, reason = "widening casts in a const fn")]
const fn product(a: u64, b: u64) -> u128 {
    (a as u128).wrapping_mul(b as u128)
}

/// The low sixty-four bits of a wide value.
#[expect(clippy::as_conversions, reason = "the mask keeps only the low 64 bits")]
const fn low64(value: u128) -> u64 {
    (value & 0xFFFF_FFFF_FFFF_FFFF) as u64
}
