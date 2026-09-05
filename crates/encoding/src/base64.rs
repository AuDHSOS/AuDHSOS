// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Base64 of RFC 4648 in its standard alphabet, strict in both
//! directions.
//!
//! Strict is the whole point. A decoder that skips whitespace, tolerates a
//! missing pad, or ignores the unused bits of the last quantum accepts
//! several texts for one sequence of bytes, and a certificate that can be
//! written two ways is a certificate that can be compared wrongly. Every
//! one of those is an error here.

use crate::error::EncodingError;

/// The standard alphabet of RFC 4648, index to character.
const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// The pad character.
const PAD: u8 = b'=';

/// The number of characters a text of `bytes` bytes takes, or `None` when
/// that count does not fit a `usize`.
#[must_use]
pub const fn encoded_len(bytes: usize) -> Option<usize> {
    match bytes.checked_add(2) {
        Some(rounded) => match rounded.checked_div(3) {
            Some(quanta) => quanta.checked_mul(4),
            None => None,
        },
        None => None,
    }
}

/// The largest number of bytes a text of `characters` characters can
/// decode to, or `None` when the length is not a multiple of four.
#[must_use]
pub const fn decoded_len(characters: usize) -> Option<usize> {
    if characters.wrapping_rem(4) != 0 {
        return None;
    }
    match characters.checked_div(4) {
        Some(quanta) => quanta.checked_mul(3),
        None => None,
    }
}

/// Writes `input` as Base64 into `out` and answers how many characters it
/// wrote.
///
/// # Errors
///
/// [`EncodingError::BufferTooSmall`] when `out` is shorter than
/// [`encoded_len`] of the input, in which case nothing is written.
pub fn encode(input: &[u8], out: &mut [u8]) -> Result<usize, EncodingError> {
    let needed = encoded_len(input.len()).ok_or(EncodingError::BufferTooSmall)?;
    if out.len() < needed {
        return Err(EncodingError::BufferTooSmall);
    }
    let mut written = 0usize;
    let (quanta, remainder) = input.as_chunks::<3>();
    for [first, second, third] in quanta.iter().copied() {
        let quantum = u32::from(first)
            .wrapping_shl(16)
            .wrapping_add(u32::from(second).wrapping_shl(8))
            .wrapping_add(u32::from(third));
        written = put(out, written, sextet(quantum, 18))?;
        written = put(out, written, sextet(quantum, 12))?;
        written = put(out, written, sextet(quantum, 6))?;
        written = put(out, written, sextet(quantum, 0))?;
    }
    match *remainder {
        [] => {}
        [first] => {
            let quantum = u32::from(first).wrapping_shl(16);
            written = put(out, written, sextet(quantum, 18))?;
            written = put(out, written, sextet(quantum, 12))?;
            written = put(out, written, PAD)?;
            written = put(out, written, PAD)?;
        }
        [first, second] => {
            let quantum = u32::from(first)
                .wrapping_shl(16)
                .wrapping_add(u32::from(second).wrapping_shl(8));
            written = put(out, written, sextet(quantum, 18))?;
            written = put(out, written, sextet(quantum, 12))?;
            written = put(out, written, sextet(quantum, 6))?;
            written = put(out, written, PAD)?;
        }
        _ => return Err(EncodingError::BufferTooSmall),
    }
    Ok(written)
}

/// Reads `input` as Base64 into `out` and answers how many bytes it wrote.
///
/// # Errors
///
/// [`EncodingError::Length`] when the input is not a multiple of four,
/// [`EncodingError::Character`] for anything outside the alphabet,
/// whitespace included, [`EncodingError::Padding`] for a pad that is not
/// one or two characters at the end, [`EncodingError::TrailingBits`] when
/// the last quantum carries bits its length declares unused, and
/// [`EncodingError::BufferTooSmall`] when `out` is too short.
pub fn decode(input: &[u8], out: &mut [u8]) -> Result<usize, EncodingError> {
    if input.len().wrapping_rem(4) != 0 {
        return Err(EncodingError::Length(input.len()));
    }
    let pad = pad_length(input)?;
    let bytes = decoded_len(input.len())
        .and_then(|full| full.checked_sub(pad))
        .ok_or(EncodingError::BufferTooSmall)?;
    if out.len() < bytes {
        return Err(EncodingError::BufferTooSmall);
    }
    let mut written = 0usize;
    let (quanta, _) = input.as_chunks::<4>();
    for (index, [a, b, c, d]) in quanta.iter().copied().enumerate() {
        let offset = index.wrapping_mul(4);
        let quantum = u32::from(value(a, offset)?)
            .wrapping_shl(18)
            .wrapping_add(u32::from(value(b, offset.wrapping_add(1))?).wrapping_shl(12))
            .wrapping_add(u32::from(pad_or_value(c, offset.wrapping_add(2))?).wrapping_shl(6))
            .wrapping_add(u32::from(pad_or_value(d, offset.wrapping_add(3))?));
        let here = if offset.wrapping_add(4) == input.len() {
            3usize.wrapping_sub(pad)
        } else {
            3
        };
        if quantum & unused_mask(here) != 0 {
            return Err(EncodingError::TrailingBits);
        }
        for byte in 0..here {
            let shift = 16u32.wrapping_sub(u32::try_from(byte).unwrap_or(0).wrapping_mul(8));
            written = put(out, written, octet(quantum, shift))?;
        }
    }
    Ok(written)
}

/// The bits of a quantum that lie below the `count` bytes it keeps, and
/// that a canonical text therefore leaves zero.
const fn unused_mask(count: usize) -> u32 {
    match count {
        1 => 0x0000_FFFF,
        2 => 0x0000_00FF,
        _ => 0,
    }
}

/// The number of pad characters at the end, checked to stand nowhere else.
fn pad_length(input: &[u8]) -> Result<usize, EncodingError> {
    let mut pad = 0usize;
    for (index, byte) in input.iter().enumerate() {
        if *byte != PAD {
            continue;
        }
        let from_end = input.len().wrapping_sub(index);
        if from_end > 2 {
            return Err(EncodingError::Padding);
        }
        pad = pad.wrapping_add(1);
    }
    if pad > 0 && input.is_empty() {
        return Err(EncodingError::Padding);
    }
    if pad == 1 && input.last() != Some(&PAD) {
        return Err(EncodingError::Padding);
    }
    Ok(pad)
}

/// The value of one alphabet character.
const fn value(byte: u8, at: usize) -> Result<u8, EncodingError> {
    match byte {
        b'A'..=b'Z' => Ok(byte.wrapping_sub(b'A')),
        b'a'..=b'z' => Ok(byte.wrapping_sub(b'a').wrapping_add(26)),
        b'0'..=b'9' => Ok(byte.wrapping_sub(b'0').wrapping_add(52)),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(EncodingError::Character(at)),
    }
}

/// The value of one character, with the pad reading as zero. The caller
/// has already checked where a pad may stand.
const fn pad_or_value(byte: u8, at: usize) -> Result<u8, EncodingError> {
    if byte == PAD { Ok(0) } else { value(byte, at) }
}

/// The alphabet character of the six bits at `shift`.
fn sextet(quantum: u32, shift: u32) -> u8 {
    let index = quantum.wrapping_shr(shift) & 0x3F;
    let index = usize::try_from(index).unwrap_or(0);
    ALPHABET.get(index).copied().unwrap_or(b'A')
}

/// The byte of the eight bits at `shift`.
fn octet(quantum: u32, shift: u32) -> u8 {
    u8::try_from(quantum.wrapping_shr(shift) & 0xFF).unwrap_or(0)
}

/// Writes one byte at `at` and answers the next position.
fn put(out: &mut [u8], at: usize, byte: u8) -> Result<usize, EncodingError> {
    let slot = out.get_mut(at).ok_or(EncodingError::BufferTooSmall)?;
    *slot = byte;
    Ok(at.wrapping_add(1))
}
