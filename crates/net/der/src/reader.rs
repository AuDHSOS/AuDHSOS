// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The reader itself.
//!
//! Invariants: the reader holds the bytes it has not consumed and the
//! depth it sits at; every typed read consumes one complete value or
//! leaves the reader untouched and returns an error; a value is a slice
//! into the input, so nothing is copied and the borrow keeps the result
//! tied to the bytes it came from.

use crate::error::DerError;
use crate::tag::Tag;
use audhsos_time::CivilTime;

/// How deep a value may nest. A certificate reaches about six.
pub const MAX_DEPTH: usize = 16;

/// The longest length the reader accepts in the long form: four bytes,
/// which is more than any certificate needs and less than an address.
const MAX_LENGTH_BYTES: usize = 4;

/// A bit string: its content and how many bits of the last byte are unused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BitString<'a> {
    /// The bytes, including the partial last one.
    pub bytes: &'a [u8],
    /// How many bits of the last byte are not part of the value.
    pub unused: u8,
}

impl<'a> BitString<'a> {
    /// The bytes, if the string ends on a byte boundary.
    ///
    /// # Errors
    ///
    /// [`DerError::BadBitString`] when bits are unused. Every bit string a
    /// certificate carries in a place this system reads is a whole number
    /// of bytes.
    pub const fn whole_bytes(self) -> Result<&'a [u8], DerError> {
        if self.unused == 0 {
            Ok(self.bytes)
        } else {
            Err(DerError::BadBitString)
        }
    }
}

/// An object identifier, compared as the bytes it is encoded as.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Oid<'a>(&'a [u8]);

impl<'a> Oid<'a> {
    /// The encoded components, without the tag and the length.
    #[must_use]
    pub const fn as_bytes(self) -> &'a [u8] {
        self.0
    }
}

/// A reader over one value or over the content of a constructed one.
#[derive(Clone, Debug)]
pub struct Reader<'a> {
    /// What has not been consumed.
    bytes: &'a [u8],
    /// How deep this reader sits.
    depth: usize,
}

impl<'a> Reader<'a> {
    /// A reader over `bytes`.
    #[must_use]
    pub const fn new(bytes: &'a [u8]) -> Reader<'a> {
        Reader { bytes, depth: 0 }
    }

    /// Whether everything has been consumed.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// What has not been consumed, for a caller that needs the remaining
    /// bytes verbatim, such as one hashing a signed body.
    #[must_use]
    pub const fn rest(&self) -> &'a [u8] {
        self.bytes
    }

    /// Consumes the reader, rejecting anything left.
    ///
    /// # Errors
    ///
    /// [`DerError::TrailingData`] when bytes remain.
    pub const fn finish(self) -> Result<(), DerError> {
        if self.bytes.is_empty() {
            Ok(())
        } else {
            Err(DerError::TrailingData)
        }
    }

    /// The tag of the next value, without consuming it.
    #[must_use]
    pub fn peek(&self) -> Option<Tag> {
        self.bytes.first().copied().map(Tag::new)
    }

    /// Whether the next value carries `tag`.
    #[must_use]
    pub fn peek_is(&self, tag: Tag) -> bool {
        self.peek() == Some(tag)
    }

    /// The next value: its tag and its content.
    ///
    /// # Errors
    ///
    /// Any of the encoding rules the header must follow; see [`DerError`].
    pub fn read_any(&mut self) -> Result<(Tag, &'a [u8]), DerError> {
        let (&identifier, after_tag) = self.bytes.split_first().ok_or(DerError::EndOfInput)?;
        let tag = Tag::new(identifier);
        if tag.is_high_form() {
            return Err(DerError::HighTagNumber);
        }

        let (length, after_length) = read_length(after_tag)?;
        let (content, rest) = after_length
            .split_at_checked(length)
            .ok_or(DerError::LengthOutOfRange)?;
        self.bytes = rest;
        Ok((tag, content))
    }

    /// The content of the next value, which must carry `tag`.
    ///
    /// # Errors
    ///
    /// [`DerError::UnexpectedTag`] when the tag differs, plus the header
    /// errors of [`Reader::read_any`].
    pub fn read_tagged(&mut self, tag: Tag) -> Result<&'a [u8], DerError> {
        let mut lookahead = self.clone();
        let (found, content) = lookahead.read_any()?;
        if found != tag {
            return Err(DerError::UnexpectedTag);
        }
        self.bytes = lookahead.bytes;
        Ok(content)
    }

    /// A reader over the content of the next constructed value, which must
    /// carry `tag`.
    ///
    /// # Errors
    ///
    /// [`DerError::WrongForm`] when the tag is not constructed,
    /// [`DerError::TooDeep`] at the nesting bound, plus the errors of
    /// [`Reader::read_tagged`].
    pub fn read_constructed(&mut self, tag: Tag) -> Result<Reader<'a>, DerError> {
        if !tag.is_constructed() {
            return Err(DerError::WrongForm);
        }
        let depth = self.depth.wrapping_add(1);
        if depth >= MAX_DEPTH {
            return Err(DerError::TooDeep);
        }
        let content = self.read_tagged(tag)?;
        Ok(Reader {
            bytes: content,
            depth,
        })
    }

    /// A reader over the content of the next `SEQUENCE`.
    ///
    /// # Errors
    ///
    /// See [`Reader::read_constructed`].
    pub fn read_sequence(&mut self) -> Result<Reader<'a>, DerError> {
        self.read_constructed(Tag::SEQUENCE)
    }

    /// A reader over the content of the next `SET`.
    ///
    /// # Errors
    ///
    /// See [`Reader::read_constructed`].
    pub fn read_set(&mut self) -> Result<Reader<'a>, DerError> {
        self.read_constructed(Tag::SET)
    }

    /// The content of the next explicitly tagged context value.
    ///
    /// # Errors
    ///
    /// See [`Reader::read_constructed`].
    pub fn read_context(&mut self, number: u8) -> Result<Reader<'a>, DerError> {
        self.read_constructed(Tag::context(number, true))
    }

    /// The next value when it carries `tag`, and nothing when it does not.
    ///
    /// # Errors
    ///
    /// The header errors of [`Reader::read_any`], when a value with that
    /// tag is present but malformed.
    pub fn read_optional(&mut self, tag: Tag) -> Result<Option<&'a [u8]>, DerError> {
        if self.peek_is(tag) {
            self.read_tagged(tag).map(Some)
        } else {
            Ok(None)
        }
    }

    /// The next `INTEGER`, as the bytes of a non-negative value without a
    /// padding zero.
    ///
    /// Negative integers are refused: nothing this system reads uses one,
    /// and accepting them would mean carrying a sign through every caller.
    ///
    /// # Errors
    ///
    /// [`DerError::BadInteger`] when the value is empty, negative, or not
    /// in the shortest form.
    pub fn read_integer(&mut self) -> Result<&'a [u8], DerError> {
        let content = self.read_tagged(Tag::INTEGER)?;
        let (&first, rest) = content.split_first().ok_or(DerError::BadInteger)?;
        if first & 0x80 != 0 {
            return Err(DerError::BadInteger);
        }
        match rest.first() {
            // A leading zero is allowed only to keep a value non-negative.
            Some(&second) if first == 0x00 && second & 0x80 == 0 => Err(DerError::BadInteger),
            Some(_) if first == 0x00 => Ok(rest),
            None if first == 0x00 => Ok(content),
            _ => Ok(content),
        }
    }

    /// The next `BOOLEAN`.
    ///
    /// # Errors
    ///
    /// [`DerError::BadBoolean`] unless the content is one byte of zero or
    /// of all ones, which is the only encoding the distinguished rules
    /// allow.
    pub fn read_boolean(&mut self) -> Result<bool, DerError> {
        let content = self.read_tagged(Tag::BOOLEAN)?;
        match content {
            [0x00] => Ok(false),
            [0xFF] => Ok(true),
            _ => Err(DerError::BadBoolean),
        }
    }

    /// The next `NULL`.
    ///
    /// # Errors
    ///
    /// [`DerError::BadNull`] when the value carries content.
    pub fn read_null(&mut self) -> Result<(), DerError> {
        let content = self.read_tagged(Tag::NULL)?;
        if content.is_empty() {
            Ok(())
        } else {
            Err(DerError::BadNull)
        }
    }

    /// The next `OCTET STRING`.
    ///
    /// # Errors
    ///
    /// The header errors of [`Reader::read_any`].
    pub fn read_octet_string(&mut self) -> Result<&'a [u8], DerError> {
        self.read_tagged(Tag::OCTET_STRING)
    }

    /// The next `BIT STRING`.
    ///
    /// # Errors
    ///
    /// [`DerError::BadBitString`] when the count of unused bits is absent,
    /// above seven, non-zero for an empty string, or when the unused bits
    /// themselves are not zero.
    pub fn read_bit_string(&mut self) -> Result<BitString<'a>, DerError> {
        let content = self.read_tagged(Tag::BIT_STRING)?;
        let (&unused, bytes) = content.split_first().ok_or(DerError::BadBitString)?;
        if unused > 7 {
            return Err(DerError::BadBitString);
        }
        match bytes.last() {
            None if unused != 0 => Err(DerError::BadBitString),
            Some(&last) if last & mask_of(unused) != 0 => Err(DerError::BadBitString),
            _ => Ok(BitString { bytes, unused }),
        }
    }

    /// The next `OBJECT IDENTIFIER`.
    ///
    /// The components are not decoded. An identifier is compared against a
    /// constant, and comparing the encodings answers that question without
    /// a decoder that could disagree with the encoder.
    ///
    /// # Errors
    ///
    /// [`DerError::BadObjectIdentifier`] when the value is empty, ends in
    /// a continuation byte, or encodes a component with a leading zero.
    pub fn read_object_identifier(&mut self) -> Result<Oid<'a>, DerError> {
        let content = self.read_tagged(Tag::OBJECT_IDENTIFIER)?;
        let (&last, _) = content.split_last().ok_or(DerError::BadObjectIdentifier)?;
        if last & 0x80 != 0 {
            return Err(DerError::BadObjectIdentifier);
        }
        let mut starting = true;
        for &byte in content {
            if starting && byte == 0x80 {
                return Err(DerError::BadObjectIdentifier);
            }
            starting = byte & 0x80 == 0;
        }
        Ok(Oid(content))
    }

    /// The next time value, in either of the two forms a certificate uses.
    ///
    /// # Errors
    ///
    /// [`DerError::UnexpectedTag`] when the value is neither form, and
    /// [`DerError::BadTime`] when the form is right and the content is
    /// not.
    pub fn read_time(&mut self) -> Result<CivilTime, DerError> {
        match self.peek() {
            Some(Tag::UTC_TIME) => {
                let content = self.read_tagged(Tag::UTC_TIME)?;
                crate::time::from_utc_time(content)
            }
            Some(Tag::GENERALIZED_TIME) => {
                let content = self.read_tagged(Tag::GENERALIZED_TIME)?;
                crate::time::from_generalized_time(content)
            }
            Some(_) => Err(DerError::UnexpectedTag),
            None => Err(DerError::EndOfInput),
        }
    }
}

/// The mask of the bits a count of unused bits declares unused.
fn mask_of(unused: u8) -> u8 {
    match unused {
        0 => 0x00,
        _ => 0xFFu8.wrapping_shr(u32::from(8u8.wrapping_sub(unused))),
    }
}

/// The length at the front of `bytes`, and what follows it.
fn read_length(bytes: &[u8]) -> Result<(usize, &[u8]), DerError> {
    let (&first, rest) = bytes.split_first().ok_or(DerError::Truncated)?;
    if first & 0x80 == 0 {
        return Ok((usize::from(first), rest));
    }

    let count = usize::from(first & 0x7F);
    if count == 0 {
        return Err(DerError::IndefiniteLength);
    }
    if count > MAX_LENGTH_BYTES {
        return Err(DerError::LengthOutOfRange);
    }
    let (digits, after) = rest.split_at_checked(count).ok_or(DerError::Truncated)?;

    let (&leading, _) = digits.split_first().ok_or(DerError::Truncated)?;
    if leading == 0x00 {
        return Err(DerError::NonMinimalLength);
    }

    let mut length = 0usize;
    for &byte in digits {
        length = length.wrapping_shl(8) | usize::from(byte);
    }
    if length < 0x80 {
        // A value that fits the short form must use it.
        return Err(DerError::NonMinimalLength);
    }
    Ok((length, after))
}
