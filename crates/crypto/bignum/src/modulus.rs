// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A modulus that arrives at run time, with the two constants Montgomery
//! arithmetic needs derived from it, and exponentiation over it.

use crypto_ct::{Choice, ct_swap_u64};

use crate::error::BignumError;
use crate::limbs::{is_less, montgomery, montgomery_secret, shift_left_one, subtract};

/// Limbs of sixty-four bits a value occupies: four thousand and
/// ninety-six bits, which is the widest key this system accepts.
pub const MAX_LIMBS: usize = 64;

/// Bytes a value of [`MAX_LIMBS`] limbs occupies.
pub const MAX_BYTES: usize = MAX_LIMBS.saturating_mul(8);

/// An odd modulus of at most [`MAX_LIMBS`] limbs, with the two constants
/// Montgomery arithmetic reduces by.
///
/// Limbs at or above `used` are zero, in this value and in every value
/// derived from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Modulus {
    /// The modulus, least significant limb first.
    pub(crate) limbs: [u64; MAX_LIMBS],
    /// How many limbs the modulus occupies.
    pub(crate) used: usize,
    /// The negative inverse of the low limb modulo `2^64`, which is what
    /// the reduction step multiplies by.
    pub(crate) n0inv: u64,
    /// `2^(128*used)` modulo the modulus, which converts into Montgomery
    /// form.
    pub(crate) r2: [u64; MAX_LIMBS],
}

impl Modulus {
    /// The modulus the big-endian bytes encode.
    ///
    /// # Errors
    ///
    /// [`BignumError::TooWide`] when the encoding needs more than
    /// [`MAX_LIMBS`] limbs, [`BignumError::NotNormalized`] when the value
    /// is zero or its most significant limb is, and
    /// [`BignumError::Even`] when it is even.
    pub fn new(big_endian: &[u8]) -> Result<Modulus, BignumError> {
        let mut limbs = [0u64; MAX_LIMBS];
        let used = read_be(big_endian, &mut limbs)?;
        if prefix(&limbs, used).last().copied().unwrap_or(0) == 0 {
            return Err(BignumError::NotNormalized);
        }
        let low = limbs.first().copied().unwrap_or(0);
        if low & 1 == 0 {
            return Err(BignumError::Even);
        }
        let n0inv = n0inv_of(low);
        let r2 = r2_of(&limbs, used);
        Ok(Modulus {
            limbs,
            used,
            n0inv,
            r2,
        })
    }

    /// The bit length of the modulus, which is what an encoding rule that
    /// speaks of `modBits` means.
    #[must_use]
    pub fn bits(&self) -> usize {
        let top = prefix(&self.limbs, self.used).last().copied().unwrap_or(0);
        let significant =
            64usize.saturating_sub(usize::try_from(top.leading_zeros()).unwrap_or(64));
        self.used
            .saturating_sub(1)
            .saturating_mul(64)
            .saturating_add(significant)
    }

    /// Bytes this modulus occupies when it is written to whole limbs.
    ///
    /// Every value below the modulus fits in that many bytes, so this is
    /// the width [`Modulus::pow_secret`] writes at and the shortest `out`
    /// it accepts.
    #[must_use]
    pub const fn width(&self) -> usize {
        self.used.saturating_mul(8)
    }

    /// `base^exponent` modulo this modulus, written big-endian into `out`
    /// with leading zeros.
    ///
    /// This is the verification direction: an RSA public exponent is
    /// three or sixty-five thousand five hundred and thirty-seven, and
    /// both fit here.
    ///
    /// # Errors
    ///
    /// [`BignumError::TooWide`] when the base needs more than
    /// [`MAX_LIMBS`] limbs, [`BignumError::OutOfRange`] when it is not
    /// below the modulus, and [`BignumError::OutputTooShort`] when the
    /// result does not fit in `out`.
    pub fn pow(&self, base: &[u8], exponent: u64, out: &mut [u8]) -> Result<(), BignumError> {
        self.pow_wide(base, &exponent.to_be_bytes(), out)
    }

    /// `base^exponent` modulo this modulus for an exponent as wide as the
    /// modulus, written big-endian into `out` with leading zeros.
    ///
    /// Nothing in the product calls this: a public exponent fits in
    /// [`Modulus::pow`]. It is here because the private exponent of a
    /// fixed test key does not, and signing a test certificate is that
    /// one call.
    ///
    /// # Errors
    ///
    /// As [`Modulus::pow`].
    pub fn pow_wide(
        &self,
        base: &[u8],
        exponent: &[u8],
        out: &mut [u8],
    ) -> Result<(), BignumError> {
        let mut value = [0u64; MAX_LIMBS];
        let _ = read_be(base, &mut value)?;
        if !is_less(&value, &self.limbs) {
            return Err(BignumError::OutOfRange);
        }

        let used = self.used;
        let modulus = prefix(&self.limbs, used);
        let r2 = prefix(&self.r2, used);
        let unit = one();

        let mut base_mont = [0u64; MAX_LIMBS];
        montgomery(
            prefix(&value, used),
            r2,
            modulus,
            self.n0inv,
            prefix_mut(&mut base_mont, used),
        );

        let mut result = [0u64; MAX_LIMBS];
        let mut scratch = [0u64; MAX_LIMBS];
        match bit_length(exponent).checked_sub(1) {
            // An exponent of zero: the answer is one, in Montgomery form.
            None => montgomery(
                prefix(&unit, used),
                r2,
                modulus,
                self.n0inv,
                prefix_mut(&mut result, used),
            ),
            // Left to right, starting at the highest set bit, which is
            // the multiplication that would otherwise be by one.
            Some(top) => {
                result = base_mont;
                for position in (0..top).rev() {
                    montgomery(
                        prefix(&result, used),
                        prefix(&result, used),
                        modulus,
                        self.n0inv,
                        prefix_mut(&mut scratch, used),
                    );
                    core::mem::swap(&mut result, &mut scratch);
                    if bit_at(exponent, position) {
                        montgomery(
                            prefix(&result, used),
                            prefix(&base_mont, used),
                            modulus,
                            self.n0inv,
                            prefix_mut(&mut scratch, used),
                        );
                        core::mem::swap(&mut result, &mut scratch);
                    }
                }
            }
        }

        montgomery(
            prefix(&result, used),
            prefix(&unit, used),
            modulus,
            self.n0inv,
            prefix_mut(&mut scratch, used),
        );
        write_be(prefix(&scratch, used), out)
    }

    /// `base^exponent` modulo this modulus for an exponent that must not
    /// be observable, written big-endian into `out` with leading zeros.
    ///
    /// This is the Diffie-Hellman direction, where the base and the
    /// modulus are public and the exponent is the private value. It is a
    /// Montgomery ladder: two working values whose invariant is that the
    /// second is the first times the base, one squaring and one
    /// multiplication per bit whatever that bit is, and a masked exchange
    /// rather than a branch to decide which of the two the bit puts in
    /// front. The products go through [`montgomery_secret`], whose final
    /// subtraction is masked as well, so no branch and no memory index in
    /// the whole loop depends on a bit of the exponent or on a limb of an
    /// intermediate value.
    ///
    /// What is public is the length of `exponent`, because it is a buffer
    /// length and it sets the number of rounds: eight per byte, leading
    /// zero bytes included. A caller that wants a shorter exponent passes
    /// a shorter slice rather than a padded one.
    ///
    /// # Errors
    ///
    /// [`BignumError::TooWide`] when the base needs more than
    /// [`MAX_LIMBS`] limbs, [`BignumError::OutOfRange`] when it is not
    /// below the modulus, and [`BignumError::OutputTooShort`] when `out`
    /// is narrower than [`Modulus::width`]. All three are decided before
    /// the ladder runs and none of them looks at the exponent.
    pub fn pow_secret(
        &self,
        base: &[u8],
        exponent: &[u8],
        out: &mut [u8],
    ) -> Result<(), BignumError> {
        let mut value = [0u64; MAX_LIMBS];
        let _ = read_be(base, &mut value)?;
        if !is_less(&value, &self.limbs) {
            return Err(BignumError::OutOfRange);
        }
        if out.len() < self.width() {
            return Err(BignumError::OutputTooShort);
        }

        let used = self.used;
        let modulus = prefix(&self.limbs, used);
        let r2 = prefix(&self.r2, used);
        let unit = one();

        // `ahead` is `power` times the base, and stays so.
        let mut power = [0u64; MAX_LIMBS];
        montgomery(
            prefix(&unit, used),
            r2,
            modulus,
            self.n0inv,
            prefix_mut(&mut power, used),
        );
        let mut ahead = [0u64; MAX_LIMBS];
        montgomery(
            prefix(&value, used),
            r2,
            modulus,
            self.n0inv,
            prefix_mut(&mut ahead, used),
        );

        let mut product = [0u64; MAX_LIMBS];
        let mut square = [0u64; MAX_LIMBS];
        for (index, _) in exponent.iter().enumerate() {
            for shift in (0u32..8).rev() {
                let bit = bit_of(exponent, index, shift);
                ct_swap_u64(bit, &mut power, &mut ahead);
                montgomery_secret(
                    prefix(&power, used),
                    prefix(&ahead, used),
                    modulus,
                    self.n0inv,
                    prefix_mut(&mut product, used),
                );
                montgomery_secret(
                    prefix(&power, used),
                    prefix(&power, used),
                    modulus,
                    self.n0inv,
                    prefix_mut(&mut square, used),
                );
                power = square;
                ahead = product;
                ct_swap_u64(bit, &mut power, &mut ahead);
            }
        }

        let mut result = [0u64; MAX_LIMBS];
        montgomery_secret(
            prefix(&power, used),
            prefix(&unit, used),
            modulus,
            self.n0inv,
            prefix_mut(&mut result, used),
        );
        write_be(prefix(&result, used), out)
    }
}

/// The first `count` limbs, or every limb there is when there are fewer.
fn prefix(limbs: &[u64], count: usize) -> &[u64] {
    limbs.split_at(count.min(limbs.len())).0
}

/// The first `count` limbs, or every limb there is when there are fewer.
fn prefix_mut(limbs: &mut [u64], count: usize) -> &mut [u64] {
    let count = count.min(limbs.len());
    limbs.split_at_mut(count).0
}

/// The value one.
fn one() -> [u64; MAX_LIMBS] {
    core::array::from_fn(|index| u64::from(index == 0))
}

/// `-m^-1` modulo `2^64`, by Hensel doubling from the low limb.
///
/// An odd `m` is its own inverse modulo eight, so the iteration starts
/// three bits correct and doubles that precision each time: three, six,
/// twelve, twenty-four, forty-eight, ninety-six. Five steps therefore
/// pass sixty-four, and the negation at the end is the sign the reduction
/// step wants.
fn n0inv_of(low: u64) -> u64 {
    let mut inverse = low;
    for _ in 0..5 {
        let product = low.wrapping_mul(inverse);
        inverse = inverse.wrapping_mul(2u64.wrapping_sub(product));
    }
    inverse.wrapping_neg()
}

/// `2^(128*used)` modulo the modulus, by that many modular doublings.
///
/// At four thousand and ninety-six bits this is eight thousand one
/// hundred and ninety-two doublings of sixty-four limbs, paid once per
/// key.
fn r2_of(limbs: &[u64; MAX_LIMBS], used: usize) -> [u64; MAX_LIMBS] {
    let modulus = prefix(limbs, used);
    let mut value = one();
    let window = prefix_mut(&mut value, used);
    if !is_less(window, modulus) {
        let _ = subtract(window, modulus);
    }
    for _ in 0..used.saturating_mul(128) {
        let carry = shift_left_one(window);
        if carry != 0 || !is_less(window, modulus) {
            let _ = subtract(window, modulus);
        }
    }
    value
}

/// The limbs of a big-endian encoding, and how many of them the encoding
/// occupies.
fn read_be(bytes: &[u8], limbs: &mut [u64; MAX_LIMBS]) -> Result<usize, BignumError> {
    let used = bytes.len().div_ceil(8);
    if used > MAX_LIMBS {
        return Err(BignumError::TooWide);
    }
    let mut source = bytes.iter().rev().copied();
    for slot in limbs.iter_mut() {
        let mut limb = 0u64;
        for (index, byte) in (0u32..8).zip(source.by_ref()) {
            limb |= u64::from(byte).wrapping_shl(index.wrapping_mul(8));
        }
        *slot = limb;
    }
    Ok(used)
}

/// The big-endian encoding of the limbs, right-aligned in `out` and
/// padded with leading zeros.
fn write_be(limbs: &[u64], out: &mut [u8]) -> Result<(), BignumError> {
    out.fill(0);
    let mut source = limbs.iter().copied().flat_map(u64::to_le_bytes);
    for (slot, byte) in out.iter_mut().rev().zip(source.by_ref()) {
        *slot = byte;
    }
    if source.any(|byte| byte != 0) {
        return Err(BignumError::OutputTooShort);
    }
    Ok(())
}

/// How many bits the big-endian encoding actually carries.
fn bit_length(bytes: &[u8]) -> usize {
    let mut bits = 0usize;
    for byte in bytes {
        let significant = 8usize.saturating_sub(usize::try_from(byte.leading_zeros()).unwrap_or(8));
        bits = if bits == 0 {
            significant
        } else {
            bits.saturating_add(8)
        };
    }
    bits
}

/// Bit `shift` of byte `index` of a big-endian encoding, as a choice
/// rather than as a branch.
///
/// Both coordinates come from the loop counters of the ladder and are
/// public; the byte they select is not. So every byte is read and the
/// wanted one is kept by a mask, and the running time and the memory
/// accesses are the same whatever the bytes are.
fn bit_of(bytes: &[u8], index: usize, shift: u32) -> Choice {
    let mut value = 0u8;
    for (position, byte) in bytes.iter().copied().enumerate() {
        value |= Choice::from(position == index).mask_u8() & byte;
    }
    Choice::from_lsb(value.wrapping_shr(shift))
}

/// Bit `position` of a big-endian encoding, counted from the least
/// significant.
fn bit_at(bytes: &[u8], position: usize) -> bool {
    let index = position.wrapping_div(8);
    let shift = u32::try_from(position.wrapping_rem(8)).unwrap_or(0);
    let byte = bytes.iter().rev().nth(index).copied().unwrap_or(0);
    byte.wrapping_shr(shift) & 1 == 1
}
