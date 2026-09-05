// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Hexadecimal: lower case on the way out, either case on the way in.
//!
//! The asymmetry is deliberate. A hex dump is written by a program and
//! read by a person, so one canonical output is worth having; the input,
//! however, is often typed or copied from somewhere that shouts, and
//! refusing `AB` where `ab` would do buys nothing. What is refused is an
//! odd number of digits, because half a byte is not a byte.

use crate::error::EncodingError;

/// The digits of the lower-case alphabet.
const DIGITS: &[u8; 16] = b"0123456789abcdef";

/// The number of characters a text of `bytes` bytes takes, or `None` when
/// that count does not fit a `usize`.
#[must_use]
pub const fn encoded_len(bytes: usize) -> Option<usize> {
    bytes.checked_mul(2)
}

/// The number of bytes a text of `characters` characters decodes to, or
/// `None` when the length is odd.
#[must_use]
pub const fn decoded_len(characters: usize) -> Option<usize> {
    if characters.wrapping_rem(2) == 0 {
        characters.checked_div(2)
    } else {
        None
    }
}

/// Writes `input` as lower-case hex into `out` and answers how many
/// characters it wrote.
///
/// # Errors
///
/// [`EncodingError::BufferTooSmall`] when `out` is shorter than twice the
/// input, in which case nothing is written.
pub fn encode(input: &[u8], out: &mut [u8]) -> Result<usize, EncodingError> {
    let needed = encoded_len(input.len()).ok_or(EncodingError::BufferTooSmall)?;
    if out.len() < needed {
        return Err(EncodingError::BufferTooSmall);
    }
    let mut written = 0usize;
    for byte in input {
        let high = usize::from(byte.wrapping_shr(4));
        let low = usize::from(byte & 0x0F);
        written = put(out, written, DIGITS.get(high).copied().unwrap_or(b'0'))?;
        written = put(out, written, DIGITS.get(low).copied().unwrap_or(b'0'))?;
    }
    Ok(written)
}

/// Reads `input` as hex of either case into `out` and answers how many
/// bytes it wrote.
///
/// # Errors
///
/// [`EncodingError::Length`] for an odd number of characters,
/// [`EncodingError::Character`] for anything that is not a hex digit, and
/// [`EncodingError::BufferTooSmall`] when `out` is too short.
pub fn decode(input: &[u8], out: &mut [u8]) -> Result<usize, EncodingError> {
    let bytes = decoded_len(input.len()).ok_or(EncodingError::Length(input.len()))?;
    if out.len() < bytes {
        return Err(EncodingError::BufferTooSmall);
    }
    let mut written = 0usize;
    let (pairs, _) = input.as_chunks::<2>();
    for (index, [high, low]) in pairs.iter().copied().enumerate() {
        let offset = index.wrapping_mul(2);
        let high = digit(high, offset)?;
        let low = digit(low, offset.wrapping_add(1))?;
        written = put(out, written, high.wrapping_shl(4) | low)?;
    }
    Ok(written)
}

/// The value of one hex digit of either case.
const fn digit(byte: u8, at: usize) -> Result<u8, EncodingError> {
    match byte {
        b'0'..=b'9' => Ok(byte.wrapping_sub(b'0')),
        b'a'..=b'f' => Ok(byte.wrapping_sub(b'a').wrapping_add(10)),
        b'A'..=b'F' => Ok(byte.wrapping_sub(b'A').wrapping_add(10)),
        _ => Err(EncodingError::Character(at)),
    }
}

/// Writes one byte at `at` and answers the next position.
fn put(out: &mut [u8], at: usize, byte: u8) -> Result<usize, EncodingError> {
    let slot = out.get_mut(at).ok_or(EncodingError::BufferTooSmall)?;
    *slot = byte;
    Ok(at.wrapping_add(1))
}
