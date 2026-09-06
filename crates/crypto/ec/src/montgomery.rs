// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Arithmetic modulo a prime given as limbs, in Montgomery form.
//!
//! Two curves use this, and each uses it twice: for the field the curve
//! lives in and for the order of its group. Neither order has a shape a
//! Solinas reduction can use, so one Montgomery multiplication serves all
//! four moduli, parameterized by [`Params`]. A value is held as
//! `x * 2^(64*N)` modulo the parameter, and only the encodings convert.
//!
//! The limb arithmetic itself is `crypto-bignum` and no longer lives
//! here. What stays is what the curves need and RSA does not: a modulus
//! whose three constants are known when the code is compiled, and a type
//! that carries which of the four a value belongs to. D-77 records the
//! split and the reason for it — an RSA modulus arrives in a certificate,
//! so it cannot be an associated constant, and everything below the
//! constants already took its modulus as an argument.
//!
//! Everything here serves signature verification, where the values are
//! public: a key, a digest, and a signature are all on the wire. Its
//! branches may therefore depend on values, and they do, in the final
//! conditional subtraction and in the exponentiation. The constant-time
//! work of this crate is [`mod@crate::x25519`].

use core::marker::PhantomData;

use crypto_bignum::{add_limbs, is_less, montgomery, subtract};

/// The parameters of one modulus, over `N` limbs of sixty-four bits.
pub trait Params<const N: usize>: Copy {
    /// The modulus, least significant limb first.
    const MODULUS: [u64; N];
    /// The negative inverse of the modulus modulo `2^64`, which is what
    /// the reduction step multiplies by.
    const N0INV: u64;
    /// `2^(128*N)` modulo the modulus, which converts into Montgomery
    /// form.
    const R2: [u64; N];
    /// `2^(64*N)` modulo the modulus, which is the Montgomery form of one.
    const ONE: [u64; N];
}

/// An element of the field `P`, in Montgomery form.
///
/// `BYTES` is the width of the canonical encoding, and must be `8 * N`. A
/// pair that says otherwise does not compile.
pub struct Element<P: Params<N>, const N: usize, const BYTES: usize> {
    /// The value times `2^(64*N)`, modulo the parameter.
    limbs: [u64; N],
    /// Which modulus this element belongs to.
    marker: PhantomData<P>,
}

/// The element is limbs and a marker, so it copies. Both are written out
/// rather than derived, because a derive would demand the parameter be
/// `Clone` and `Copy` itself, which it need not be.
impl<P: Params<N>, const N: usize, const BYTES: usize> Copy for Element<P, N, BYTES> {}

#[expect(
    clippy::expl_impl_clone_on_copy,
    reason = "a derive would add a bound on the parameter that the type does not need"
)]
impl<P: Params<N>, const N: usize, const BYTES: usize> Clone for Element<P, N, BYTES> {
    fn clone(&self) -> Element<P, N, BYTES> {
        *self
    }
}

impl<P: Params<N>, const N: usize, const BYTES: usize> PartialEq for Element<P, N, BYTES> {
    fn eq(&self, other: &Element<P, N, BYTES>) -> bool {
        self.limbs == other.limbs
    }
}

impl<P: Params<N>, const N: usize, const BYTES: usize> Eq for Element<P, N, BYTES> {}

impl<P: Params<N>, const N: usize, const BYTES: usize> core::fmt::Debug for Element<P, N, BYTES> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Element")
    }
}

impl<P: Params<N>, const N: usize, const BYTES: usize> Element<P, N, BYTES> {
    /// The encoding is exactly as wide as the limbs.
    ///
    /// Every conversion reads this, so instantiating the type with a width
    /// that does not match its limbs is a compile error rather than a
    /// value that silently loses its top bytes.
    const WIDTH_MATCHES_LIMBS: () = assert!(BYTES == N.wrapping_mul(8));

    /// Zero.
    #[must_use]
    pub const fn zero() -> Element<P, N, BYTES> {
        Element {
            limbs: [0; N],
            marker: PhantomData,
        }
    }

    /// One.
    #[must_use]
    pub const fn one() -> Element<P, N, BYTES> {
        Element {
            limbs: P::ONE,
            marker: PhantomData,
        }
    }

    /// An element from limbs that are already in Montgomery form, for the
    /// constants of a curve.
    #[must_use]
    pub const fn from_montgomery(limbs: [u64; N]) -> Element<P, N, BYTES> {
        Element {
            limbs,
            marker: PhantomData,
        }
    }

    /// The element the big-endian bytes encode, or `None` when the value
    /// is at or above the modulus.
    #[must_use]
    pub fn from_canonical(bytes: &[u8; BYTES]) -> Option<Element<P, N, BYTES>> {
        let () = Self::WIDTH_MATCHES_LIMBS;
        let value: [u64; N] = limbs_of(bytes);
        if !is_less(&value, &P::MODULUS) {
            return None;
        }
        let mut limbs = [0u64; N];
        montgomery(&value, &P::R2, &P::MODULUS, P::N0INV, &mut limbs);
        Some(Element {
            limbs,
            marker: PhantomData,
        })
    }

    /// The element the big-endian bytes encode, reduced. A value of the
    /// encoding's own width is below twice any modulus of that width, so
    /// one subtraction is enough.
    #[must_use]
    pub fn from_bytes_reduced(bytes: &[u8; BYTES]) -> Element<P, N, BYTES> {
        let () = Self::WIDTH_MATCHES_LIMBS;
        let mut reduced: [u64; N] = limbs_of(bytes);
        if !is_less(&reduced, &P::MODULUS) {
            let _ = subtract(&mut reduced, &P::MODULUS);
        }
        let mut limbs = [0u64; N];
        montgomery(&reduced, &P::R2, &P::MODULUS, P::N0INV, &mut limbs);
        Element {
            limbs,
            marker: PhantomData,
        }
    }

    /// The canonical big-endian encoding.
    #[must_use]
    pub fn to_bytes(self) -> [u8; BYTES] {
        let () = Self::WIDTH_MATCHES_LIMBS;
        let one: [u64; N] = core::array::from_fn(|index| u64::from(index == 0));
        let mut value = [0u64; N];
        montgomery(&self.limbs, &one, &P::MODULUS, P::N0INV, &mut value);
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
    pub fn add(self, other: Element<P, N, BYTES>) -> Element<P, N, BYTES> {
        let mut sum = self.limbs;
        let carry = add_limbs(&mut sum, &other.limbs);
        let mut difference = sum;
        let borrow = subtract(&mut difference, &P::MODULUS);
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
    pub fn sub(self, other: Element<P, N, BYTES>) -> Element<P, N, BYTES> {
        let mut limbs = self.limbs;
        let borrow = subtract(&mut limbs, &other.limbs);
        if borrow == 1 {
            let _ = add_limbs(&mut limbs, &P::MODULUS);
        }
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
    pub fn neg(self) -> Element<P, N, BYTES> {
        Element::zero().sub(self)
    }

    /// Twice the element.
    #[must_use]
    pub fn double(self) -> Element<P, N, BYTES> {
        self.add(self)
    }

    /// The product.
    #[must_use]
    #[expect(
        clippy::should_implement_trait,
        reason = "the operator traits would invite `+` and `*` at every call site, where the workspace\
                  denies unchecked arithmetic; field operations are named after what they are"
    )]
    pub fn mul(self, other: Element<P, N, BYTES>) -> Element<P, N, BYTES> {
        let mut limbs = [0u64; N];
        montgomery(&self.limbs, &other.limbs, &P::MODULUS, P::N0INV, &mut limbs);
        Element {
            limbs,
            marker: PhantomData,
        }
    }

    /// The square.
    #[must_use]
    pub fn square(self) -> Element<P, N, BYTES> {
        self.mul(self)
    }

    /// The multiplicative inverse, as the power `modulus - 2`. The inverse
    /// of zero is zero, which every caller checks for separately.
    #[must_use]
    pub fn invert(self) -> Element<P, N, BYTES> {
        let two: [u64; N] = core::array::from_fn(|index| u64::from(index == 0).wrapping_mul(2));
        let mut exponent = P::MODULUS;
        let _ = subtract(&mut exponent, &two);
        let bits = u32::try_from(N).unwrap_or(0).wrapping_mul(64);
        let mut result = Element::one();
        for position in (0..bits).rev() {
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
        self.limbs == [0; N]
    }
}

/// The limbs of a big-endian encoding, least significant first.
fn limbs_of<const N: usize, const BYTES: usize>(bytes: &[u8; BYTES]) -> [u64; N] {
    let (chunks, _) = bytes.as_chunks::<8>();
    let mut limbs = [0u64; N];
    for (slot, chunk) in limbs.iter_mut().zip(chunks.iter().rev()) {
        *slot = u64::from_be_bytes(*chunk);
    }
    limbs
}

/// Bit `position` of a value, counted from the least significant.
fn bit_of<const N: usize>(value: [u64; N], position: u32) -> u64 {
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
