// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why a text was rejected, or why it could not be written.

use core::fmt;

/// What this crate found wrong with a text, or with the buffer it was
/// asked to write into.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EncodingError {
    /// The output buffer is shorter than the result. Nothing was written:
    /// a function of this crate either writes its whole answer or none of
    /// it.
    BufferTooSmall,
    /// The input has a length no text of this encoding can have: an odd
    /// number of hex digits, or a Base64 text that is not a multiple of
    /// four characters.
    Length(usize),
    /// A byte that is not in the alphabet of the encoding, at the given
    /// offset. Whitespace counts, because a strict decoder has no
    /// characters it may skip.
    Character(usize),
    /// The pad is missing, is longer than two characters, or stands
    /// anywhere but at the end.
    Padding,
    /// The last quantum of a Base64 text carries bits that its length says
    /// are unused. Accepting them would let two texts mean one sequence of
    /// bytes.
    TrailingBits,
    /// The text has no `-----BEGIN <label>-----` line.
    MissingBegin,
    /// The text has no `-----END <label>-----` line.
    MissingEnd,
    /// The end line names a different label than the begin line.
    LabelMismatch,
    /// A label carries a character RFC 7468 does not allow in one, or a
    /// space where it may not stand.
    Label,
    /// A body line other than the last is not exactly 64 characters.
    LineLength(usize),
    /// The block carries no bytes between its begin and end lines.
    EmptyPayload,
    /// Something follows the end line. Explanatory text is allowed before
    /// a block and not after it, so that appending to a file cannot go
    /// unnoticed.
    TrailingData,
}

impl fmt::Display for EncodingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EncodingError::BufferTooSmall => f.write_str("the output buffer is too small"),
            EncodingError::Length(length) => {
                write!(f, "no text of this encoding is {length} bytes long")
            }
            EncodingError::Character(at) => {
                write!(f, "the byte at offset {at} is not in the alphabet")
            }
            EncodingError::Padding => f.write_str("the padding is not one or two `=` at the end"),
            EncodingError::TrailingBits => {
                f.write_str("the last quantum carries bits its length declares unused")
            }
            EncodingError::MissingBegin => f.write_str("there is no begin line"),
            EncodingError::MissingEnd => f.write_str("there is no end line"),
            EncodingError::LabelMismatch => {
                f.write_str("the end line names a different label than the begin line")
            }
            EncodingError::Label => f.write_str("the label is not one RFC 7468 allows"),
            EncodingError::LineLength(length) => {
                write!(
                    f,
                    "a body line before the last is {length} characters, not 64"
                )
            }
            EncodingError::EmptyPayload => f.write_str("the block carries no bytes"),
            EncodingError::TrailingData => f.write_str("something follows the end line"),
        }
    }
}
