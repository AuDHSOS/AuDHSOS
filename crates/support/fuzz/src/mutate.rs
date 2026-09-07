// SPDX-License-Identifier: AGPL-3.0-only AND Apache-2.0 WITH LLVM-exception
// Copyright (C) 2026 Manuel Baesler and contributors
// Copyright (C) the LLVM Project contributors, under Apache-2.0 WITH LLVM-exception
// Ported from LLVM's libFuzzer; see NOTICE at the root of this repository.

//! How one input becomes the next.
//!
//! The set of mutations and the way one is chosen are libFuzzer's, ported.
//! Most of them are blind: erase a stretch, flip a bit, copy one part of
//! an input over another. Three are not, and they are the ones that get a
//! fuzzer past a parser's first gate: two paste a word from a dictionary,
//! and the third pastes a value the target was seen comparing against.
//!
//! Invariants: a mutation leaves the input no longer than the limit it was
//! given; a mutation that cannot apply to the input it was handed changes
//! nothing and says so, and the caller draws another; the words a round
//! pasted are remembered, so that a round which found something can put
//! them in the dictionary for good.

use crate::dictionary::{Compares, Entry, Word, entry_from_compare};
use crate::rng::Rng;
use crate::sancov::Trace;

/// How many times a draw is repeated when the mutation it drew declines.
const ATTEMPTS: usize = 10;

/// The most bytes `Mutation::InsertRepeatedBytes` inserts.
const MAX_REPEAT: usize = 128;

/// The fewest bytes `Mutation::InsertRepeatedBytes` inserts.
const MIN_REPEAT: usize = 3;

/// How many words the dictionary of paid-off entries holds before it
/// starts forgetting the oldest.
const MAX_PERSISTENT: usize = 1024;

/// One way of changing an input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mutation {
    /// Removes a stretch of bytes.
    EraseBytes,
    /// Inserts one byte.
    InsertByte,
    /// Inserts a run of one byte repeated.
    InsertRepeatedBytes,
    /// Overwrites one byte.
    ChangeByte,
    /// Flips one bit.
    ChangeBit,
    /// Shuffles a short stretch.
    ShuffleBytes,
    /// Reads a run of digits as a number and writes back another.
    ChangeAsciiInteger,
    /// Reads a fixed-width integer and writes back a near neighbour.
    ChangeBinaryInteger,
    /// Copies one part of the input over, or into, another.
    CopyPart,
    /// Takes bytes from a second input.
    CrossOver,
    /// Pastes a word the run was given on the command line.
    ManualDictionary,
    /// Pastes a word that once led somewhere new.
    PersistentDictionary,
    /// Pastes a value the target was seen comparing against.
    Compare,
}

/// Every mutation, in the order they are drawn from.
const ALL: [Mutation; 13] = [
    Mutation::EraseBytes,
    Mutation::InsertByte,
    Mutation::InsertRepeatedBytes,
    Mutation::ChangeByte,
    Mutation::ChangeBit,
    Mutation::ShuffleBytes,
    Mutation::ChangeAsciiInteger,
    Mutation::ChangeBinaryInteger,
    Mutation::CopyPart,
    Mutation::CrossOver,
    Mutation::ManualDictionary,
    Mutation::PersistentDictionary,
    Mutation::Compare,
];

impl Mutation {
    /// The name that appears in the line a run prints about an input.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::EraseBytes => "EraseBytes",
            Self::InsertByte => "InsertByte",
            Self::InsertRepeatedBytes => "InsertRepeatedBytes",
            Self::ChangeByte => "ChangeByte",
            Self::ChangeBit => "ChangeBit",
            Self::ShuffleBytes => "ShuffleBytes",
            Self::ChangeAsciiInteger => "ChangeASCIIInt",
            Self::ChangeBinaryInteger => "ChangeBinInt",
            Self::CopyPart => "CopyPart",
            Self::CrossOver => "CrossOver",
            Self::ManualDictionary => "ManualDict",
            Self::PersistentDictionary => "PersAutoDict",
            Self::Compare => "CMP",
        }
    }
}

/// The mutator of a run: its randomness, its dictionaries, and what the
/// round in progress has done so far.
#[derive(Clone, Debug)]
pub struct Mutator {
    /// Where every choice below comes from.
    pub rng: Rng,
    /// The words the run was given on the command line.
    manual: Vec<Word>,
    /// The words that once led somewhere new.
    persistent: Vec<Word>,
    /// The words this round pasted, which become persistent if it pays.
    pasted: Vec<Word>,
    /// The mutations this round applied, oldest first.
    sequence: Vec<Mutation>,
}

impl Mutator {
    /// A mutator with an empty dictionary, drawing from `seed`.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self {
            rng: Rng::new(seed),
            manual: Vec::new(),
            persistent: Vec::new(),
            pasted: Vec::new(),
            sequence: Vec::new(),
        }
    }

    /// Adds a word the run was given rather than found.
    pub fn add_word(&mut self, word: Word) {
        self.manual.push(word);
    }

    /// How many words the run was given.
    #[must_use]
    pub const fn words(&self) -> usize {
        self.manual.len()
    }

    /// Forgets what the round in progress did. The caller does this when
    /// it takes a fresh input out of the corpus.
    pub fn begin_round(&mut self) {
        self.sequence.clear();
        self.pasted.clear();
    }

    /// Keeps the words of this round for good, because it found something.
    pub fn reward(&mut self) {
        for word in self.pasted.drain(..) {
            if self.persistent.len() >= MAX_PERSISTENT {
                self.persistent.remove(0);
            }
            self.persistent.push(word);
        }
    }

    /// The mutations of the round in progress, oldest first.
    #[must_use]
    pub fn sequence(&self) -> &[Mutation] {
        &self.sequence
    }

    /// Changes `input`, leaving it no longer than `limit`.
    ///
    /// `cross` is another input of the corpus, which two of the mutations
    /// take bytes from, and `trace` is what the target's last run left
    /// behind. Answers whether anything changed: a caller that gets `false`
    /// has an input no mutation would touch, which happens for the empty
    /// input and a limit of zero.
    pub fn mutate(
        &mut self,
        input: &mut Vec<u8>,
        limit: usize,
        cross: Option<&[u8]>,
        trace: &Trace,
    ) -> bool {
        for _ in 0..ATTEMPTS {
            let choice = ALL
                .get(self.rng.below(ALL.len()))
                .copied()
                .unwrap_or(Mutation::ChangeByte);
            if self.apply(choice, input, limit, cross, trace) {
                input.truncate(limit);
                self.sequence.push(choice);
                return true;
            }
        }
        false
    }

    /// Applies one mutation, answering whether it did anything.
    fn apply(
        &mut self,
        choice: Mutation,
        input: &mut Vec<u8>,
        limit: usize,
        cross: Option<&[u8]>,
        trace: &Trace,
    ) -> bool {
        match choice {
            Mutation::EraseBytes => self.erase_bytes(input),
            Mutation::InsertByte => self.insert_byte(input, limit),
            Mutation::InsertRepeatedBytes => self.insert_repeated_bytes(input, limit),
            Mutation::ChangeByte => self.change_byte(input),
            Mutation::ChangeBit => self.change_bit(input),
            Mutation::ShuffleBytes => self.shuffle_bytes(input),
            Mutation::ChangeAsciiInteger => self.change_ascii_integer(input),
            Mutation::ChangeBinaryInteger => self.change_binary_integer(input),
            Mutation::CopyPart => self.copy_part(input, limit),
            Mutation::CrossOver => self.cross_over(input, limit, cross),
            Mutation::ManualDictionary => self.paste_from(Source::Manual, input, limit),
            Mutation::PersistentDictionary => self.paste_from(Source::Persistent, input, limit),
            Mutation::Compare => self.paste_compare(input, limit, trace),
        }
    }

    /// Removes a stretch of at most half the input.
    fn erase_bytes(&mut self, input: &mut Vec<u8>) -> bool {
        if input.len() <= 1 {
            return false;
        }
        let count = self.rng.below(input.len() / 2).saturating_add(1);
        let at = self
            .rng
            .below(input.len().saturating_sub(count).saturating_add(1));
        let end = at.saturating_add(count).min(input.len());
        input.drain(at..end);
        true
    }

    /// Inserts one byte anywhere.
    fn insert_byte(&mut self, input: &mut Vec<u8>, limit: usize) -> bool {
        if input.len() >= limit {
            return false;
        }
        let at = self.rng.below(input.len().saturating_add(1));
        let byte = self.rng.byte();
        input.insert(at, byte);
        true
    }

    /// Inserts a run of one byte repeated, because a parser that counts
    /// bytes needs more of them than a single insertion gives it.
    fn insert_repeated_bytes(&mut self, input: &mut Vec<u8>, limit: usize) -> bool {
        if input.len().saturating_add(MIN_REPEAT) >= limit {
            return false;
        }
        let room = limit.saturating_sub(input.len()).min(MAX_REPEAT);
        let count = self.rng.between(MIN_REPEAT, room);
        let at = self.rng.below(input.len().saturating_add(1));
        let byte = self.rng.byte();
        let tail: Vec<u8> = input.split_off(at);
        input.extend(core::iter::repeat_n(byte, count));
        input.extend_from_slice(&tail);
        true
    }

    /// Overwrites one byte.
    fn change_byte(&mut self, input: &mut [u8]) -> bool {
        let byte = self.rng.byte();
        let Some(slot) = self.rng.choose_mut(input) else {
            return false;
        };
        *slot = byte;
        true
    }

    /// Flips one bit.
    fn change_bit(&mut self, input: &mut [u8]) -> bool {
        let bit = u32::try_from(self.rng.below(8)).unwrap_or(0);
        let Some(slot) = self.rng.choose_mut(input) else {
            return false;
        };
        *slot ^= 1u8.wrapping_shl(bit);
        true
    }

    /// Shuffles a stretch of at most eight bytes.
    fn shuffle_bytes(&mut self, input: &mut [u8]) -> bool {
        let count = self.rng.below(input.len().min(8)).saturating_add(1);
        let Some(stretch) = self.rng.stretch_mut(input, count) else {
            return false;
        };
        self.rng.shuffle(stretch);
        true
    }

    /// Reads a run of decimal digits as a number and writes another back
    /// in its place, keeping the width.
    fn change_ascii_integer(&mut self, input: &mut [u8]) -> bool {
        let Some((start, end)) = self.find_digits(input) else {
            return false;
        };
        let mut value = 0u64;
        for byte in input.get(start..end).unwrap_or(&[]) {
            let digit = u64::from(byte.saturating_sub(b'0'));
            value = value.saturating_mul(10).saturating_add(digit);
        }
        value = match self.rng.below(5) {
            0 => value.saturating_add(1),
            1 => value.saturating_sub(1),
            2 => value / 2,
            3 => value.saturating_mul(2),
            _ => {
                let square = value.saturating_mul(value);
                u64::try_from(
                    self.rng
                        .below(usize::try_from(square).unwrap_or(usize::MAX)),
                )
                .unwrap_or(0)
            }
        };
        for slot in input
            .get_mut(start..end)
            .unwrap_or(&mut [])
            .iter_mut()
            .rev()
        {
            let digit = u8::try_from(value % 10).unwrap_or(0);
            *slot = b'0'.saturating_add(digit);
            value /= 10;
        }
        true
    }

    /// The first run of digits at or after a place drawn at random.
    fn find_digits(&mut self, input: &[u8]) -> Option<(usize, usize)> {
        if input.is_empty() {
            return None;
        }
        let from = self.rng.below(input.len());
        let start = input
            .iter()
            .enumerate()
            .skip(from)
            .find(|(_, byte)| byte.is_ascii_digit())
            .map(|(at, _)| at)?;
        let end = input
            .iter()
            .enumerate()
            .skip(start)
            .find(|(_, byte)| !byte.is_ascii_digit())
            .map_or(input.len(), |(at, _)| at);
        Some((start, end))
    }

    /// Reads a one-, two-, four- or eight-byte integer and writes back a
    /// near neighbour of it, or the length of the input, which is what a
    /// header field usually wants to hold.
    fn change_binary_integer(&mut self, input: &mut [u8]) -> bool {
        let width = 1usize.wrapping_shl(u32::try_from(self.rng.below(4)).unwrap_or(0));
        if input.len() < width {
            return false;
        }
        let at = self
            .rng
            .below(input.len().saturating_sub(width).saturating_add(1));
        let Some(field) = input.get(at..at.saturating_add(width)) else {
            return false;
        };
        let mask = mask_of(width);
        let mut value = read_le(field);
        if at < 64 && self.rng.below(4) == 0 {
            value = u64::try_from(input.len()).unwrap_or(u64::MAX) & mask;
            if self.rng.coin() {
                value = swap_width(value, width);
            }
        } else {
            let step = u64::try_from(self.rng.below(21))
                .unwrap_or(0)
                .wrapping_sub(10);
            value = if self.rng.coin() {
                swap_width(swap_width(value, width).wrapping_add(step) & mask, width)
            } else {
                value.wrapping_add(step) & mask
            };
            if step == 0 || self.rng.coin() {
                value = 0u64.wrapping_sub(value) & mask;
            }
        }
        write_le(
            input
                .get_mut(at..at.saturating_add(width))
                .unwrap_or(&mut []),
            value,
        );
        true
    }

    /// Copies one part of the input over another, or inserts it.
    fn copy_part(&mut self, input: &mut Vec<u8>, limit: usize) -> bool {
        if input.is_empty() {
            return false;
        }
        let source = input.clone();
        if input.len() >= limit || self.rng.coin() {
            self.copy_part_of(&source, input)
        } else {
            self.insert_part_of(&source, input, limit)
        }
    }

    /// Takes bytes from a second input of the corpus.
    fn cross_over(&mut self, input: &mut Vec<u8>, limit: usize, cross: Option<&[u8]>) -> bool {
        let Some(other) = cross else {
            return false;
        };
        if input.is_empty() || other.is_empty() {
            return false;
        }
        match self.rng.below(3) {
            0 => {
                let mixed = self.splice(input, other, limit);
                *input = mixed;
                true
            }
            1 => self.insert_part_of(other, input, limit) || self.copy_part_of(other, input),
            _ => self.copy_part_of(other, input),
        }
    }

    /// Copies a stretch of `from` over a stretch of `into`, leaving the
    /// length alone.
    fn copy_part_of(&mut self, from: &[u8], into: &mut [u8]) -> bool {
        if from.is_empty() || into.is_empty() {
            return false;
        }
        let to_start = self.rng.below(into.len());
        let room = into.len().saturating_sub(to_start);
        let count = self.rng.below(room).saturating_add(1).min(from.len());
        let from_start = self
            .rng
            .below(from.len().saturating_sub(count).saturating_add(1));
        let (Some(source), Some(target)) = (
            from.get(from_start..from_start.saturating_add(count)),
            into.get_mut(to_start..to_start.saturating_add(count)),
        ) else {
            return false;
        };
        target.copy_from_slice(source);
        true
    }

    /// Inserts a stretch of `from` into `into`, which grows by that much.
    fn insert_part_of(&mut self, from: &[u8], into: &mut Vec<u8>, limit: usize) -> bool {
        if from.is_empty() || into.len() >= limit {
            return false;
        }
        let room = limit.saturating_sub(into.len()).min(from.len());
        let count = self.rng.below(room).saturating_add(1);
        let Some(source) = self.rng.stretch(from, count) else {
            return false;
        };
        let at = self.rng.below(into.len().saturating_add(1));
        let tail: Vec<u8> = into.split_off(at);
        into.extend_from_slice(source);
        into.extend_from_slice(&tail);
        true
    }

    /// Alternating stretches of two inputs, which is what libFuzzer calls
    /// a crossover proper.
    fn splice(&mut self, first: &[u8], second: &[u8], limit: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(limit.min(first.len().saturating_add(second.len())));
        let (mut first_at, mut second_at) = (0usize, 0usize);
        let mut take_first = true;
        while out.len() < limit && (first_at < first.len() || second_at < second.len()) {
            let (source, at) = if take_first {
                (first, &mut first_at)
            } else {
                (second, &mut second_at)
            };
            let left = source.len().saturating_sub(*at);
            if left > 0 {
                let room = limit.saturating_sub(out.len());
                let count = self.rng.below(left).saturating_add(1).min(room);
                out.extend(source.iter().skip(*at).take(count));
                *at = at.saturating_add(count);
            }
            take_first = !take_first;
        }
        out
    }

    /// Pastes a word from one of the two dictionaries the run keeps.
    fn paste_from(&mut self, source: Source, input: &mut Vec<u8>, limit: usize) -> bool {
        let words = match source {
            Source::Manual => &self.manual,
            Source::Persistent => &self.persistent,
        };
        let Some(word) = self.rng.choose(words) else {
            return false;
        };
        self.paste(
            Entry {
                word,
                position: None,
            },
            input,
            limit,
        )
    }

    /// Pastes a value the target was seen comparing against.
    fn paste_compare(&mut self, input: &mut Vec<u8>, limit: usize, trace: &Trace) -> bool {
        let table: &Compares = if self.rng.coin() {
            &trace.compares8
        } else {
            &trace.compares4
        };
        let slot = usize::try_from(self.rng.next_u64()).unwrap_or(0);
        let compare = table.get(slot);
        if compare.left == 0 && compare.right == 0 {
            return false;
        }
        let mut width = table.width();
        if width == 4
            && compare.left.wrapping_shr(16) == 0
            && compare.right.wrapping_shr(16) == 0
            && self.rng.coin()
        {
            width = 2;
        }
        let entry = entry_from_compare(&mut self.rng, compare, width, input);
        self.paste(entry, input, limit)
    }

    /// Writes a dictionary entry into the input, either over the bytes
    /// that are there or between them.
    fn paste(&mut self, entry: Entry, input: &mut Vec<u8>, limit: usize) -> bool {
        if entry.word.is_empty() {
            return false;
        }
        let bytes = entry.word.as_slice();
        let hinted = entry
            .position
            .filter(|at| at.saturating_add(bytes.len()) < input.len() && self.rng.coin());
        let placed = if self.rng.coin() {
            if input.len().saturating_add(bytes.len()) > limit {
                return false;
            }
            let at = hinted.unwrap_or_else(|| self.rng.below(input.len().saturating_add(1)));
            let tail: Vec<u8> = input.split_off(at.min(input.len()));
            input.extend_from_slice(bytes);
            input.extend_from_slice(&tail);
            true
        } else {
            if bytes.len() > input.len() {
                return false;
            }
            let room = input.len().saturating_sub(bytes.len()).saturating_add(1);
            let at = hinted.unwrap_or_else(|| self.rng.below(room));
            match input.get_mut(at..at.saturating_add(bytes.len())) {
                Some(target) => {
                    target.copy_from_slice(bytes);
                    true
                }
                None => false,
            }
        };
        if placed {
            self.pasted.push(entry.word);
        }
        placed
    }
}

/// Which of the two dictionaries a paste draws from.
#[derive(Clone, Copy, Debug)]
enum Source {
    /// The words the run was given.
    Manual,
    /// The words that once led somewhere new.
    Persistent,
}

/// The mask of the low `width` bytes.
fn mask_of(width: usize) -> u64 {
    if width >= 8 {
        return u64::MAX;
    }
    let bits = u32::try_from(width.saturating_mul(8)).unwrap_or(0);
    1u64.wrapping_shl(bits).wrapping_sub(1)
}

/// The low `width` bytes of `value` with their order reversed.
fn swap_width(value: u64, width: usize) -> u64 {
    let unused = u32::try_from(8usize.saturating_sub(width).saturating_mul(8)).unwrap_or(0);
    value.swap_bytes().wrapping_shr(unused)
}

/// The bytes of `field`, least significant first, as a number.
fn read_le(field: &[u8]) -> u64 {
    let mut value = 0u64;
    for (step, byte) in field.iter().enumerate().take(8) {
        let shift = u32::try_from(step.saturating_mul(8)).unwrap_or(0);
        value |= u64::from(*byte).wrapping_shl(shift);
    }
    value
}

/// Writes `value` into `field`, least significant byte first.
fn write_le(field: &mut [u8], value: u64) {
    for (step, slot) in field.iter_mut().enumerate().take(8) {
        let shift = u32::try_from(step.saturating_mul(8)).unwrap_or(0);
        *slot = u8::try_from(value.wrapping_shr(shift) & 0xff).unwrap_or(0);
    }
}
