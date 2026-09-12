// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The error type: every variant says what it means.

use crate::error::EncodingError;

#[test]
fn every_variant_renders_a_message() {
    for (error, text) in [
        (
            EncodingError::BufferTooSmall,
            "the output buffer is too small",
        ),
        (
            EncodingError::Length(3),
            "no text of this encoding is 3 bytes long",
        ),
        (
            EncodingError::Character(7),
            "the byte at offset 7 is not in the alphabet",
        ),
        (
            EncodingError::Padding,
            "the padding is not one or two `=` at the end",
        ),
        (
            EncodingError::TrailingBits,
            "the last quantum carries bits its length declares unused",
        ),
        (EncodingError::MissingBegin, "there is no begin line"),
        (EncodingError::MissingEnd, "there is no end line"),
        (
            EncodingError::LabelMismatch,
            "the end line names a different label than the begin line",
        ),
        (EncodingError::Label, "the label is not one RFC 7468 allows"),
        (
            EncodingError::LineLength {
                characters: 60,
                wrap: 64,
            },
            "a body line is 60 characters, not the 64 this text wraps at",
        ),
        (EncodingError::EmptyPayload, "the block carries no bytes"),
        (
            EncodingError::TrailingData,
            "something follows the end line",
        ),
    ] {
        assert_eq!(error.to_string(), text);
    }
}

#[test]
fn two_errors_of_the_same_kind_and_value_are_equal() {
    assert_eq!(EncodingError::Character(3), EncodingError::Character(3));
    assert_ne!(EncodingError::Character(3), EncodingError::Character(4));
    assert_ne!(EncodingError::Character(3), EncodingError::Length(3));
}
