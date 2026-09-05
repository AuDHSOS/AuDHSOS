// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::utf16`.

use crate::utf16::{Utf16Error, encode};

#[test]
fn a_name_is_encoded_with_its_terminator() {
    let mut buffer = [0u16; 32];
    let encoded = encode("AUDHSOS\\KERNEL.ELF", &mut buffer).unwrap();
    assert_eq!(encoded.len(), 19);
    assert_eq!(encoded.first().copied(), Some(u16::from(b'A')));
    assert_eq!(encoded.last().copied(), Some(0), "the terminator");
    let text: String = encoded[..encoded.len() - 1]
        .iter()
        .map(|unit| char::from(u8::try_from(*unit).unwrap()))
        .collect();
    assert_eq!(text, "AUDHSOS\\KERNEL.ELF");
}

#[test]
fn an_empty_name_is_a_single_terminator() {
    let mut buffer = [0xFFFFu16; 4];
    assert_eq!(encode("", &mut buffer), Ok([0u16].as_slice()));
}

#[test]
fn a_buffer_that_cannot_hold_the_terminator_is_rejected() {
    let mut exact = [0u16; 3];
    assert_eq!(encode("ab", &mut exact).map(<[u16]>::len), Ok(3));
    let mut short = [0u16; 2];
    assert_eq!(
        encode("ab", &mut short),
        Err(Utf16Error::BufferTooSmall { needed: 3 })
    );
    let mut empty: [u16; 0] = [];
    assert_eq!(
        encode("", &mut empty),
        Err(Utf16Error::BufferTooSmall { needed: 1 })
    );
}

#[test]
fn a_name_outside_ascii_is_rejected() {
    let mut buffer = [0u16; 32];
    assert_eq!(encode("kärnel", &mut buffer), Err(Utf16Error::NotAscii));
    for error in [
        Utf16Error::NotAscii,
        Utf16Error::BufferTooSmall { needed: 4 },
    ] {
        assert!(!format!("{error}").is_empty());
    }
}
