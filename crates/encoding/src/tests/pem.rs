// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! PEM against RFC 7468: what a block is, and everything that looks like
//! one and is not.

use test_support::property::check;

use crate::error::EncodingError;
use crate::pem::{Block, LINE, decode, encode, encoded_len};
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
        Err(EncodingError::LineLength(LINE - 4))
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
