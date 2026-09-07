// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What this crate writes, read back by what this crate reads.

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use test_support::generators::{bytes, range, vec as gen_vec};
use test_support::property::check;

use crate::{
    Error, Scratch, bound, bound_zlib, compress, compress_zlib, decompress, decompress_zlib,
};

/// Compresses, reads back, and answers with both what came back and how
/// many bytes the stream took.
fn round(input: &[u8]) -> (Vec<u8>, usize) {
    let mut scratch = Scratch::new();
    let mut written = vec![0u8; bound(input.len())];
    let count = compress(input, &mut written, &mut scratch).expect("a compressed stream");
    let mut back = vec![0u8; input.len().saturating_add(1)];
    let read = decompress(written.get(..count).unwrap_or_default(), &mut back)
        .expect("the stream reads back");
    back.truncate(read);
    (back, count)
}

#[test]
fn what_goes_in_comes_back_out() {
    let prose: &[u8] = b"the quick brown fox jumps over the lazy dog, and then the quick brown fox jumps over it again";
    assert_eq!(round(prose).0, prose);
}

#[test]
fn nothing_goes_in_and_nothing_comes_back() {
    assert!(round(b"").0.is_empty());
}

#[test]
fn one_byte_goes_in_and_one_comes_back() {
    assert_eq!(round(b"x").0, b"x");
}

#[test]
fn a_run_of_one_byte_becomes_a_run_and_not_a_list() {
    let input = vec![b'a'; 4096];
    let (back, written) = round(&input);
    assert_eq!(back, input);
    assert!(written < 64, "4096 of one byte took {written} bytes");
}

#[test]
fn a_run_that_reaches_into_itself_reads_back() {
    // Three bytes copied from one byte back are that byte three times,
    // which is the case a decoder gets wrong if it copies in one go.
    let mut input = Vec::new();
    input.extend_from_slice(b"abc");
    input.extend(core::iter::repeat_n(b'z', 300));
    assert_eq!(round(&input).0, input);
}

#[test]
fn text_that_repeats_gets_smaller() {
    let mut input = Vec::new();
    for _ in 0..40 {
        input.extend_from_slice(b"a line that says the same thing over and over again\n");
    }
    let (back, written) = round(&input);
    assert_eq!(back, input);
    assert!(
        written < input.len() / 8,
        "{} bytes became {written}",
        input.len()
    );
}

#[test]
fn what_will_not_compress_is_carried_unchanged() {
    // Bytes with no run in them: coding each as a literal would cost more
    // than the byte itself, so the stream says them plainly.
    let input: Vec<u8> = (0..=255u8).flat_map(|byte| [byte, byte ^ 0x5A]).collect();
    let (back, written) = round(&input);
    assert_eq!(back, input);
    assert!(
        written <= bound(input.len()),
        "{written} is more than the bound"
    );
}

#[test]
fn a_stream_longer_than_one_stored_block_is_still_carried() {
    let input: Vec<u8> = (0..70_000u32)
        .map(|index| u8::try_from(index.wrapping_mul(2_654_435_761) >> 24).unwrap_or(0))
        .collect();
    assert_eq!(round(&input).0, input);
}

#[test]
fn a_zlib_stream_carries_its_wrapper_and_its_checksum() {
    let input = b"a zlib stream has two bytes in front of it and four behind";
    let mut scratch = Scratch::new();
    let mut written = vec![0u8; bound_zlib(input.len())];
    let count = compress_zlib(input, &mut written, &mut scratch).expect("a zlib stream");
    let stream = written.get(..count).unwrap_or_default();
    assert_eq!(stream.first().copied(), Some(0x78));
    let head = u16::from_be_bytes([
        stream.first().copied().unwrap_or(0),
        stream.get(1).copied().unwrap_or(0),
    ]);
    assert_eq!(head % 31, 0, "the header must divide by thirty-one");
    let mut back = vec![0u8; input.len()];
    let read = decompress_zlib(stream, &mut back).expect("it reads back");
    assert_eq!(back.get(..read), Some(input.as_slice()));
}

#[test]
fn a_zlib_stream_whose_checksum_is_wrong_is_refused() {
    let input = b"the checksum is the one thing the inner format cannot say";
    let mut scratch = Scratch::new();
    let mut written = vec![0u8; bound_zlib(input.len())];
    let count = compress_zlib(input, &mut written, &mut scratch).expect("a zlib stream");
    if let Some(byte) = written.get_mut(count.saturating_sub(1)) {
        *byte ^= 0xFF;
    }
    let mut back = vec![0u8; input.len()];
    assert_eq!(
        decompress_zlib(written.get(..count).unwrap_or_default(), &mut back),
        Err(Error::Checksum)
    );
}

#[test]
fn a_wrapper_this_crate_does_not_know_is_refused() {
    let mut back = [0u8; 16];
    assert_eq!(decompress_zlib(&[], &mut back), Err(Error::Input));
    // A method that is not DEFLATE, a header that does not divide, and a
    // stream that names a dictionary it does not carry.
    assert_eq!(
        decompress_zlib(&[0x79, 0x01, 0, 0, 0, 0], &mut back),
        Err(Error::Input)
    );
    assert_eq!(
        decompress_zlib(&[0x78, 0x00, 0, 0, 0, 0], &mut back),
        Err(Error::Input)
    );
    assert_eq!(
        decompress_zlib(&[0x78, 0xBB, 0, 0, 0, 0], &mut back),
        Err(Error::Input)
    );
}

#[test]
fn a_buffer_with_no_room_is_an_error_and_not_a_shorter_stream() {
    let input = vec![b'q'; 4000];
    let mut scratch = Scratch::new();
    let mut written = [0u8; 4];
    assert_eq!(
        compress(&input, &mut written, &mut scratch),
        Err(Error::Output)
    );
    let mut room = vec![0u8; bound(input.len())];
    let count = compress(&input, &mut room, &mut scratch).expect("a stream");
    let mut back = [0u8; 8];
    assert_eq!(
        decompress(room.get(..count).unwrap_or_default(), &mut back),
        Err(Error::Output)
    );
}

#[test]
fn a_stream_of_nonsense_is_refused_and_does_not_run_away() {
    let mut back = [0u8; 64];
    assert_eq!(decompress(&[], &mut back), Err(Error::Input));
    // A block type the format does not have.
    assert_eq!(decompress(&[0x07], &mut back), Err(Error::Input));
    // A stored block whose two lengths do not agree.
    assert_eq!(
        decompress(&[0x01, 0x05, 0x00, 0x00, 0x00], &mut back),
        Err(Error::Input)
    );
}

#[test]
fn whatever_goes_in_comes_back_out() {
    check("bytes round trip", &bytes(0..=600), |input| {
        let (back, _) = round(input);
        if back == *input {
            Ok(())
        } else {
            Err(alloc::format!(
                "{} bytes came back as {}",
                input.len(),
                back.len()
            ))
        }
    });
}

#[test]
fn whatever_repeats_comes_back_out() {
    // Bytes from a small alphabet, which is what makes runs: the input a
    // compressor is actually given, rather than noise.
    let generator = gen_vec(range(b'a'..=b'f'), 0..=800);
    check("runs round trip", &generator, |input| {
        let (back, _) = round(input);
        if back == *input {
            Ok(())
        } else {
            Err(alloc::format!(
                "{} bytes came back as {}",
                input.len(),
                back.len()
            ))
        }
    });
}

#[test]
fn what_will_not_compress_and_is_long_is_carried_in_several_blocks() {
    // A stored block holds sixty-five thousand bytes and no more, so
    // this takes two of them — and the first says it is not the last.
    let mut state = 0x1234_5678u32;
    let input: Vec<u8> = (0..70_000u32)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            u8::try_from(state >> 24).unwrap_or(0)
        })
        .collect();
    let (back, written) = round(&input);
    assert_eq!(back, input);
    assert!(
        written > input.len(),
        "random bytes should be carried, not coded"
    );
}
