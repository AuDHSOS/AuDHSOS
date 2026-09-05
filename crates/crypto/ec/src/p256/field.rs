// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Arithmetic modulo the two primes of P-256: the field of the curve and
//! the order of its group.
//!
//! Both are 256 bits and neither has a shape that a Solinas reduction can
//! use for the second one, so the same Montgomery multiplication serves
//! both, parameterized by the modulus. A value is held in Montgomery form,
//! `x * 2^256` modulo the parameter, and only the encodings convert.
//!
//! Everything in this module serves signature verification, where the
//! values are public: a key, a digest, and a signature are all on the
//! wire. Its branches may therefore depend on values, and they do, in the
//! final conditional subtraction and in the exponentiation. The
//! constant-time work of this crate is [`mod@crate::x25519`].

use core::marker::PhantomData;

/// Bytes of an encoded value.
pub const BYTES: usize = 32;

/// The parameters of one modulus.
pub trait Params: Copy {
    /// The modulus, as four limbs, least significant first.
    const MODULUS: [u64; 4];
    /// The negative inverse of the modulus modulo `2^64`, which is what
    /// the reduction step multiplies by.
    const N0INV: u64;
    /// `2^512` modulo the modulus, which converts into Montgomery form.
    const R2: [u64; 4];
    /// `2^256` modulo the modulus, which is the Montgomery form of one.
    const ONE: [u64; 4];
}

/// The field of the curve: `2^256 - 2^224 + 2^192 + 2^96 - 1`.
#[derive(Clone, Copy, Debug)]
pub struct Prime;

impl Params for Prime {
    const MODULUS: [u64; 4] = [
        0xffff_ffff_ffff_ffff,
        0x0000_0000_ffff_ffff,
        0x0000_0000_0000_0000,
        0xffff_ffff_0000_0001,
    ];
    const N0INV: u64 = 0x0000_0000_0000_0001;
    const R2: [u64; 4] = [
        0x0000_0000_0000_0003,
        0xffff_fffb_ffff_ffff,
        0xffff_ffff_ffff_fffe,
        0x0000_0004_ffff_fffd,
    ];
    const ONE: [u64; 4] = [
        0x0000_0000_0000_0001,
        0xffff_ffff_0000_0000,
        0xffff_ffff_ffff_ffff,
        0x0000_0000_ffff_fffe,
    ];
}

/// The order of the group of the curve.
#[derive(Clone, Copy, Debug)]
pub struct Order;

impl Params for Order {
    const MODULUS: [u64; 4] = [
        0xf3b9_cac2_fc63_2551,
        0xbce6_faad_a717_9e84,
        0xffff_ffff_ffff_ffff,
        0xffff_ffff_0000_0000,
    ];
    const N0INV: u64 = 0xccd1_c8aa_ee00_bc4f;
    const R2: [u64; 4] = [
        0x8324_4c95_be79_eea2,
        0x4699_799c_49bd_6fa6,
        0x2845_b239_2b6b_ec59,
        0x66e1_2d94_f3d9_5620,
    ];
    const ONE: [u64; 4] = [
        0x0c46_353d_039c_daaf,
        0x4319_0552_58e8_617b,
        0x0000_0000_0000_0000,
        0x0000_0000_ffff_ffff,
    ];
}

/// An element of the field `P`, in Montgomery form.
pub struct Element<P: Params> {
    /// The value times `2^256`, modulo the parameter.
    limbs: [u64; 4],
    /// Which modulus this element belongs to.
    marker: PhantomData<P>,
}

/// The element is four limbs and a marker, so it copies. Both are written
/// out rather than derived, because a derive would demand the parameter be
/// `Clone` and `Copy` itself, which it need not be.
impl<P: Params> Copy for Element<P> {}

#[expect(
    clippy::expl_impl_clone_on_copy,
    reason = "a derive would add a bound on the parameter that the type does not need"
)]
impl<P: Params> Clone for Element<P> {
    fn clone(&self) -> Element<P> {
        *self
    }
}

impl<P: Params> PartialEq for Element<P> {
    fn eq(&self, other: &Element<P>) -> bool {
        self.limbs == other.limbs
    }
}

impl<P: Params> Eq for Element<P> {}

impl<P: Params> core::fmt::Debug for Element<P> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Element")
    }
}

impl<P: Params> Element<P> {
    /// Zero.
    #[must_use]
    pub const fn zero() -> Element<P> {
        Element {
            limbs: [0; 4],
            marker: PhantomData,
        }
    }

    /// One.
    #[must_use]
    pub const fn one() -> Element<P> {
        Element {
            limbs: P::ONE,
            marker: PhantomData,
        }
    }

    /// An element from limbs that are already in Montgomery form, for the
    /// constants of the curve.
    #[must_use]
    pub const fn from_montgomery(limbs: [u64; 4]) -> Element<P> {
        Element {
            limbs,
            marker: PhantomData,
        }
    }

    /// The element the big-endian bytes encode, or `None` when the value
    /// is at or above the modulus.
    #[must_use]
    pub fn from_canonical(bytes: &[u8; BYTES]) -> Option<Element<P>> {
        let value = limbs_of(bytes);
        let (_, borrow) = subtract(value, P::MODULUS);
        if borrow == 0 {
            return None;
        }
        Some(Element {
            limbs: montgomery(value, P::R2, P::MODULUS, P::N0INV),
            marker: PhantomData,
        })
    }

    /// The element the big-endian bytes encode, reduced. A 256-bit value
    /// is below twice either modulus, so one subtraction is enough.
    #[must_use]
    pub fn from_bytes_reduced(bytes: &[u8; BYTES]) -> Element<P> {
        let value = limbs_of(bytes);
        let (difference, borrow) = subtract(value, P::MODULUS);
        let reduced = if borrow == 0 { difference } else { value };
        Element {
            limbs: montgomery(reduced, P::R2, P::MODULUS, P::N0INV),
            marker: PhantomData,
        }
    }

    /// The canonical big-endian encoding.
    #[must_use]
    pub fn to_bytes(self) -> [u8; BYTES] {
        let value = montgomery(self.limbs, [1, 0, 0, 0], P::MODULUS, P::N0INV);
        let mut bytes = [0u8; BYTES];
        let (chunks, _) = bytes.as_chunks_mut::<8>();
        for (chunk, limb) in chunks.iter_mut().rev().zip(value) {
            *chunk = limb.to_be_bytes();
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
    pub fn add(self, other: Element<P>) -> Element<P> {
        let (sum, carry) = add_limbs(self.limbs, other.limbs);
        let (difference, borrow) = subtract(sum, P::MODULUS);
        let limbs = if carry == 1 || borrow == 0 {
            difference
        } else {
            sum
        };
        Element {
            limbs,
            marker: PhantomData,
        }
    }

    /// The difference.
    #[must_use]
    #[expect(
        clippy::should_implement_trait,
        reason = "the operator traits would invite `+` and `*` at every call site, where the workspace\
                  denies unchecked arithmetic; field operations are named after what they are"
    )]
    pub fn sub(self, other: Element<P>) -> Element<P> {
        let (difference, borrow) = subtract(self.limbs, other.limbs);
        let limbs = if borrow == 1 {
            let (wrapped, _) = add_limbs(difference, P::MODULUS);
            wrapped
        } else {
            difference
        };
        Element {
            limbs,
            marker: PhantomData,
        }
    }

    /// The negation.
    #[must_use]
    #[expect(
        clippy::should_implement_trait,
        reason = "the operator traits would invite `+` and `*` at every call site, where the workspace\
                  denies unchecked arithmetic; field operations are named after what they are"
    )]
    pub fn neg(self) -> Element<P> {
        Element::zero().sub(self)
    }

    /// Twice the element.
    #[must_use]
    pub fn double(self) -> Element<P> {
        self.add(self)
    }

    /// The product.
    #[must_use]
    #[expect(
        clippy::should_implement_trait,
        reason = "the operator traits would invite `+` and `*` at every call site, where the workspace\
                  denies unchecked arithmetic; field operations are named after what they are"
    )]
    pub fn mul(self, other: Element<P>) -> Element<P> {
        Element {
            limbs: montgomery(self.limbs, other.limbs, P::MODULUS, P::N0INV),
            marker: PhantomData,
        }
    }

    /// The square.
    #[must_use]
    pub fn square(self) -> Element<P> {
        self.mul(self)
    }

    /// The multiplicative inverse, as the power `modulus - 2`. The inverse
    /// of zero is zero, which every caller checks for separately.
    #[must_use]
    pub fn invert(self) -> Element<P> {
        let (exponent, _) = subtract(P::MODULUS, [2, 0, 0, 0]);
        let mut result = Element::one();
        for position in (0..256u32).rev() {
            result = result.square();
            if bit_of(exponent, position) == 1 {
                result = result.mul(self);
            }
        }
        result
    }

    /// Whether the element is zero.
    #[must_use]
    pub fn is_zero(self) -> bool {
        self.limbs == [0; 4]
    }
}

/// The limbs of a big-endian encoding, least significant first.
fn limbs_of(bytes: &[u8; BYTES]) -> [u64; 4] {
    let (chunks, _) = bytes.as_chunks::<8>();
    let mut limbs = [0u64; 4];
    for (slot, chunk) in limbs.iter_mut().zip(chunks.iter().rev()) {
        *slot = u64::from_be_bytes(*chunk);
    }
    limbs
}

/// Bit `position` of a four-limb value.
fn bit_of(value: [u64; 4], position: u32) -> u64 {
    let index = position.wrapping_shr(6);
    let within = position & 63;
    let mut limb = 0u64;
    for (slot, candidate) in (0u32..).zip(value) {
        if slot == index {
            limb = candidate;
        }
    }
    limb.wrapping_shr(within) & 1
}

/// The sum with the carry out.
fn add_limbs(a: [u64; 4], b: [u64; 4]) -> ([u64; 4], u64) {
    let mut result = [0u64; 4];
    let mut carry = 0u64;
    for (slot, (left, right)) in result.iter_mut().zip(a.iter().zip(b)) {
        let sum = u128::from(*left)
            .wrapping_add(u128::from(right))
            .wrapping_add(u128::from(carry));
        *slot = low(sum);
        carry = high(sum);
    }
    (result, carry)
}

/// The difference with the borrow out.
fn subtract(a: [u64; 4], b: [u64; 4]) -> ([u64; 4], u64) {
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

/// The Montgomery product: `a * b * 2^-256` modulo `modulus`, by the
/// coarsely integrated operand scanning method.
fn montgomery(a: [u64; 4], b: [u64; 4], modulus: [u64; 4], n0inv: u64) -> [u64; 4] {
    let mut t = [0u64; 6];
    for factor in b {
        // t += a * factor
        let mut carry = 0u64;
        for (slot, limb) in t.iter_mut().zip(a) {
            let sum = u128::from(*slot)
                .wrapping_add(u128::from(limb).wrapping_mul(u128::from(factor)))
                .wrapping_add(u128::from(carry));
            *slot = low(sum);
            carry = high(sum);
        }
        let sum = u128::from(t[4]).wrapping_add(u128::from(carry));
        t[4] = low(sum);
        t[5] = high(sum);

        // t += m * modulus, chosen so that the lowest limb becomes zero
        let m = t[0].wrapping_mul(n0inv);
        let mut carry = 0u64;
        for (slot, limb) in t.iter_mut().zip(modulus) {
            let sum = u128::from(*slot)
                .wrapping_add(u128::from(m).wrapping_mul(u128::from(limb)))
                .wrapping_add(u128::from(carry));
            *slot = low(sum);
            carry = high(sum);
        }
        let sum = u128::from(t[4]).wrapping_add(u128::from(carry));
        t[4] = low(sum);
        t[5] = t[5].wrapping_add(high(sum));

        // Divide by 2^64: the lowest limb is zero and rotates to the top.
        t.rotate_left(1);
    }

    let result = [t[0], t[1], t[2], t[3]];
    let (difference, borrow) = subtract(result, modulus);
    if t[4] != 0 || borrow == 0 {
        difference
    } else {
        result
    }
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
