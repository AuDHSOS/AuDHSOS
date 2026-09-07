// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Bytes in, a DEFLATE stream out.
//!
//! One pass, and one decision per position: is there a run of at least
//! three bytes that has been seen before within the last thirty-two
//! kilobytes? If there is, the longest one found is written as a length
//! and a distance; if there is not, the byte is written as itself. That is
//! the whole of the compressor, and what makes it quick is the one table
//! it keeps: for every three bytes seen, where they were last seen, and
//! from there a chain back through every earlier place with the same three.
//!
//! The codes are the fixed ones of RFC 1951, section 3.2.6 — the ones the
//! format itself names, so that no table has to be written into the
//! stream. A block with its own code table would be smaller still; it
//! would also be a second algorithm, and this one is what makes the
//! difference between a file of six megabytes and one of two.

use crate::bits::Writer;
use crate::huffman::Codes;
use crate::tables::{
    DISTANCES, END_OF_BLOCK, LITERALS, MAX_MATCH, MIN_MATCH, WINDOW, distance_code,
    fixed_distance_lengths, fixed_literal_lengths, length_code,
};
use crate::{Error, Scratch};

/// How many places back along a chain are looked at before the search
/// gives up. A longer walk finds longer runs and costs time for them;
/// this is the number at which the two stop trading well.
const CHAIN: usize = 128;

/// How many bits of hash the table is indexed by.
pub(crate) const HASH_BITS: u32 = 15;

/// How many places the table has.
pub(crate) const HASH_SIZE: usize = 1 << HASH_BITS;

/// The most bytes a stored block may carry.
const STORED_BLOCK: usize = 65535;

/// Compresses `input` into `out`, falling back to blocks that carry it
/// unchanged when coding it would cost more than saying it plainly.
pub(crate) fn compress(
    input: &[u8],
    out: &mut [u8],
    scratch: &mut Scratch,
) -> Result<usize, Error> {
    let plain = stored_length(input.len());
    match deflate(input, out, scratch) {
        Ok(written) if written < plain => Ok(written),
        Ok(_) | Err(Error::Output) => store(input, out),
        Err(other) => Err(other),
    }
}

/// The hash of the three bytes at `at`, or nothing if there are not three.
pub(crate) fn hash_of(input: &[u8], at: usize) -> Option<usize> {
    let one = u32::from(input.get(at).copied()?);
    let two = u32::from(input.get(at.saturating_add(1)).copied()?);
    let three = u32::from(input.get(at.saturating_add(2)).copied()?);
    let mixed = (one << 10) ^ (two << 5) ^ three;
    usize::try_from(mixed)
        .ok()
        .map(|value| value.wrapping_rem(HASH_SIZE))
}

/// Writes `input` into `out` as one compressed block, and says how many
/// bytes it took.
fn deflate(input: &[u8], out: &mut [u8], scratch: &mut Scratch) -> Result<usize, Error> {
    let literals = Codes::<LITERALS>::new(&fixed_literal_lengths());
    let distances = Codes::<DISTANCES>::new(&fixed_distance_lengths());
    let mut writer = Writer::new(out);
    // One block, and it is the last: BFINAL, then BTYPE 01 for the fixed
    // codes, both written low bit first.
    writer.bits(1, 1)?;
    writer.bits(1, 2)?;
    scratch.clear();
    let mut at = 0usize;
    while at < input.len() {
        if let Some((length, distance)) = longest(input, at, scratch) {
            let (code, extra, value) = length_code(length).ok_or(Error::Input)?;
            symbol(&mut writer, &literals, code)?;
            writer.bits(u32::from(value), u32::from(extra))?;
            let (code, extra, value) = distance_code(distance).ok_or(Error::Input)?;
            symbol(&mut writer, &distances, code)?;
            writer.bits(u32::from(value), u32::from(extra))?;
            for step in 0..length {
                scratch.insert(input, at.saturating_add(step));
            }
            at = at.saturating_add(length);
        } else {
            let byte = input.get(at).copied().ok_or(Error::Input)?;
            symbol(&mut writer, &literals, u16::from(byte))?;
            scratch.insert(input, at);
            at = at.saturating_add(1);
        }
    }
    symbol(&mut writer, &literals, END_OF_BLOCK)?;
    writer.finish()
}

/// Writes `input` into `out` as blocks that carry it unchanged, and says
/// how many bytes it took. This is what a stream that will not compress
/// gets: five bytes for every sixty-four kilobytes and not one more.
pub(crate) fn store(input: &[u8], out: &mut [u8]) -> Result<usize, Error> {
    let mut writer = Writer::new(out);
    let mut at = 0usize;
    loop {
        let end = at.saturating_add(STORED_BLOCK).min(input.len());
        let piece = input.get(at..end).ok_or(Error::Input)?;
        let last = u32::from(end >= input.len());
        writer.bits(last, 1)?;
        writer.bits(0, 2)?;
        writer.align()?;
        let length = u16::try_from(piece.len()).map_err(|_| Error::Input)?;
        writer.bytes(&length.to_le_bytes())?;
        writer.bytes(&(!length).to_le_bytes())?;
        writer.bytes(piece)?;
        at = end;
        if last == 1 {
            break;
        }
    }
    writer.finish()
}

/// How many bytes `store` needs for an input of `length` bytes.
pub(crate) const fn stored_length(length: usize) -> usize {
    let blocks = match length.div_ceil(STORED_BLOCK) {
        0 => 1,
        found => found,
    };
    length.saturating_add(blocks.saturating_mul(5))
}

/// Writes one symbol of an alphabet.
fn symbol<const N: usize>(
    writer: &mut Writer<'_>,
    codes: &Codes<N>,
    symbol: u16,
) -> Result<(), Error> {
    let at = usize::from(symbol);
    let code = codes.code.get(at).copied().ok_or(Error::Input)?;
    let length = codes.length.get(at).copied().ok_or(Error::Input)?;
    writer.code(code, length)
}

/// The longest run at `at` that has been seen before, and how far back it
/// was seen. Nothing shorter than three bytes is worth a code.
fn longest(input: &[u8], at: usize, scratch: &Scratch) -> Option<(usize, usize)> {
    let limit = input.len().saturating_sub(at).min(MAX_MATCH);
    if limit < MIN_MATCH {
        return None;
    }
    let mut candidate = scratch.head(input, at)?;
    let mut best: Option<(usize, usize)> = None;
    for _ in 0..CHAIN {
        let distance = at.checked_sub(candidate)?;
        if distance == 0 || distance > WINDOW {
            break;
        }
        let length = run(input, candidate, at, limit);
        if length >= MIN_MATCH && best.is_none_or(|(found, _)| length > found) {
            best = Some((length, distance));
            if length >= limit {
                break;
            }
        }
        if let Some(earlier) = scratch.previous(candidate)
            && earlier < candidate
        {
            candidate = earlier;
        } else {
            break;
        }
    }
    best
}

/// How many bytes are the same at two places, up to `limit`.
fn run(input: &[u8], earlier: usize, later: usize, limit: usize) -> usize {
    let mut length = 0usize;
    while length < limit {
        let one = input.get(earlier.saturating_add(length));
        let two = input.get(later.saturating_add(length));
        match (one, two) {
            (Some(one), Some(two)) if one == two => length = length.saturating_add(1),
            _ => break,
        }
    }
    length
}
