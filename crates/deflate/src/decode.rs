// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A DEFLATE stream in, the bytes it stands for out.
//!
//! The decoder reads all three kinds of block RFC 1951 has, including the
//! kind this crate never writes — one that carries its own code table.
//! That is deliberate: a decoder that only read what this crate writes
//! could not be held against anybody else's compressor, and a stream from
//! a real one is the only proof that the codes here are the codes the
//! format means.
//!
//! What is decoded goes into a buffer the caller owns, and a back
//! reference is copied out of what has already been written there — which
//! is why a run may reach into itself: three bytes copied from one byte
//! back are that byte three times, and the format says so.

use crate::Error;
use crate::bits::Reader;
use crate::huffman::Tree;
use crate::tables::{
    DISTANCES, END_OF_BLOCK, FIRST_LENGTH, LITERALS, distance_of, fixed_distance_lengths,
    fixed_literal_lengths, length_of,
};

/// The order the code lengths of the code length alphabet are written in,
/// from RFC 1951, section 3.2.7.
const ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

/// How many symbols the code length alphabet has.
const CODE_LENGTHS: usize = 19;

/// Reads a stream into `out`, and says how many bytes it stands for.
pub(crate) fn inflate(input: &[u8], out: &mut [u8]) -> Result<usize, Error> {
    let mut reader = Reader::new(input);
    let mut written = 0usize;
    loop {
        let last = reader.bit()?;
        let kind = reader.bits(2)?;
        match kind {
            0 => written = stored(&mut reader, out, written)?,
            1 => {
                let literals = Tree::<LITERALS>::new(&fixed_literal_lengths());
                let distances = Tree::<DISTANCES>::new(&fixed_distance_lengths());
                written = block(&mut reader, out, written, &literals, &distances)?;
            }
            2 => {
                let (literals, distances) = tables(&mut reader)?;
                written = block(&mut reader, out, written, &literals, &distances)?;
            }
            _ => return Err(Error::Input),
        }
        if last == 1 {
            return Ok(written);
        }
    }
}

/// A block that carries its bytes unchanged.
fn stored(reader: &mut Reader<'_>, out: &mut [u8], written: usize) -> Result<usize, Error> {
    reader.align();
    let length = u16::from_le_bytes(reader.array::<2>()?);
    let complement = u16::from_le_bytes(reader.array::<2>()?);
    if length != !complement {
        return Err(Error::Input);
    }
    let bytes = reader.bytes(usize::from(length))?;
    let end = written.saturating_add(bytes.len());
    let room = out.get_mut(written..end).ok_or(Error::Output)?;
    room.copy_from_slice(bytes);
    Ok(end)
}

/// A block of coded symbols, whichever code it is coded with.
fn block(
    reader: &mut Reader<'_>,
    out: &mut [u8],
    start: usize,
    literals: &Tree<LITERALS>,
    distances: &Tree<DISTANCES>,
) -> Result<usize, Error> {
    let mut written = start;
    loop {
        let symbol = literals.decode(reader)?;
        if symbol == END_OF_BLOCK {
            return Ok(written);
        }
        if symbol < END_OF_BLOCK {
            let byte = u8::try_from(symbol).map_err(|_| Error::Input)?;
            let slot = out.get_mut(written).ok_or(Error::Output)?;
            *slot = byte;
            written = written.saturating_add(1);
            continue;
        }
        if symbol < FIRST_LENGTH {
            return Err(Error::Input);
        }
        let (extra, base) = length_of(symbol).ok_or(Error::Input)?;
        let length = usize::from(base)
            .saturating_add(usize::try_from(reader.bits(u32::from(extra))?).unwrap_or(0));
        let code = distances.decode(reader)?;
        let (extra, base) = distance_of(code).ok_or(Error::Input)?;
        let distance = usize::from(base)
            .saturating_add(usize::try_from(reader.bits(u32::from(extra))?).unwrap_or(0));
        let mut from = written.checked_sub(distance).ok_or(Error::Input)?;
        for _ in 0..length {
            let byte = out.get(from).copied().ok_or(Error::Input)?;
            let slot = out.get_mut(written).ok_or(Error::Output)?;
            *slot = byte;
            from = from.saturating_add(1);
            written = written.saturating_add(1);
        }
    }
}

/// The two code tables a dynamic block carries, read from the stream.
fn tables(reader: &mut Reader<'_>) -> Result<(Tree<LITERALS>, Tree<DISTANCES>), Error> {
    let literal_count = usize::try_from(reader.bits(5)?)
        .unwrap_or(0)
        .saturating_add(257);
    let distance_count = usize::try_from(reader.bits(5)?)
        .unwrap_or(0)
        .saturating_add(1);
    let code_count = usize::try_from(reader.bits(4)?)
        .unwrap_or(0)
        .saturating_add(4);
    // The three counts cannot be out of range: five bits reach 31 and
    // 287 literals, five reach 32 distances, four reach 19 code lengths,
    // and those are the sizes of the alphabets themselves.
    let mut code_lengths = [0u8; CODE_LENGTHS];
    for at in ORDER.into_iter().take(code_count) {
        let length = u8::try_from(reader.bits(3)?).unwrap_or(0);
        let slot = code_lengths.get_mut(at).ok_or(Error::Input)?;
        *slot = length;
    }
    let lengths_tree = Tree::<CODE_LENGTHS>::new(&code_lengths);
    // The lengths of both alphabets are one run of numbers, and a repeat
    // may reach from the end of the first into the start of the second.
    let total = literal_count.saturating_add(distance_count);
    let mut lengths = [0u8; LITERALS + DISTANCES];
    let mut at = 0usize;
    while at < total {
        let symbol = lengths_tree.decode(reader)?;
        let (value, times) = match symbol {
            0..=15 => (u8::try_from(symbol).map_err(|_| Error::Input)?, 1usize),
            16 => {
                let previous = at
                    .checked_sub(1)
                    .and_then(|before| lengths.get(before).copied())
                    .ok_or(Error::Input)?;
                (previous, repeat(reader, 2, 3)?)
            }
            17 => (0, repeat(reader, 3, 3)?),
            18 => (0, repeat(reader, 7, 11)?),
            _ => return Err(Error::Input),
        };
        for _ in 0..times {
            let slot = lengths.get_mut(at).ok_or(Error::Input)?;
            *slot = value;
            at = at.saturating_add(1);
        }
    }
    if at != total {
        return Err(Error::Input);
    }
    let literal_lengths = lengths.get(..literal_count).ok_or(Error::Input)?;
    let distance_lengths = lengths.get(literal_count..total).ok_or(Error::Input)?;
    Ok((
        Tree::<LITERALS>::new(literal_lengths),
        Tree::<DISTANCES>::new(distance_lengths),
    ))
}

/// How many times a repeat code repeats: what it says, plus its base.
fn repeat(reader: &mut Reader<'_>, bits: u32, base: usize) -> Result<usize, Error> {
    let extra = usize::try_from(reader.bits(bits)?).unwrap_or(0);
    Ok(base.saturating_add(extra))
}
