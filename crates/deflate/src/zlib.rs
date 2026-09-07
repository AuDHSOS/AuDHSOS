// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The wrapper of RFC 1950: two bytes in front and a checksum behind.
//!
//! The two in front say which method and how large a window; the four
//! behind are an Adler-32 of the bytes that went in, which is the one
//! thing the inner format has no way of saying. A PDF calls the pair
//! `FlateDecode`; a PNG carries it in its image chunks.

use crate::{Error, Scratch, encode};

/// The method: the only one the format has, which is DEFLATE.
const METHOD: u8 = 8;

/// The window, as the logarithm the header wants: seven means the
/// thirty-two kilobytes this crate looks back over.
const WINDOW_CODE: u8 = 7;

/// The number the two header bytes, read as one, must divide by.
const CHECK: u16 = 31;

/// The modulus of both halves of the checksum.
const BASE: u32 = 65521;

/// Writes `input` into `out` as a zlib stream.
pub(crate) fn wrap(input: &[u8], out: &mut [u8], scratch: &mut Scratch) -> Result<usize, Error> {
    let (first, second) = header();
    let head = out.get_mut(..2).ok_or(Error::Output)?;
    head.copy_from_slice(&[first, second]);
    let body = out.get_mut(2..).ok_or(Error::Output)?;
    let written = encode::compress(input, body, scratch)?;
    let end = written.saturating_add(2);
    let tail = out
        .get_mut(end..end.saturating_add(4))
        .ok_or(Error::Output)?;
    tail.copy_from_slice(&adler32(input).to_be_bytes());
    Ok(end.saturating_add(4))
}

/// Reads a zlib stream into `out`, and says how many bytes it held.
pub(crate) fn unwrap(input: &[u8], out: &mut [u8]) -> Result<usize, Error> {
    let (Some(first), Some(second)) = (input.first().copied(), input.get(1).copied()) else {
        return Err(Error::Input);
    };
    if first & 0x0F != METHOD {
        return Err(Error::Input);
    }
    if u16::from_be_bytes([first, second]).wrapping_rem(CHECK) != 0 {
        return Err(Error::Input);
    }
    // A stream with a dictionary in front of it is one this crate does
    // not write and cannot read: the dictionary is not in the stream.
    if second & 0x20 != 0 {
        return Err(Error::Input);
    }
    let body = input
        .get(2..input.len().saturating_sub(4))
        .ok_or(Error::Input)?;
    let written = crate::decode::inflate(body, out)?;
    let bytes = out.get(..written).ok_or(Error::Output)?;
    let stated = input
        .get(input.len().saturating_sub(4)..)
        .and_then(|tail| <[u8; 4]>::try_from(tail).ok())
        .map(u32::from_be_bytes)
        .ok_or(Error::Input)?;
    if stated != adler32(bytes) {
        return Err(Error::Checksum);
    }
    Ok(written)
}

/// The two header bytes: the method and the window, and the check bits
/// that make the pair divide by thirty-one.
fn header() -> (u8, u8) {
    let first = METHOD | (WINDOW_CODE << 4);
    let rough = u16::from(first).saturating_mul(256);
    let missing = CHECK
        .saturating_sub(rough.wrapping_rem(CHECK))
        .wrapping_rem(CHECK);
    (first, u8::try_from(missing).unwrap_or(0))
}

/// The checksum of RFC 1950, section 9: the sum of the bytes, and the sum
/// of those sums, both to the modulus the document names.
#[must_use]
pub fn adler32(bytes: &[u8]) -> u32 {
    let mut low = 1u32;
    let mut high = 0u32;
    for byte in bytes {
        low = low.saturating_add(u32::from(*byte)).wrapping_rem(BASE);
        high = high.saturating_add(low).wrapping_rem(BASE);
    }
    high.saturating_mul(65536).saturating_add(low)
}
