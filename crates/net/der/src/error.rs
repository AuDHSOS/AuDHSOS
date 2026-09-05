// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why an encoding was rejected.

use core::fmt;

/// Why an encoding was rejected.
///
/// Every variant names one rule of the distinguished encoding rules or of
/// the profile RFC 5280 puts on top of them, so that a rejected
/// certificate can be described rather than merely refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DerError {
    /// The input ended inside a value.
    Truncated,
    /// A value was expected and the input was empty.
    EndOfInput,
    /// The tag was not the one the caller expected.
    UnexpectedTag,
    /// The tag used the high tag number form. Nothing in a certificate
    /// needs a tag above thirty.
    HighTagNumber,
    /// A primitive value carried the constructed bit, or a constructed one
    /// did not.
    WrongForm,
    /// The length used the indefinite form, which the distinguished
    /// encoding rules forbid.
    IndefiniteLength,
    /// The length was not encoded in the shortest form.
    NonMinimalLength,
    /// The length does not fit in an address, or reaches past the input.
    LengthOutOfRange,
    /// Bytes were left after the outermost value, or after the last value
    /// of a sequence the caller finished.
    TrailingData,
    /// The nesting is deeper than `MAX_DEPTH`.
    TooDeep,
    /// An integer was empty, negative, or not in the shortest form.
    BadInteger,
    /// A boolean was not one byte of zero or two hundred and fifty-five.
    BadBoolean,
    /// A bit string had no count of unused bits, a count above seven, or
    /// unused bits that were not zero.
    BadBitString,
    /// An object identifier was empty, ended in a continuation byte, or
    /// encoded a component with a leading zero byte.
    BadObjectIdentifier,
    /// A null value carried content.
    BadNull,
    /// A time was not in the form RFC 5280 requires, or a field was out of
    /// range.
    BadTime,
}

impl fmt::Display for DerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DerError::Truncated => f.write_str("the input ends inside a value"),
            DerError::EndOfInput => f.write_str("a value was expected and the input is empty"),
            DerError::UnexpectedTag => f.write_str("the tag is not the expected one"),
            DerError::HighTagNumber => f.write_str("the tag uses the high tag number form"),
            DerError::WrongForm => f.write_str("the constructed bit does not match the type"),
            DerError::IndefiniteLength => f.write_str("the length uses the indefinite form"),
            DerError::NonMinimalLength => f.write_str("the length is not in the shortest form"),
            DerError::LengthOutOfRange => f.write_str("the length reaches past the input"),
            DerError::TrailingData => f.write_str("bytes are left after the value"),
            DerError::TooDeep => f.write_str("the nesting is too deep"),
            DerError::BadInteger => f.write_str("the integer is empty, negative, or padded"),
            DerError::BadBoolean => f.write_str("the boolean is not zero or all ones"),
            DerError::BadBitString => f.write_str("the bit string has a bad count of unused bits"),
            DerError::BadObjectIdentifier => f.write_str("the object identifier is malformed"),
            DerError::BadNull => f.write_str("the null value carries content"),
            DerError::BadTime => f.write_str("the time is not in the required form"),
        }
    }
}
