// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The group, the two operations of the exchange over it, and the range
//! check that has to happen before a received value is used.

use crypto_bignum::{MAX_BYTES, Modulus};
use crypto_ct::{Choice, ct_eq};

use crate::error::DhError;

/// A MODP group: a prime modulus and a generator, with the constants
/// Montgomery arithmetic derives from the prime.
///
/// The type is written for a group that arrives as bytes rather than for
/// one group alone, so that the groups of RFC 3526 the arithmetic is wide
/// enough for are a constant away from each other. [`crate::group14`]
/// builds the one SSH requires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModpGroup {
    /// The prime, and what reducing modulo it needs.
    prime: Modulus,
    /// `p-1`, right-aligned in [`MAX_BYTES`] bytes. It is the exclusive
    /// upper bound of every public value and of every shared secret, and
    /// it is stored rather than computed because both checks want it.
    upper: [u8; MAX_BYTES],
    /// Bytes a value of this group occupies.
    width: usize,
    /// The generator.
    generator: u8,
}

impl ModpGroup {
    /// The group of the big-endian `prime` and of `generator`.
    ///
    /// The generator is a small integer because every group of RFC 3526
    /// has two as its generator. Its upper bound needs no check: the
    /// prime has a non-zero top limb, so it is at least `2^63`, and a
    /// byte cannot reach `p-1`.
    ///
    /// # Errors
    ///
    /// [`DhError::InvalidGenerator`] when the generator is below two, and
    /// [`DhError::InvalidPrime`] when the prime is even, zero, carries
    /// leading zero limbs, or is wider than the arithmetic allows.
    pub fn new(prime: &[u8], generator: u8) -> Result<ModpGroup, DhError> {
        if generator < 2 {
            return Err(DhError::InvalidGenerator);
        }
        let modulus = Modulus::new(prime).map_err(|_| DhError::InvalidPrime)?;
        let mut upper = [0u8; MAX_BYTES];
        fill_right(prime, &mut upper);
        // The prime is odd — `Modulus::new` refuses an even one — so
        // `p-1` is the prime with its lowest bit cleared.
        for slot in upper.iter_mut().rev().take(1) {
            *slot &= 0xFE;
        }
        Ok(ModpGroup {
            prime: modulus,
            upper,
            width: modulus.width(),
            generator,
        })
    }

    /// Bytes a public value and a shared secret of this group occupy,
    /// which is the shortest output buffer the two operations take.
    #[must_use]
    pub const fn public_len(&self) -> usize {
        self.width
    }

    /// Whether `value` may be used as a public value of this group.
    ///
    /// RFC 8268, section 4, amends RFC 4253, section 8, which had the
    /// range wrong: both `1 < e < p-1` and `1 < f < p-1` MUST hold, and
    /// the key exchange fails otherwise. A value of one or of `p-1` puts
    /// the exchange in a subgroup of one or two elements, where the
    /// shared secret is known to anyone watching.
    ///
    /// `value` is big-endian and may carry leading zero bytes, which is
    /// what an SSH `mpint` looks like with its length prefix removed.
    ///
    /// The comparisons are ordinary ones and not constant-time ones. What
    /// they judge came from the network, and so does the refusal.
    ///
    /// # Errors
    ///
    /// [`DhError::PublicValueOutOfRange`] when the value is outside that
    /// interval, and when it carries a non-zero byte above the widest
    /// value the arithmetic holds.
    pub fn check_public(&self, value: &[u8]) -> Result<(), DhError> {
        let padded = padded(value).ok_or(DhError::PublicValueOutOfRange)?;
        if padded <= one() || padded >= self.upper {
            return Err(DhError::PublicValueOutOfRange);
        }
        Ok(())
    }

    /// The public value of `secret`: the generator raised to it, written
    /// big-endian into `out`, right-aligned and padded with leading
    /// zeros.
    ///
    /// `secret` is the private exponent and is the only secret here. Its
    /// length is not: it sets the number of rounds the ladder runs, eight
    /// per byte. See `GROUP14_SECRET_BYTES` for what to give it.
    ///
    /// # Errors
    ///
    /// [`DhError::OutputTooShort`] when `out` is shorter than
    /// [`ModpGroup::public_len`] — the only way the exponentiation can
    /// refuse, since the generator is a byte and every byte is below the
    /// prime. [`DhError::PublicValueOutOfRange`] when the value the
    /// secret produces is one or `p-1`; a secret of zero does that, and
    /// such a value must not be sent.
    pub fn public_value(&self, secret: &[u8], out: &mut [u8]) -> Result<(), DhError> {
        self.prime
            .pow_secret(&[self.generator], secret, out)
            .map_err(|_| DhError::OutputTooShort)?;
        self.check_public(out)
    }

    /// The shared secret of `secret` and the peer's public value: the
    /// peer's value raised to the secret, written big-endian into `out`,
    /// right-aligned and padded with leading zeros.
    ///
    /// The peer's value is checked before it is used, which is what makes
    /// the exponentiation safe; the result is checked after, because a
    /// degenerate exponent can still reach a value neither side may key
    /// from. That second check is constant time: the shared secret is
    /// secret, and only the refusal is public.
    ///
    /// # Errors
    ///
    /// [`DhError::PublicValueOutOfRange`] when the peer's value fails
    /// [`ModpGroup::check_public`], [`DhError::OutputTooShort`] when
    /// `out` is shorter than [`ModpGroup::public_len`], and
    /// [`DhError::DegenerateSharedSecret`] when the result is zero, one,
    /// or `p-1`.
    pub fn shared_secret(&self, secret: &[u8], peer: &[u8], out: &mut [u8]) -> Result<(), DhError> {
        self.check_public(peer)?;
        self.prime
            .pow_secret(peer, secret, out)
            .map_err(|_| DhError::OutputTooShort)?;
        if self.degenerate(out).is_true() {
            return Err(DhError::DegenerateSharedSecret);
        }
        Ok(())
    }

    /// Whether `value` is zero, one, or `p-1`.
    ///
    /// The exponentiation has already reduced the value modulo the prime,
    /// so those three are all that is left to exclude to land back in the
    /// open interval `1 < v < p-1`. Every byte of the three comparisons
    /// is read whatever the value is, and the answer is a [`Choice`]
    /// rather than a branch; the caller turns it into one, which is the
    /// deliberate exit from constant time and is allowed because the
    /// refusal aborts the exchange in the open.
    ///
    /// Bytes of `value` above [`MAX_BYTES`] are dropped. The
    /// exponentiation zero-fills what it writes, so they are the leading
    /// zeros of a value that fits.
    fn degenerate(&self, value: &[u8]) -> Choice {
        let mut buffer = [0u8; MAX_BYTES];
        fill_right(value, &mut buffer);
        ct_eq(&buffer, &[0u8; MAX_BYTES]) | ct_eq(&buffer, &one()) | ct_eq(&buffer, &self.upper)
    }
}

/// Writes `value` right-aligned into `buffer` with leading zeros, and
/// drops the bytes that do not fit.
fn fill_right(value: &[u8], buffer: &mut [u8; MAX_BYTES]) {
    buffer.fill(0);
    let mut source = value.iter().rev().copied();
    for (slot, byte) in buffer.iter_mut().rev().zip(source.by_ref()) {
        *slot = byte;
    }
}

/// `value` right-aligned in [`MAX_BYTES`] bytes, or `None` when it does
/// not fit there and so is no value of any group this arithmetic holds.
///
/// A value longer than [`MAX_BYTES`] still fits when what hangs over the
/// front is zeros, which is the case that matters: an SSH `mpint` carries
/// a leading zero byte whenever the top bit of the value is set, and a
/// caller may hand over a buffer wider than the group.
fn padded(value: &[u8]) -> Option<[u8; MAX_BYTES]> {
    let mut buffer = [0u8; MAX_BYTES];
    let mut source = value.iter().rev().copied();
    for (slot, byte) in buffer.iter_mut().rev().zip(source.by_ref()) {
        *slot = byte;
    }
    if source.any(|byte| byte != 0) {
        return None;
    }
    Some(buffer)
}

/// The value one, right-aligned in [`MAX_BYTES`] bytes.
fn one() -> [u8; MAX_BYTES] {
    core::array::from_fn(|index| u8::from(index == MAX_BYTES.saturating_sub(1)))
}
