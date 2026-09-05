// SPDX-License-Identifier: AGPL-3.0-only AND Apache-2.0 WITH LLVM-exception
// Copyright (C) 2026 Manuel Baesler and contributors
// Copyright (C) the LLVM Project contributors, under Apache-2.0 WITH LLVM-exception
// Ported from LLVM's libFuzzer; see NOTICE at the root of this repository.

//! The words a mutator pastes into an input, and where they come from.
//!
//! The table of recent compares is libFuzzer's idea, ported here: every
//! comparison the target makes leaves its two sides in a small round-robin
//! table, and one of the mutators draws from that table. A reader that
//! refuses every tag but `0x30` hands the fuzzer `0x30` the first time it
//! looks at one, which random mutation alone would take a very long time
//! to find.
//!
//! Invariants: a word is at most [`MAX_WORD`] bytes, so that a table entry
//! is a value and never an allocation; an index into a table is taken
//! modulo its size, so that no caller has to know how large it is.

use crate::rng::Rng;

/// The longest word a dictionary entry holds.
pub const MAX_WORD: usize = 64;

/// The number of comparisons one table remembers.
pub const TABLE_SIZE: usize = 32;

/// The most positions of an existing value that a dictionary entry is
/// drawn from. A value that occurs more often than this is common enough
/// that the first few places are as good as any.
const MAX_POSITIONS: usize = 8;

/// A short run of bytes: what a mutator inserts or writes over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Word {
    /// The bytes, of which the first `len` count.
    bytes: [u8; MAX_WORD],
    /// How many bytes the word holds.
    len: usize,
}

impl Word {
    /// The word of `bytes`, truncated to [`MAX_WORD`].
    #[must_use]
    pub fn new(bytes: &[u8]) -> Self {
        let len = bytes.len().min(MAX_WORD);
        let mut word = Self {
            bytes: [0; MAX_WORD],
            len,
        };
        if let (Some(into), Some(from)) = (word.bytes.get_mut(..len), bytes.get(..len)) {
            into.copy_from_slice(from);
        }
        word
    }

    /// The bytes of the word.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        self.bytes.get(..self.len).unwrap_or(&[])
    }

    /// How many bytes the word holds.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether the word holds no bytes.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// A word together with the place in the input it was seen at, if it was
/// seen at one. The place is a hint: a mutator may write the word there or
/// anywhere else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The bytes to paste.
    pub word: Word,
    /// Where the value this word replaces was found.
    pub position: Option<usize>,
}

/// One comparison the target made.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Compare {
    /// The left side.
    pub left: u64,
    /// The right side.
    pub right: u64,
}

/// The last [`TABLE_SIZE`] comparisons of one operand width.
///
/// The slot a comparison lands in is chosen by the two sides differing,
/// not by a running counter: comparisons that keep failing the same way
/// keep one slot between them instead of flushing the table.
#[derive(Clone, Debug)]
pub struct Compares {
    /// The remembered comparisons.
    entries: [Compare; TABLE_SIZE],
    /// How many bytes of each side count, 4 or 8.
    width: usize,
}

impl Compares {
    /// An empty table for comparisons of `width` bytes.
    #[must_use]
    pub const fn new(width: usize) -> Self {
        Self {
            entries: [Compare { left: 0, right: 0 }; TABLE_SIZE],
            width,
        }
    }

    /// How many bytes of each side of this table's comparisons count.
    #[must_use]
    pub const fn width(&self) -> usize {
        self.width
    }

    /// Remembers a comparison.
    pub fn insert(&mut self, left: u64, right: u64) {
        let slot = usize::try_from(left ^ right).unwrap_or(0) % TABLE_SIZE;
        if let Some(entry) = self.entries.get_mut(slot) {
            *entry = Compare { left, right };
        }
    }

    /// The comparison in slot `index`, taken modulo the table size.
    #[must_use]
    pub fn get(&self, index: usize) -> Compare {
        self.entries
            .get(index % TABLE_SIZE)
            .copied()
            .unwrap_or_default()
    }
}

/// The low `width` bytes of `value` with their order reversed.
fn swap_width(value: u64, width: usize) -> u64 {
    let unused_bits = u32::try_from(MAX_WIDTH.saturating_sub(width).saturating_mul(8)).unwrap_or(0);
    value.swap_bytes().wrapping_shr(unused_bits)
}

/// The widest comparison a table holds.
const MAX_WIDTH: usize = 8;

/// The low `width` bytes of `value`, least significant first.
fn to_bytes(value: u64, width: usize) -> Word {
    let bytes = value.to_le_bytes();
    Word::new(bytes.get(..width.min(MAX_WIDTH)).unwrap_or(&[]))
}

/// The places in `haystack` where `needle` occurs, at most
/// [`MAX_POSITIONS`] of them.
fn positions_of(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return Vec::new();
    }
    haystack
        .windows(needle.len())
        .enumerate()
        .filter(|(_, window)| *window == needle)
        .map(|(at, _)| at)
        .take(MAX_POSITIONS)
        .collect()
}

/// A dictionary entry made from one remembered comparison.
///
/// Either side may be the one the target wanted and either may be the one
/// the input carries, so both readings are tried, in an order the draw
/// decides. The word is the wanted side moved by one in either direction
/// as often as not, because a length that is compared with `<` is passed
/// by the value next to it and not by the value itself. An entry whose
/// existing side is nowhere in `data` carries no position, and a mutator
/// then writes it wherever it likes.
pub fn entry_from_compare(rng: &mut Rng, compare: Compare, width: usize, data: &[u8]) -> Entry {
    let left = if rng.coin() {
        swap_width(compare.left, width)
    } else {
        compare.left
    };
    let right = if rng.coin() {
        swap_width(compare.right, width)
    } else {
        compare.right
    };
    let wanted = [
        left.wrapping_add(nudge(rng)),
        right.wrapping_add(nudge(rng)),
    ];
    let existing = [right, left];
    let first = usize::from(rng.coin());
    let mut word = Word::new(&[]);
    for step in 0..2 {
        let which = (first ^ step) & 1;
        let (Some(wanted), Some(existing)) = (wanted.get(which), existing.get(which)) else {
            continue;
        };
        word = to_bytes(*wanted, width);
        let places = positions_of(data, to_bytes(*existing, width).as_slice());
        if let Some(position) = places.get(rng.below(places.len())).copied() {
            return Entry {
                word,
                position: Some(position),
            };
        }
    }
    Entry {
        word,
        position: None,
    }
}

/// Minus one, nothing, or plus one.
fn nudge(rng: &mut Rng) -> u64 {
    match rng.below(3) {
        0 => u64::MAX,
        1 => 0,
        _ => 1,
    }
}
