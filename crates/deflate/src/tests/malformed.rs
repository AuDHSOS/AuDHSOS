// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Streams written bit by bit: code tables RFC 1951 does or does not
//! define, runs of fixed blocks, and zlib headers RFC 1950 forbids.

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use crate::bits::Writer;
use crate::huffman::Codes;
use crate::tables::{LITERALS, fixed_literal_lengths};
use crate::{Error, Scratch, bound_zlib, compress_zlib, decompress, decompress_zlib};

/// The order of the code length code lengths, RFC 1951, section 3.2.7.
const ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

/// A code length code of sixteen four-bit codes for the lengths 0 to 15.
fn four_bits() -> [u8; 19] {
    core::array::from_fn(|symbol| if symbol < 16 { 4 } else { 0 })
}

/// Literal lengths: `count` symbols, the listed ones set.
fn lengths(count: usize, set: &[(usize, u8)]) -> Vec<u8> {
    let mut lengths = vec![0u8; count];
    for (symbol, length) in set {
        if let Some(slot) = lengths.get_mut(*symbol) {
            *slot = *length;
        }
    }
    lengths
}

/// One final dynamic block: the code length code `code_lengths`, the
/// literal and distance lengths, then `data` as codes of given length.
fn dynamic(
    code_lengths: &[u8; 19],
    literals: &[u8],
    distances: &[u8],
    data: &[(u16, u8)],
) -> Vec<u8> {
    let mut out = vec![0u8; 512];
    let mut writer = Writer::new(&mut out);
    writer.bits(1, 1).expect("room");
    writer.bits(2, 2).expect("room");
    let hlit = u32::try_from(literals.len().saturating_sub(257)).unwrap_or(0);
    let hdist = u32::try_from(distances.len().saturating_sub(1)).unwrap_or(0);
    writer.bits(hlit, 5).expect("room");
    writer.bits(hdist, 5).expect("room");
    writer.bits(15, 4).expect("room");
    for symbol in ORDER {
        let length = code_lengths.get(symbol).copied().unwrap_or(0);
        writer.bits(u32::from(length), 3).expect("room");
    }
    let codes = Codes::<19>::new(code_lengths);
    for length in literals.iter().chain(distances) {
        let at = usize::from(*length);
        let code = codes.code.get(at).copied().unwrap_or(0);
        let bits = codes.length.get(at).copied().unwrap_or(0);
        writer.code(code, bits).expect("room");
    }
    for (code, bits) in data {
        writer.code(*code, *bits).expect("room");
    }
    let written = writer.finish().expect("room");
    out.truncate(written);
    out
}

/// What `stream` decodes to, or the error.
fn inflate(stream: &[u8]) -> Result<Vec<u8>, Error> {
    let mut out = vec![0u8; 64];
    let read = decompress(stream, &mut out)?;
    out.truncate(read);
    Ok(out)
}

#[test]
fn a_complete_code_with_no_distance_codes_reads_back() {
    // 'a' and the end of block in one bit each; one distance length of
    // zero is no distance code, RFC 1951, section 3.2.7.
    let literals = lengths(257, &[(97, 1), (256, 1)]);
    let stream = dynamic(&four_bits(), &literals, &[0], &[(0, 1), (1, 1)]);
    assert_eq!(inflate(&stream), Ok(b"a".to_vec()));
}

#[test]
fn one_distance_code_of_one_bit_reads_back() {
    // 'a' is 0, the end of block 10, the length 3 is 11; the one distance
    // code is 0 and means distance 1.
    let literals = lengths(258, &[(97, 1), (256, 2), (257, 2)]);
    let stream = dynamic(
        &four_bits(),
        &literals,
        &[1],
        &[(0, 1), (0b11, 2), (0, 1), (0b10, 2)],
    );
    assert_eq!(inflate(&stream), Ok(b"aaaa".to_vec()));
}

#[test]
fn an_over_subscribed_literal_code_is_refused() {
    // Two one-bit codes and a two-bit one: 0 would read 'a' and 1 the end
    // of block, which a decoder without the check accepts.
    let literals = lengths(257, &[(97, 1), (98, 2), (256, 1)]);
    let stream = dynamic(&four_bits(), &literals, &[0], &[(0, 1), (1, 1)]);
    assert_eq!(inflate(&stream), Err(Error::Input));
}

#[test]
fn an_incomplete_literal_code_is_refused() {
    // 'a' is 0 and the end of block 10; the code 11 is unused.
    let literals = lengths(257, &[(97, 1), (256, 2)]);
    let stream = dynamic(&four_bits(), &literals, &[0], &[(0, 1), (0b10, 2)]);
    assert_eq!(inflate(&stream), Err(Error::Input));
}

#[test]
fn one_literal_code_of_one_bit_is_refused() {
    // RFC 1951, section 3.2.7, permits the lone one-bit code for distances
    // only.
    let literals = lengths(257, &[(256, 1)]);
    let stream = dynamic(&four_bits(), &literals, &[0], &[(0, 1)]);
    assert_eq!(inflate(&stream), Err(Error::Input));
}

#[test]
fn an_over_subscribed_distance_code_is_refused() {
    let literals = lengths(257, &[(97, 1), (256, 1)]);
    let stream = dynamic(&four_bits(), &literals, &[1, 1, 1], &[(0, 1), (1, 1)]);
    assert_eq!(inflate(&stream), Err(Error::Input));
}

#[test]
fn an_incomplete_code_length_code_is_refused() {
    // Fifteen four-bit codes leave the sixteenth unused.
    let mut code_lengths = four_bits();
    if let Some(slot) = code_lengths.get_mut(15) {
        *slot = 0;
    }
    let literals = lengths(257, &[(97, 1), (256, 1)]);
    let stream = dynamic(&code_lengths, &literals, &[0], &[(0, 1), (1, 1)]);
    assert_eq!(inflate(&stream), Err(Error::Input));
}

#[test]
fn a_run_of_fixed_blocks_reads_back() {
    // Ten thousand empty fixed blocks, then one per byte of "abc".
    let codes = Codes::<LITERALS>::new(&fixed_literal_lengths());
    let code = |symbol: usize| {
        (
            codes.code.get(symbol).copied().unwrap_or(0),
            codes.length.get(symbol).copied().unwrap_or(0),
        )
    };
    let mut out = vec![0u8; 16384];
    let mut writer = Writer::new(&mut out);
    let (end, end_bits) = code(256);
    for _ in 0..10_000 {
        writer.bits(0, 1).expect("room");
        writer.bits(1, 2).expect("room");
        writer.code(end, end_bits).expect("room");
    }
    for (index, byte) in b"abc".iter().enumerate() {
        writer.bits(u32::from(index == 2), 1).expect("room");
        writer.bits(1, 2).expect("room");
        let (literal, bits) = code(usize::from(*byte));
        writer.code(literal, bits).expect("room");
        writer.code(end, end_bits).expect("room");
    }
    let written = writer.finish().expect("room");
    assert_eq!(
        inflate(out.get(..written).unwrap_or_default()),
        Ok(b"abc".to_vec())
    );
}

/// A zlib stream of `input` whose header says window code `cinfo`.
fn zlib_with_window(input: &[u8], cinfo: u8) -> Vec<u8> {
    let mut scratch = Scratch::new();
    let mut stream = vec![0u8; bound_zlib(input.len())];
    let count = compress_zlib(input, &mut stream, &mut scratch).expect("a zlib stream");
    stream.truncate(count);
    let first = (cinfo << 4) | 8;
    let rough = u16::from(first).saturating_mul(256);
    let check = (31u16.saturating_sub(rough % 31)) % 31;
    if let Some(slot) = stream.get_mut(0) {
        *slot = first;
    }
    if let Some(slot) = stream.get_mut(1) {
        *slot = u8::try_from(check).unwrap_or(0);
    }
    stream
}

#[test]
fn a_zlib_window_above_thirty_two_kilobytes_is_refused() {
    // RFC 1950, section 2.2: CINFO above seven is not allowed.
    let input = b"a window the format does not have";
    let mut back = vec![0u8; input.len()];
    for cinfo in 0..=7u8 {
        let stream = zlib_with_window(input, cinfo);
        assert_eq!(
            decompress_zlib(&stream, &mut back),
            Ok(input.len()),
            "CINFO {cinfo}"
        );
    }
    for cinfo in 8..=15u8 {
        let stream = zlib_with_window(input, cinfo);
        assert_eq!(
            decompress_zlib(&stream, &mut back),
            Err(Error::Input),
            "CINFO {cinfo}"
        );
    }
}
