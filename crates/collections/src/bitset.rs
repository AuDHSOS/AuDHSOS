// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A set of bits of fixed size.
//!
//! The parameter is the number of 64-bit words, not the number of bits,
//! because stable Rust cannot size an array from an expression over a
//! const parameter and `[u64; BITS.div_ceil(64)]` is such an expression.
//! [`BitSet::BITS`] says how many bits that is, and every operation is
//! checked against it, so a caller reads the size from the type rather
//! than computing it.

use crate::error::CollectionError;

/// Bits in one word.
const WORD: usize = 64;

/// A set of `WORDS * 64` bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BitSet<const WORDS: usize> {
    /// The words, least significant bit of the first word first.
    words: [u64; WORDS],
}

impl<const WORDS: usize> BitSet<WORDS> {
    /// How many bits the set holds.
    pub const BITS: usize = WORDS.wrapping_mul(WORD);

    /// A set with every bit clear.
    #[must_use]
    pub const fn new() -> BitSet<WORDS> {
        BitSet { words: [0; WORDS] }
    }

    /// A set with every bit set.
    #[must_use]
    pub const fn full() -> BitSet<WORDS> {
        BitSet {
            words: [u64::MAX; WORDS],
        }
    }

    /// How many bits the set holds.
    #[must_use]
    pub const fn bits(&self) -> usize {
        Self::BITS
    }

    /// Whether every bit is clear.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.words.iter().all(|word| *word == 0)
    }

    /// How many bits are set.
    #[must_use]
    pub fn count(&self) -> usize {
        self.words.iter().fold(0usize, |total, word| {
            total.wrapping_add(usize::try_from(word.count_ones()).unwrap_or(0))
        })
    }

    /// Whether the bit at `index` is set.
    ///
    /// # Errors
    ///
    /// [`CollectionError::Index`] when `index` is at or beyond
    /// [`BitSet::BITS`].
    pub fn test(&self, index: usize) -> Result<bool, CollectionError> {
        let (word, mask) = Self::place(index)?;
        let word = self.words.get(word).ok_or(CollectionError::Index(index))?;
        Ok(word & mask != 0)
    }

    /// Sets the bit at `index` and answers what it was.
    ///
    /// # Errors
    ///
    /// [`CollectionError::Index`] when `index` is at or beyond
    /// [`BitSet::BITS`].
    pub fn set(&mut self, index: usize) -> Result<bool, CollectionError> {
        let (word, mask) = Self::place(index)?;
        let word = self
            .words
            .get_mut(word)
            .ok_or(CollectionError::Index(index))?;
        let was = *word & mask != 0;
        *word |= mask;
        Ok(was)
    }

    /// Clears the bit at `index` and answers what it was.
    ///
    /// # Errors
    ///
    /// [`CollectionError::Index`] when `index` is at or beyond
    /// [`BitSet::BITS`].
    pub fn clear(&mut self, index: usize) -> Result<bool, CollectionError> {
        let (word, mask) = Self::place(index)?;
        let word = self
            .words
            .get_mut(word)
            .ok_or(CollectionError::Index(index))?;
        let was = *word & mask != 0;
        *word &= !mask;
        Ok(was)
    }

    /// Clears every bit.
    pub const fn clear_all(&mut self) {
        self.words = [0; WORDS];
    }

    /// The lowest index whose bit is set, or `None` when none is. This is
    /// the operation a scheduler runs on every switch, which is why it is
    /// a scan of words and not of bits.
    #[must_use]
    pub fn first_set(&self) -> Option<usize> {
        self.scan(|word| word)
    }

    /// The lowest index whose bit is clear, or `None` when none is.
    #[must_use]
    pub fn first_clear(&self) -> Option<usize> {
        self.scan(|word| !word)
    }

    /// The lowest index at which `wanted` of a word has a set bit.
    fn scan(&self, wanted: impl Fn(u64) -> u64) -> Option<usize> {
        for (index, word) in self.words.iter().enumerate() {
            let candidate = wanted(*word);
            if candidate == 0 {
                continue;
            }
            let bit = usize::try_from(candidate.trailing_zeros()).ok()?;
            return index.checked_mul(WORD)?.checked_add(bit);
        }
        None
    }

    /// The word and the mask of a bit, or an error when it is outside.
    fn place(index: usize) -> Result<(usize, u64), CollectionError> {
        if index >= Self::BITS {
            return Err(CollectionError::Index(index));
        }
        let word = index.wrapping_div(WORD);
        let bit = index.wrapping_rem(WORD);
        let mask = 1u64
            .checked_shl(u32::try_from(bit).map_err(|_| CollectionError::Index(index))?)
            .ok_or(CollectionError::Index(index))?;
        Ok((word, mask))
    }
}

impl<const WORDS: usize> Default for BitSet<WORDS> {
    fn default() -> BitSet<WORDS> {
        BitSet::new()
    }
}
