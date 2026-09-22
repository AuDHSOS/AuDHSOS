// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Code lengths in, codes out — and the other way round.
//!
//! RFC 1951, section 3.2.2, is what makes this short: a set of code
//! lengths stands for exactly one code, so neither side of a stream has to
//! carry the codes themselves. Shorter codes come before longer ones, and
//! codes of one length are given out in the order of the symbols they
//! belong to. Building a code counts the symbols of each length, gives
//! each length its first code, and assigns the codes in one pass over the
//! symbols; reading one uses the same counts.

use crate::Error;

/// The longest code the format allows.
pub(crate) const MAX_BITS: usize = 15;

/// A code, for writing: what each symbol is written as and how long that
/// is. A symbol whose length is zero is not in the code at all.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Codes<const N: usize> {
    /// The code of every symbol, most significant bit first.
    pub(crate) code: [u16; N],
    /// The length of every code.
    pub(crate) length: [u8; N],
}

impl<const N: usize> Codes<N> {
    /// The one code a set of lengths stands for, in O(N): RFC 1951,
    /// section 3.2.2, steps 1 to 3.
    pub(crate) fn new(lengths: &[u8; N]) -> Self {
        let counts = counts_of(lengths);
        // The next code of each length, starting at the first:
        // `(first[b - 1] + count[b - 1]) * 2`, RFC 1951, section 3.2.2, step 2.
        let mut next = [0u16; MAX_BITS.saturating_add(1)];
        let mut code = 0u16;
        let mut before = 0u16;
        for (slot, count) in next.iter_mut().zip(counts) {
            code = code.saturating_add(before).saturating_mul(2);
            *slot = code;
            before = count;
        }
        let mut codes = Self {
            code: [0u16; N],
            length: *lengths,
        };
        for (target, length) in codes.code.iter_mut().zip(lengths) {
            if *length == 0 {
                continue;
            }
            if let Some(slot) = next.get_mut(usize::from(*length)) {
                *target = *slot;
                *slot = slot.saturating_add(1);
            }
        }
        codes
    }
}

/// How many codes there are of each length, in O(N). A length the format
/// does not have — a stream cannot say more than fifteen — counts for
/// none, and the symbol carrying it is not in the code.
fn counts_of(lengths: &[u8]) -> [u16; MAX_BITS.saturating_add(1)] {
    let mut counts = [0u16; MAX_BITS.saturating_add(1)];
    for length in lengths {
        if *length == 0 {
            continue;
        }
        if let Some(count) = counts.get_mut(usize::from(*length)) {
            *count = count.saturating_add(1);
        }
    }
    counts
}

/// How many codes of the longest length the lengths leave unused: zero
/// for a complete code, above zero for an incomplete one, and `None` for
/// one that asks for more codes than there are.
fn unused(counts: &[u16; MAX_BITS.saturating_add(1)]) -> Option<i32> {
    let mut left = 1i32;
    for count in counts.iter().skip(1) {
        left = left.checked_mul(2)?.checked_sub(i32::from(*count))?;
        if left < 0 {
            return None;
        }
    }
    Some(left)
}

/// A code, for reading: how many symbols have each length, and the
/// symbols themselves in the order the lengths put them.
#[derive(Clone, Debug)]
pub(crate) struct Tree<const N: usize> {
    /// How many codes there are of each length.
    counts: [u16; MAX_BITS.saturating_add(1)],
    /// The symbols, shortest code first and by symbol within a length.
    symbols: [u16; N],
}

impl<const N: usize> Tree<N> {
    /// The tree a set of lengths stands for, in O(N).
    ///
    /// # Errors
    ///
    /// [`Error::Input`] for lengths that over-subscribe the code or leave
    /// it incomplete, as RFC 1951, section 3.2.2, defines a code only for
    /// a complete set. Lengths that are all zero are a code of no
    /// symbols, and reading from it fails.
    pub(crate) fn new(lengths: &[u8]) -> Result<Self, Error> {
        Self::build(lengths, false)
    }

    /// The distance tree a set of lengths stands for, which may also be
    /// one code of length one, as RFC 1951, section 3.2.7, permits.
    ///
    /// # Errors
    ///
    /// [`Error::Input`] as for [`Tree::new`].
    pub(crate) fn distances(lengths: &[u8]) -> Result<Self, Error> {
        Self::build(lengths, true)
    }

    /// The tree, with `lone` allowing one code of length one.
    fn build(lengths: &[u8], lone: bool) -> Result<Self, Error> {
        let counts = counts_of(lengths);
        let left = unused(&counts).ok_or(Error::Input)?;
        let total = counts
            .iter()
            .fold(0u16, |sum, count| sum.saturating_add(*count));
        let lone_code = lone && total == 1 && counts.get(1) == Some(&1);
        if left > 0 && total > 0 && !lone_code {
            return Err(Error::Input);
        }
        // The index of the first symbol of each length: shortest code first,
        // by symbol within a length. Symbols past N are dropped.
        let mut offsets = [0usize; MAX_BITS.saturating_add(1)];
        let mut start = 0usize;
        for (slot, count) in offsets.iter_mut().zip(counts) {
            *slot = start;
            start = start.saturating_add(usize::from(count));
        }
        let mut symbols = [0u16; N];
        for (symbol, length) in lengths.iter().enumerate() {
            if *length == 0 {
                continue;
            }
            let Some(offset) = offsets.get_mut(usize::from(*length)) else {
                continue;
            };
            if let Some(slot) = symbols.get_mut(*offset) {
                *slot = u16::try_from(symbol).unwrap_or(0);
            }
            *offset = offset.saturating_add(1);
        }
        Ok(Self { counts, symbols })
    }

    /// Reads one symbol: a bit at a time, until the code is one of the
    /// codes of the length read so far.
    pub(crate) fn decode(&self, reader: &mut crate::bits::Reader<'_>) -> Result<u16, Error> {
        let mut code = 0i32;
        let mut first = 0i32;
        let mut index = 0i32;
        for bits in 1..=MAX_BITS {
            code |= i32::try_from(reader.bit()?).unwrap_or(0);
            let count = i32::from(self.counts.get(bits).copied().unwrap_or(0));
            if code.saturating_sub(count) < first {
                let at = index.saturating_add(code.saturating_sub(first));
                let at = usize::try_from(at).map_err(|_| Error::Input)?;
                return self.symbols.get(at).copied().ok_or(Error::Input);
            }
            index = index.saturating_add(count);
            first = first.saturating_add(count).saturating_mul(2);
            code = code.saturating_mul(2);
        }
        Err(Error::Input)
    }
}
