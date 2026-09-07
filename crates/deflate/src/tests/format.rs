// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The pieces a stream is made of: the bits, the codes, the tables of
//! RFC 1951, and the checksum of RFC 1950.

use crate::bits::{Reader, Writer};
use crate::huffman::{Codes, Tree};
use crate::tables::{
    DISTANCES, LITERALS, distance_code, distance_of, fixed_distance_lengths, fixed_literal_lengths,
    length_code, length_of,
};
use crate::{Error, adler32, bound};

#[test]
fn a_bit_written_is_the_bit_read() {
    let mut out = [0u8; 8];
    let mut writer = Writer::new(&mut out);
    writer.bits(1, 1).expect("room");
    writer.bits(5, 3).expect("room");
    writer.bits(0x2A, 6).expect("room");
    let written = writer.finish().expect("room");
    let mut reader = Reader::new(out.get(..written).unwrap_or_default());
    assert_eq!(reader.bits(1), Ok(1));
    assert_eq!(reader.bits(3), Ok(5));
    assert_eq!(reader.bits(6), Ok(0x2A));
}

#[test]
fn a_writer_with_no_room_says_so() {
    let mut out = [0u8; 1];
    let mut writer = Writer::new(&mut out);
    assert_eq!(writer.bits(0xFFFF, 16), Err(Error::Output));
}

#[test]
fn a_reader_that_runs_out_says_so() {
    let mut reader = Reader::new(&[0x01]);
    assert_eq!(reader.bits(8), Ok(1));
    assert_eq!(reader.bits(1), Err(Error::Input));
}

#[test]
fn a_huffman_code_goes_out_with_its_first_bit_first() {
    // Three bits of the code 0b101, most significant first, are the bits
    // 1, 0, 1 — and a reader taking them one at a time sees them so.
    let mut out = [0u8; 2];
    let mut writer = Writer::new(&mut out);
    writer.code(0b101, 3).expect("room");
    let written = writer.finish().expect("room");
    let mut reader = Reader::new(out.get(..written).unwrap_or_default());
    assert_eq!(reader.bit(), Ok(1));
    assert_eq!(reader.bit(), Ok(0));
    assert_eq!(reader.bit(), Ok(1));
}

#[test]
fn what_is_aligned_is_written_whole() {
    let mut out = [0u8; 8];
    let mut writer = Writer::new(&mut out);
    writer.bits(1, 3).expect("room");
    writer.align().expect("room");
    writer.bytes(&[0xAB, 0xCD]).expect("room");
    let written = writer.finish().expect("room");
    assert_eq!(out.get(..written), Some([0x01, 0xAB, 0xCD].as_slice()));
    let mut reader = Reader::new(&out);
    assert_eq!(reader.bits(3), Ok(1));
    reader.align();
    assert_eq!(reader.bytes(2), Ok([0xAB, 0xCD].as_slice()));
    assert_eq!(reader.bytes(99), Err(Error::Input));
}

#[test]
fn the_fixed_code_is_the_one_the_document_states() {
    // RFC 1951, section 3.2.6, states the codes the fixed lengths build,
    // and these are the four corners of that table.
    let codes = Codes::<LITERALS>::new(&fixed_literal_lengths());
    assert_eq!(codes.code.first().copied(), Some(0b0011_0000));
    assert_eq!(codes.code.get(143).copied(), Some(0b1011_1111));
    assert_eq!(codes.code.get(144).copied(), Some(0b1_1001_0000));
    assert_eq!(codes.code.get(255).copied(), Some(0b1_1111_1111));
    assert_eq!(codes.code.get(256).copied(), Some(0b000_0000));
    assert_eq!(codes.code.get(279).copied(), Some(0b001_0111));
    assert_eq!(codes.code.get(280).copied(), Some(0b1100_0000));
    assert_eq!(codes.code.get(287).copied(), Some(0b1100_0111));
}

#[test]
fn a_code_written_is_the_symbol_read() {
    let lengths = fixed_literal_lengths();
    let codes = Codes::<LITERALS>::new(&lengths);
    let tree = Tree::<LITERALS>::new(&lengths);
    let mut out = [0u8; 64];
    let mut writer = Writer::new(&mut out);
    for symbol in [0u16, 65, 143, 144, 255, 256, 279, 285] {
        let at = usize::from(symbol);
        let code = codes.code.get(at).copied().unwrap_or_default();
        let length = codes.length.get(at).copied().unwrap_or_default();
        writer.code(code, length).expect("room");
    }
    let written = writer.finish().expect("room");
    let mut reader = Reader::new(out.get(..written).unwrap_or_default());
    for symbol in [0u16, 65, 143, 144, 255, 256, 279, 285] {
        assert_eq!(tree.decode(&mut reader), Ok(symbol));
    }
}

#[test]
fn the_distance_code_is_five_bits_of_its_own_number() {
    let codes = Codes::<DISTANCES>::new(&fixed_distance_lengths());
    assert_eq!(codes.code.first().copied(), Some(0));
    assert_eq!(codes.code.get(1).copied(), Some(1));
    assert_eq!(codes.code.get(29).copied(), Some(29));
}

#[test]
fn a_tree_of_nothing_reads_nothing() {
    let tree = Tree::<LITERALS>::new(&[0u8; LITERALS]);
    let mut reader = Reader::new(&[0xFF, 0xFF, 0xFF]);
    assert_eq!(tree.decode(&mut reader), Err(Error::Input));
}

#[test]
fn every_length_is_written_as_a_code_that_means_it_again() {
    for length in 3..=258usize {
        let (code, extra, value) = length_code(length).expect("a code");
        let (bits, base) = length_of(code).expect("the code again");
        assert_eq!(bits, extra, "length {length}");
        assert_eq!(
            usize::from(base).saturating_add(usize::from(value)),
            length,
            "length {length}"
        );
    }
    assert_eq!(length_code(2), None);
}

#[test]
fn every_distance_is_written_as_a_code_that_means_it_again() {
    for distance in 1..=32768usize {
        let (code, extra, value) = distance_code(distance).expect("a code");
        let (bits, base) = distance_of(code).expect("the code again");
        assert_eq!(bits, extra, "distance {distance}");
        assert_eq!(
            usize::from(base).saturating_add(usize::from(value)),
            distance,
            "distance {distance}"
        );
    }
    assert_eq!(distance_code(0), None);
    assert_eq!(length_of(9), None);
    assert_eq!(distance_of(30), None);
}

#[test]
fn the_checksum_is_the_one_the_document_defines() {
    // RFC 1950, section 9: one is the sum of the bytes and the other the
    // sum of those sums, both to the modulus given there.
    assert_eq!(adler32(b""), 1);
    assert_eq!(adler32(b"a"), 0x0062_0062);
    assert_eq!(adler32(b"abc"), 0x024d_0127);
    assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
}

#[test]
fn the_room_a_result_needs_counts_the_blocks_it_takes() {
    assert_eq!(bound(0), 5);
    assert_eq!(bound(10), 15);
    assert_eq!(bound(65535), 65540);
    assert_eq!(bound(65536), 65546);
    assert_eq!(crate::bound_zlib(10), 21);
}

#[test]
fn a_code_length_the_format_does_not_have_is_ignored() {
    // Nothing in a stream can say sixteen — the lengths come out of three
    // bits or out of an alphabet that stops at fifteen — but a tree is
    // built from a slice, and a slice can say anything.
    // Only the two ones are a code; the first bit read tells them apart.
    let tree = Tree::<8>::new(&[16, 1, 1, 200]);
    let mut reader = Reader::new(&[0b0000_0001]);
    assert_eq!(tree.decode(&mut reader), Ok(2));
    let mut reader = Reader::new(&[0b0000_0000]);
    assert_eq!(tree.decode(&mut reader), Ok(1));
}

#[test]
fn more_symbols_than_a_tree_holds_are_dropped_and_not_written_past() {
    let tree = Tree::<4>::new(&[1, 1, 2, 2, 3, 3, 3, 3]);
    let mut reader = Reader::new(&[0b0000_0000]);
    assert_eq!(tree.decode(&mut reader), Ok(0));
}

#[test]
fn a_code_of_one_symbol_is_still_a_code() {
    let codes = Codes::<2>::new(&[1, 0]);
    assert_eq!(codes.code.first().copied(), Some(0));
    assert_eq!(codes.length.first().copied(), Some(1));
}

#[test]
fn every_error_of_the_crate_reads_as_a_sentence() {
    extern crate alloc;
    use alloc::string::ToString as _;
    for error in [Error::Output, Error::Input, Error::Checksum] {
        let said = error.to_string();
        assert!(said.len() > 20, "{error:?} says `{said}`");
        assert!(said.starts_with("the "), "{error:?} says `{said}`");
    }
}

#[test]
fn a_table_a_caller_did_not_fill_is_the_empty_one() {
    let made = crate::Scratch::default();
    let empty = crate::Scratch::new();
    assert!(made.head(b"abc", 0).is_none());
    assert!(empty.previous(0).is_none());
}
