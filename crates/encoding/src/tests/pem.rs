// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! PEM against RFC 7468: what a block is, and everything that looks like
//! one and is not.

use core::num::NonZeroUsize;

use test_support::property::check;

use crate::error::EncodingError;
use crate::pem::{
    Block, LINE, decode, decode_wrapped, encode, encode_wrapped, encoded_len, encoded_len_wrapped,
};
use crate::strategies::{any_pem_bytes, write_block};

/// A payload of a hundred bytes, so that a block has two body lines: one
/// full and one short.
fn payload() -> Vec<u8> {
    (0u16..100)
        .map(|value| u8::try_from(value % 251).unwrap_or(0))
        .collect()
}

/// Decodes into a buffer that is large enough.
fn read(text: &[u8]) -> Result<(String, Vec<u8>), EncodingError> {
    let mut out = vec![0u8; text.len()];
    let block = decode(text, &mut out)?;
    Ok((block.label.to_owned(), block.bytes.to_vec()))
}

#[test]
fn a_block_reads_back_the_label_and_the_bytes_it_was_written_with() {
    let bytes = payload();
    let text = write_block("CERTIFICATE", &bytes);
    let rendered = String::from_utf8(text.clone()).expect("ascii");
    assert!(rendered.starts_with("-----BEGIN CERTIFICATE-----\n"));
    assert!(rendered.ends_with("-----END CERTIFICATE-----\n"));
    assert_eq!(read(&text), Ok(("CERTIFICATE".to_owned(), bytes)));
}

#[test]
fn every_body_line_but_the_last_is_sixty_four_characters() {
    let text = write_block("CERTIFICATE", &payload());
    let rendered = String::from_utf8(text).expect("ascii");
    let body: Vec<&str> = rendered
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect();
    let (last, full) = body.split_last().expect("a body");
    assert!(!full.is_empty(), "this payload needs more than one line");
    for line in full {
        assert_eq!(line.len(), LINE, "{line:?}");
    }
    assert!(!last.is_empty() && last.len() <= LINE, "{last:?}");
}

#[test]
fn explanatory_text_before_the_block_is_skipped_and_not_returned() {
    let bytes = payload();
    let mut text = b"Certificate:\n  Issuer: the test authority\n\n".to_vec();
    text.extend_from_slice(&write_block("CERTIFICATE", &bytes));
    assert_eq!(read(&text), Ok(("CERTIFICATE".to_owned(), bytes)));
}

#[test]
fn anything_after_the_end_line_is_refused() {
    let mut text = write_block("CERTIFICATE", &payload());
    text.extend_from_slice(b"and one more thing\n");
    assert_eq!(read(&text), Err(EncodingError::TrailingData));

    // A line terminator after the end line is not data.
    let mut newlines = write_block("CERTIFICATE", &payload());
    newlines.extend_from_slice(b"\n\n");
    assert!(read(&newlines).is_ok());
}

#[test]
fn the_end_line_has_to_name_the_label_of_the_begin_line() {
    let text = write_block("CERTIFICATE", &payload());
    let rendered = String::from_utf8(text).expect("ascii");
    let mismatched = rendered.replace("-----END CERTIFICATE-----", "-----END PUBLIC KEY-----");
    assert_eq!(
        read(mismatched.as_bytes()),
        Err(EncodingError::LabelMismatch)
    );
}

#[test]
fn a_missing_frame_line_is_refused() {
    let rendered = String::from_utf8(write_block("CERTIFICATE", &payload())).expect("ascii");
    let no_end = rendered.replace("-----END CERTIFICATE-----\n", "");
    assert_eq!(read(no_end.as_bytes()), Err(EncodingError::MissingEnd));
    let no_begin = rendered.replace("-----BEGIN CERTIFICATE-----\n", "");
    assert_eq!(read(no_begin.as_bytes()), Err(EncodingError::MissingBegin));
    assert_eq!(read(b""), Err(EncodingError::MissingBegin));
}

#[test]
fn a_body_line_that_is_not_sixty_four_characters_is_refused() {
    let rendered = String::from_utf8(write_block("CERTIFICATE", &payload())).expect("ascii");
    let mut lines: Vec<&str> = rendered.lines().collect();
    // Move four characters from the first body line to the second, which
    // leaves the base64 intact and the line lengths wrong.
    let first = lines.get(1).copied().expect("a first body line");
    let (keep, moved) = first.split_at(LINE - 4);
    let joined = format!("{moved}{}", lines.get(2).copied().unwrap_or(""));
    lines[1] = keep;
    lines[2] = &joined;
    let text = format!("{}\n", lines.join("\n"));
    assert_eq!(
        read(text.as_bytes()),
        Err(EncodingError::LineLength {
            characters: LINE - 4,
            wrap: LINE,
        })
    );
}

#[test]
fn a_block_with_no_body_is_refused() {
    let text = b"-----BEGIN CERTIFICATE-----\n-----END CERTIFICATE-----\n";
    assert_eq!(read(text), Err(EncodingError::EmptyPayload));
    let mut out = [0u8; 8];
    assert_eq!(
        encode("CERTIFICATE", &[], &mut out),
        Err(EncodingError::EmptyPayload)
    );
}

#[test]
fn a_pad_before_the_last_body_line_is_refused() {
    let rendered = String::from_utf8(write_block("CERTIFICATE", &payload())).expect("ascii");
    let mut lines: Vec<String> = rendered.lines().map(str::to_owned).collect();
    let first = lines.get_mut(1).expect("a first body line");
    first.replace_range(LINE - 1.., "=");
    let text = format!("{}\n", lines.join("\n"));
    assert_eq!(read(text.as_bytes()), Err(EncodingError::Padding));
}

#[test]
fn a_label_the_rfc_does_not_allow_is_refused() {
    let mut out = [0u8; 128];
    for label in [" CERTIFICATE", "CERTIFICATE ", "TWO  SPACES", "WITH-HYPHEN"] {
        assert_eq!(
            encode(label, &[1, 2, 3], &mut out),
            Err(EncodingError::Label),
            "{label:?}"
        );
    }
    assert!(encode("PRIVATE KEY", &[1, 2, 3], &mut out).is_ok());
    assert!(encode("", &[1, 2, 3], &mut out).is_ok());
    assert_eq!(
        read(b"-----BEGIN A\x7fB-----\nAQID\n-----END A\x7fB-----\n"),
        Err(EncodingError::Label)
    );
}

#[test]
fn carriage_returns_are_accepted_as_part_of_the_terminator() {
    let bytes = payload();
    let rendered = String::from_utf8(write_block("CERTIFICATE", &bytes)).expect("ascii");
    let crlf = rendered.replace('\n', "\r\n");
    assert_eq!(read(crlf.as_bytes()), Ok(("CERTIFICATE".to_owned(), bytes)));
}

#[test]
fn a_buffer_that_is_too_small_is_an_error() {
    let bytes = payload();
    let text = write_block("CERTIFICATE", &bytes);
    let mut out = [0u8; 8];
    assert_eq!(decode(&text, &mut out), Err(EncodingError::BufferTooSmall));
    let mut small = vec![0u8; text.len() - 1];
    assert_eq!(
        encode("CERTIFICATE", &bytes, &mut small),
        Err(EncodingError::BufferTooSmall)
    );
    assert_eq!(encoded_len("CERTIFICATE", usize::MAX), None);
}

#[test]
fn a_block_is_what_it_says_it_is() {
    let mut out = [0u8; 16];
    let block = decode(b"-----BEGIN X-----\nAQID\n-----END X-----\n", &mut out)
        .expect("a well formed block");
    assert_eq!(
        block,
        Block {
            label: "X",
            bytes: &[1, 2, 3]
        }
    );
    assert!(!format!("{block:?}").is_empty());
}

#[test]
fn no_text_makes_the_parser_panic_and_what_it_accepts_re_encodes() {
    check("pem never panics", &any_pem_bytes(), |text| {
        let mut out = vec![0u8; text.len().max(1)];
        let Ok(block) = decode(text, &mut out) else {
            return Ok(());
        };
        let canonical = write_block(block.label, block.bytes);
        let mut again = vec![0u8; canonical.len()];
        let second = decode(&canonical, &mut again).map_err(|error| {
            format!("the canonical form of an accepted block was refused: {error}")
        })?;
        if second.label == block.label && second.bytes == block.bytes {
            Ok(())
        } else {
            Err(format!("{block:?} re-encoded to something else"))
        }
    });
}

/// The width `openssh-key-v1` is written at, which is not a multiple of
/// four and is therefore the case the wrapped pair exists for.
fn seventy() -> NonZeroUsize {
    NonZeroUsize::new(70).expect("70 is not zero")
}

/// Writes a block at `wrap` into a buffer of exactly the right size.
fn write_wrapped(label: &str, payload: &[u8], wrap: NonZeroUsize) -> Vec<u8> {
    let needed = encoded_len_wrapped(label, payload.len(), wrap).expect("a length that fits");
    let mut text = vec![0u8; needed];
    let written = encode_wrapped(label, payload, wrap, &mut text).expect("a buffer of that size");
    assert_eq!(written, needed);
    text
}

#[test]
fn a_body_wrapped_at_seventy_reads_back_what_it_was_written_with() {
    let bytes = payload();
    let text = write_wrapped("OPENSSH PRIVATE KEY", &bytes, seventy());
    let rendered = String::from_utf8(text.clone()).expect("ascii");
    let body: Vec<&str> = rendered
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect();
    let (last, full) = body.split_last().expect("a body");
    assert!(!full.is_empty(), "this payload needs more than one line");
    for line in full {
        assert_eq!(line.len(), 70, "{line:?}");
    }
    assert!(!last.is_empty() && last.len() <= 70, "{last:?}");

    let mut out = vec![0u8; text.len()];
    let block = decode_wrapped(&text, &mut out, seventy()).expect("a block");
    assert_eq!(
        block,
        Block {
            label: "OPENSSH PRIVATE KEY",
            bytes: &bytes
        }
    );
}

#[test]
fn a_text_wrapped_at_seventy_is_not_rfc_7468() {
    let text = write_wrapped("OPENSSH PRIVATE KEY", &payload(), seventy());
    assert_eq!(
        read(&text),
        Err(EncodingError::LineLength {
            characters: 70,
            wrap: LINE,
        })
    );
}

#[test]
fn a_wrapped_reader_takes_a_text_wrapped_more_narrowly() {
    let bytes = payload();
    let text = write_block("CERTIFICATE", &bytes);
    let mut out = vec![0u8; text.len()];
    let block = decode_wrapped(&text, &mut out, seventy()).expect("a block");
    assert_eq!(block.bytes, bytes);

    // Longer than the width is still refused, and so is an empty line.
    let wide = write_wrapped("CERTIFICATE", &bytes, seventy());
    let mut narrow = vec![0u8; wide.len()];
    assert_eq!(
        decode_wrapped(
            &wide,
            &mut narrow,
            NonZeroUsize::new(60).expect("60 is not zero")
        ),
        Err(EncodingError::LineLength {
            characters: 70,
            wrap: 60,
        })
    );
    let empty = b"-----BEGIN X-----

AQID
-----END X-----
";
    let mut out = [0u8; 16];
    assert_eq!(
        decode_wrapped(empty, &mut out, seventy()),
        Err(EncodingError::LineLength {
            characters: 0,
            wrap: 70,
        })
    );
}

#[test]
fn a_pad_before_the_last_line_of_a_wrapped_text_is_refused() {
    let rendered = String::from_utf8(write_wrapped("X", &payload(), seventy())).expect("ascii");
    let mut lines: Vec<String> = rendered.lines().map(str::to_owned).collect();
    let first = lines.get_mut(1).expect("a first body line");
    first.replace_range(69.., "=");
    let text = format!("{}\n", lines.join("\n"));
    let mut out = vec![0u8; text.len()];
    assert_eq!(
        decode_wrapped(text.as_bytes(), &mut out, seventy()),
        Err(EncodingError::Padding)
    );
}

#[test]
fn a_body_whose_characters_do_not_fill_a_quantum_is_refused() {
    let mut out = [0u8; 16];
    assert_eq!(
        decode_wrapped(
            b"-----BEGIN X-----\nAQI\n-----END X-----\n",
            &mut out,
            seventy()
        ),
        Err(EncodingError::Length(3))
    );
}

#[test]
fn a_character_error_in_a_wrapped_body_names_its_offset_in_the_body() {
    let mut text = write_wrapped("X", &payload(), seventy());
    let body_at = b"-----BEGIN X-----\n".len();
    let at = body_at + 71 + 5;
    *text.get_mut(at).expect("a body character") = b' ';
    let mut out = vec![0u8; text.len()];
    assert_eq!(
        decode_wrapped(&text, &mut out, seventy()),
        Err(EncodingError::Character(75))
    );
}

#[test]
fn a_wrapped_buffer_that_is_too_small_is_an_error_and_writes_nothing() {
    let bytes = payload();
    let needed = encoded_len_wrapped("X", bytes.len(), seventy()).expect("a length");
    let mut small = vec![0u8; needed - 1];
    assert_eq!(
        encode_wrapped("X", &bytes, seventy(), &mut small),
        Err(EncodingError::BufferTooSmall)
    );
    assert!(small.iter().all(|byte| *byte == 0));
    assert_eq!(encoded_len_wrapped("X", usize::MAX, seventy()), None);
}
