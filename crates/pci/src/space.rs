// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The one seam of this crate: reading and writing a word of a function's
//! configuration space.
//!
//! Invariant: every function of this crate reaches the bytes only through
//! this trait, so nothing here holds a mapping, an address space, or a
//! pointer. The adapter that holds the mapping answers `None` for an
//! address or an offset it cannot reach, and a `None` becomes
//! [`PciError::Unreadable`] rather than a zero that reads like hardware.

use crate::address::Address;
use crate::error::PciError;

/// The configuration space of every function the caller can reach.
pub trait ConfigSpace {
    /// The word at `offset` of `address`, or `None` when the space does not
    /// reach it.
    fn read_u32(&self, address: Address, offset: u16) -> Option<u32>;

    /// Writes `value` at `offset` of `address`. A write the space does not
    /// reach is dropped: a configuration register that cannot be written is
    /// one the caller may not depend on having written.
    fn write_u32(&mut self, address: Address, offset: u16, value: u32);
}

/// The word at `offset`, as an error rather than a `None`.
///
/// # Errors
///
/// [`PciError::Unreadable`] when the space does not reach the offset.
pub fn read_word(space: &impl ConfigSpace, address: Address, offset: u16) -> Result<u32, PciError> {
    space
        .read_u32(address, offset)
        .ok_or(PciError::Unreadable(offset))
}

/// The lower half of the word at `offset`.
///
/// # Errors
///
/// The errors of [`read_word`].
pub fn read_u16(space: &impl ConfigSpace, address: Address, offset: u16) -> Result<u16, PciError> {
    let word = read_word(space, address, word_of(offset))?;
    Ok(u16::try_from(half(word, offset)).unwrap_or(0))
}

/// The byte at `offset`.
///
/// # Errors
///
/// The errors of [`read_word`].
pub fn read_u8(space: &impl ConfigSpace, address: Address, offset: u16) -> Result<u8, PciError> {
    let word = read_word(space, address, word_of(offset))?;
    Ok(u8::try_from(byte(word, offset)).unwrap_or(0))
}

/// Writes the lower or upper half of the word `offset` lies in, leaving
/// the other half as it was. A register of two bytes is written this way
/// because the mechanism carries whole words.
///
/// # Errors
///
/// The errors of [`read_word`].
pub fn write_u16(
    space: &mut impl ConfigSpace,
    address: Address,
    offset: u16,
    value: u16,
) -> Result<(), PciError> {
    let at = word_of(offset);
    let word = read_word(space, address, at)?;
    let shift = shift_of(offset);
    let mask = u32::from(u16::MAX).wrapping_shl(shift);
    let kept = word & !mask;
    space.write_u32(
        address,
        at,
        kept | (u32::from(value).wrapping_shl(shift) & mask),
    );
    Ok(())
}

/// The offset of the word that holds `offset`.
const fn word_of(offset: u16) -> u16 {
    offset & !0b11
}

/// How far into its word `offset` lies, in bits.
fn shift_of(offset: u16) -> u32 {
    u32::from(offset & 0b11).wrapping_mul(8)
}

/// The two bytes of `word` that start at `offset`.
fn half(word: u32, offset: u16) -> u32 {
    word.wrapping_shr(shift_of(offset)) & 0xFFFF
}

/// The byte of `word` at `offset`.
fn byte(word: u32, offset: u16) -> u32 {
    word.wrapping_shr(shift_of(offset)) & 0xFF
}
