// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Base64 against the vectors of RFC 4648 and against the texts a strict
//! decoder has to refuse.

use test_support::generators::bytes;
use test_support::property::check;

use crate::base64::{decode, decoded_len, encode, encoded_len};
use crate::error::EncodingError;

/// The vectors of RFC 4648 section 10, for the lengths zero to six.
const VECTORS: [(&str, &str); 7] = [
    ("", ""),
    ("f", "Zg=="),
    ("fo", "Zm8="),
    ("foo", "Zm9v"),
    ("foob", "Zm9vYg=="),
    ("fooba", "Zm9vYmE="),
    ("foobar", "Zm9vYmFy"),
];

/// Encodes into a buffer of exactly the right size.
fn encoded(input: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; encoded_len(input.len()).expect("a length that fits")];
    let written = encode(input, &mut out).expect("a buffer of the right size");
    out.truncate(written);
    out
}

/// Decodes into a buffer of exactly the right size.
fn decoded(text: &str) -> Result<Vec<u8>, EncodingError> {
    let mut out = vec![0u8; decoded_len(text.len()).unwrap_or(0)];
    let written = decode(text.as_bytes(), &mut out)?;
    out.truncate(written);
    Ok(out)
}

#[test]
fn the_vectors_of_the_rfc_encode_and_decode() {
    for (plain, text) in VECTORS {
        assert_eq!(encoded(plain.as_bytes()), text.as_bytes(), "{plain:?}");
        assert_eq!(decoded(text), Ok(plain.as_bytes().to_vec()), "{text:?}");
    }
}

#[test]
fn a_buffer_one_byte_too_small_is_an_error_and_writes_nothing() {
    for (plain, text) in VECTORS {
        let Some(needed) = encoded_len(plain.len()).and_then(|len| len.checked_sub(1)) else {
            continue;
        };
        let mut out = vec![0xAAu8; text.len()];
        assert_eq!(
            encode(plain.as_bytes(), &mut out[..needed]),
            Err(EncodingError::BufferTooSmall),
            "{plain:?}"
        );
        assert!(
            out.iter().all(|byte| *byte == 0xAA),
            "a failed encode wrote into the buffer"
        );
    }
    let mut out = [0u8; 1];
    assert_eq!(
        decode(b"Zm9v", &mut out),
        Err(EncodingError::BufferTooSmall)
    );
}

#[test]
fn a_length_that_is_not_a_multiple_of_four_is_refused() {
    for text in ["Z", "Zg", "Zm9", "Zm9vY"] {
        assert_eq!(
            decoded(text),
            Err(EncodingError::Length(text.len())),
            "{text:?}"
        );
    }
    assert_eq!(decoded_len(3), None);
    assert_eq!(decoded_len(4), Some(3));
    assert_eq!(encoded_len(0), Some(0));
    assert_eq!(encoded_len(usize::MAX), None);
}

#[test]
fn the_padding_has_to_be_one_or_two_characters_at_the_end() {
    // A missing pad, three pads, a pad in the middle, and a lone pad
    // before a character.
    for text in ["Zg", "Zg==Zg==", "Z===", "====", "=Zm8", "Z=g="] {
        let result = decoded(text);
        assert!(
            matches!(
                result,
                Err(EncodingError::Padding | EncodingError::Length(_))
            ),
            "{text:?} gave {result:?}"
        );
    }
}

#[test]
fn a_character_outside_the_alphabet_is_refused_and_names_its_offset() {
    assert_eq!(decoded("Zm9-"), Err(EncodingError::Character(3)));
    assert_eq!(decoded("-m9v"), Err(EncodingError::Character(0)));
    assert_eq!(decoded("Zm9v Zm9v"), Err(EncodingError::Length(9)));
    // Whitespace inside a quantum: a strict decoder skips nothing.
    assert_eq!(decoded("Zm 9v"), Err(EncodingError::Length(5)));
    assert_eq!(decoded("Z m9v"), Err(EncodingError::Length(5)));
    assert_eq!(decoded("Zm\n9v"), Err(EncodingError::Length(5)));
    assert_eq!(decoded("Zm9vZm 9v"), Err(EncodingError::Length(9)));
    assert_eq!(decoded("Zm9v\tm9v"), Err(EncodingError::Character(4)));
}

#[test]
fn the_unused_bits_of_the_last_quantum_have_to_be_zero() {
    // `Zg==` is the canonical text of `f`; `Zh==` decodes to the same byte
    // and must therefore be refused.
    assert_eq!(decoded("Zg=="), Ok(b"f".to_vec()));
    assert_eq!(decoded("Zh=="), Err(EncodingError::TrailingBits));
    assert_eq!(decoded("Zm8="), Ok(b"fo".to_vec()));
    assert_eq!(decoded("Zm9="), Err(EncodingError::TrailingBits));
}

#[test]
fn every_byte_string_round_trips() {
    check("base64 round trip", &bytes(0..=64), |input| {
        let text = encoded(input);
        let mut out = vec![0u8; input.len()];
        let written = decode(&text, &mut out).map_err(|error| error.to_string())?;
        if out.get(..written) == Some(input.as_slice()) {
            Ok(())
        } else {
            Err(format!("{input:?} came back as {out:?}"))
        }
    });
}

#[test]
fn no_text_makes_the_decoder_panic() {
    check("base64 never panics", &bytes(0..=32), |input| {
        let mut out = [0u8; 32];
        let _ = decode(input, &mut out);
        Ok(())
    });
}
