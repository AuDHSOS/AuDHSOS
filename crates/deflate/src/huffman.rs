// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Code lengths in, codes out — and the other way round.
//!
//! RFC 1951, section 3.2.2, is what makes this short: a set of code
//! lengths stands for exactly one code, so neither side of a stream has to
//! carry the codes themselves. Shorter codes come before longer ones, and
//! codes of one length are given out in the order of the symbols they
//! belong to. Building a code is therefore counting how many symbols have
//! each length, giving each length its first code, and walking the symbols
//! once; reading one is the same walk from the other end.

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
    /// The one code a set of lengths stands for.
    pub(crate) fn new(lengths: &[u8; N]) -> Self {
        let counts = counts_of(lengths);
        // The first code of each length: twice the first code of the
        // length before it, plus however many codes that length used.
        let mut first = [0u16; MAX_BITS.saturating_add(1)];
        let mut code = 0u16;
        let mut before = 0u16;
        for (slot, count) in first.iter_mut().zip(counts) {
            code = code.saturating_add(before).saturating_mul(2);
            *slot = code;
            before = count;
        }
        let mut codes = Self {
            code: [0u16; N],
            length: *lengths,
        };
        // A symbol's code is the first code of its length plus however
        // many symbols of that length come before it.
        for (symbol, target) in codes.code.iter_mut().enumerate() {
            let length = lengths.get(symbol).copied().unwrap_or(0);
            if length == 0 {
                continue;
            }
            let rank = lengths
                .iter()
                .take(symbol)
                .filter(|other| **other == length)
                .count();
            *target = first
                .get(usize::from(length))
                .copied()
                .unwrap_or(0)
                .saturating_add(u16::try_from(rank).unwrap_or(0));
        }
        codes
    }
}

/// How many codes there are of each length. A length the format does not
/// have — nothing in a stream can say more than fifteen — counts for
/// none, and the symbol carrying it falls out of the code.
fn counts_of(lengths: &[u8]) -> [u16; MAX_BITS.saturating_add(1)] {
    core::array::from_fn(|bits| {
        let found = lengths
            .iter()
            .filter(|length| usize::from(**length) == bits && bits > 0)
            .count();
        u16::try_from(found).unwrap_or(u16::MAX)
    })
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
    /// The tree a set of lengths stands for.
    pub(crate) fn new(lengths: &[u8]) -> Self {
        let counts = counts_of(lengths);
        // The symbols, shortest code first and by symbol inside a length.
        // A tree with less room than the lengths ask for keeps what fits:
        // the caller gave the size, and a stream that wants more of them
        // is one that will not decode anyway.
        let mut symbols = [0u16; N];
        let mut places = symbols.iter_mut();
        for bits in 1..=MAX_BITS {
            let of_this_length = lengths
                .iter()
                .enumerate()
                .filter(|(_, length)| usize::from(**length) == bits);
            for (symbol, _) in of_this_length {
                let Some(slot) = places.next() else {
                    break;
                };
                *slot = u16::try_from(symbol).unwrap_or(0);
            }
        }
        Self { counts, symbols }
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
