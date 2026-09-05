// SPDX-License-Identifier: AGPL-3.0-only AND Apache-2.0 WITH LLVM-exception
// Copyright (C) 2026 Manuel Baesler and contributors
// Copyright (C) the LLVM Project contributors, under Apache-2.0 WITH LLVM-exception
// Ported from LLVM's libFuzzer; see NOTICE at the root of this repository.

//! What a run of the target is worth: the features its counters stand for.
//!
//! A feature is one number. Two inputs that produce the same set of them
//! are, as far as the fuzzer can tell, the same input, and an input that
//! produces one nobody has produced before is worth keeping. The bucketing
//! is AFL's and reached this project by way of libFuzzer: an edge taken
//! twice differs from an edge taken once, an edge taken a hundred times
//! does not differ from one taken ninety.
//!
//! Invariant: a counter of zero has no feature. The caller skips zero
//! bytes, which is what makes collecting features cost about as much as
//! reading the counters once.

/// The number of buckets one counter is sorted into.
pub const BUCKETS: u32 = 8;

/// The bucket of a non-zero counter, in `0 ..8`.
///
/// ```text
/// counter: [1] [2] [3] [4-7] [8-15] [16-31] [32-127] [128+]
/// bucket:   0   1   2    3     4       5       6       7
/// ```
///
/// A counter of zero answers zero, the bucket of a single visit, because
/// the callers of this function never pass one.
#[must_use]
pub const fn bucket(counter: u8) -> u32 {
    match counter {
        0..=1 => 0,
        2 => 1,
        3 => 2,
        4..=7 => 3,
        8..=15 => 4,
        16..=31 => 5,
        32..=127 => 6,
        128..=255 => 7,
    }
}

/// The feature of the counter at `index`, given the count it holds.
///
/// The counters of a run form one numbering, so `index` is the position of
/// the counter in that numbering and not in its own region.
#[must_use]
pub fn feature_of(index: usize, counter: u8) -> u32 {
    let base = u32::try_from(index)
        .unwrap_or(u32::MAX)
        .saturating_mul(BUCKETS);
    base.saturating_add(bucket(counter))
}

/// The number of bits in the value map, a power of two so that the
/// remainder is a mask.
const VALUE_BITS: usize = 1 << 16;

/// The number of bits one word of the map holds.
const BITS_PER_WORD: usize = 64;

/// The number of words that many bits need.
const VALUE_WORDS: usize = VALUE_BITS / BITS_PER_WORD;

/// The bits a run has set from the comparisons it made.
///
/// This is libFuzzer's value profile: a comparison contributes not only
/// the branch it decided but how close its two sides were, so that an
/// input which gets a magic number half right is worth more than one that
/// gets it wrong entirely. It costs a clear and a scan of eight kilobytes
/// per run, which is why the engine only keeps it when it is asked to.
#[derive(Clone, Debug)]
pub struct ValueMap {
    /// The bits, `VALUE_BITS` of them.
    words: [u64; VALUE_WORDS],
}

impl Default for ValueMap {
    fn default() -> Self {
        Self::new()
    }
}

impl ValueMap {
    /// A map with nothing set.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            words: [0; VALUE_WORDS],
        }
    }

    /// Sets the bit that `value` hashes to and answers whether it was
    /// clear before.
    pub fn add(&mut self, value: u64) -> bool {
        let index = usize::try_from(value % VALUE_BITS_U64).unwrap_or(0);
        let (word_index, mask) = position(index);
        let Some(word) = self.words.get_mut(word_index) else {
            return false;
        };
        let was = *word;
        *word |= mask;
        was != *word
    }

    /// Clears every bit.
    pub fn clear(&mut self) {
        self.words.fill(0);
    }

    /// Calls `visit` with every bit that is set.
    pub fn for_each(&self, mut visit: impl FnMut(u32)) {
        for (word_index, word) in self.words.iter().enumerate() {
            let mut rest = *word;
            while rest != 0 {
                let bit = rest.trailing_zeros();
                let index = u32::try_from(word_index)
                    .unwrap_or(u32::MAX)
                    .saturating_mul(u64::BITS)
                    .saturating_add(bit);
                visit(index);
                rest &= rest.wrapping_sub(1);
            }
        }
    }
}

/// The number of bits in the value map, as the type the remainder needs.
const VALUE_BITS_U64: u64 = 1 << 16;

/// The word and the bit mask that bit `index` of a value map lives in.
fn position(index: usize) -> (usize, u64) {
    let word = index / BITS_PER_WORD;
    let bit = u32::try_from(index % BITS_PER_WORD).unwrap_or(0);
    (word, 1u64.wrapping_shl(bit))
}
