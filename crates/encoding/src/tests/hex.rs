// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Hex: one case out, either case in, and never half a byte.

use test_support::generators::bytes;
use test_support::property::check;

use crate::error::EncodingError;
use crate::hex::{decode, decoded_len, encode, encoded_len};

#[test]
fn encoding_is_lower_case() {
    let mut out = [0u8; 8];
    let written = encode(&[0x00, 0xDE, 0xAD, 0xFF], &mut out).expect("room for four bytes");
    assert_eq!(&out[..written], b"00deadff");
}

#[test]
fn decoding_takes_either_case() {
    let mut out = [0u8; 4];
    for text in ["00deadff", "00DEADFF", "00DeAdFf"] {
        let written = decode(text.as_bytes(), &mut out).expect("well formed hex");
        assert_eq!(&out[..written], &[0x00, 0xDE, 0xAD, 0xFF], "{text:?}");
    }
}

#[test]
fn an_odd_number_of_digits_is_refused() {
    let mut out = [0u8; 4];
    assert_eq!(decode(b"abc", &mut out), Err(EncodingError::Length(3)));
    assert_eq!(decode(b"a", &mut out), Err(EncodingError::Length(1)));
    assert_eq!(decoded_len(3), None);
    assert_eq!(decoded_len(4), Some(2));
    assert_eq!(encoded_len(4), Some(8));
    assert_eq!(encoded_len(usize::MAX), None);
}

#[test]
fn a_character_that_is_not_a_digit_is_refused_and_names_its_offset() {
    let mut out = [0u8; 4];
    assert_eq!(decode(b"0g", &mut out), Err(EncodingError::Character(1)));
    assert_eq!(decode(b"g0", &mut out), Err(EncodingError::Character(0)));
    assert_eq!(decode(b"00 1", &mut out), Err(EncodingError::Character(2)));
}

#[test]
fn a_buffer_that_is_too_small_is_an_error_and_writes_nothing() {
    let mut out = [0xAAu8; 3];
    assert_eq!(
        encode(&[1, 2], &mut out),
        Err(EncodingError::BufferTooSmall)
    );
    assert_eq!(out, [0xAA; 3]);
    let mut small = [0u8; 1];
    assert_eq!(
        decode(b"0011", &mut small),
        Err(EncodingError::BufferTooSmall)
    );
}

#[test]
fn every_byte_string_round_trips() {
    check("hex round trip", &bytes(0..=64), |input| {
        let mut text = vec![0u8; encoded_len(input.len()).unwrap_or(0)];
        let written = encode(input, &mut text).map_err(|error| error.to_string())?;
        let mut back = vec![0u8; input.len()];
        let read = decode(text.get(..written).unwrap_or(&[]), &mut back)
            .map_err(|error| error.to_string())?;
        if back.get(..read) == Some(input.as_slice()) {
            Ok(())
        } else {
            Err(format!("{input:?} came back as {back:?}"))
        }
    });
}
