// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The record layer: what a record looks like on the wire, and where its
//! boundaries are.
//!
//! The version field of a record is not checked. RFC 8446 section 5.1
//! says a receiver must ignore it, because middleboxes were found to
//! insist on values the standard had moved past; the version that matters
//! is negotiated in an extension.
//!
//! Invariant: a header is five bytes, the length it carries is the length
//! of what follows, and nothing in this module looks past that.

use crate::error::TlsError;

/// Bytes of record header.
pub const HEADER_LEN: usize = 5;
/// The longest plaintext a record may carry.
pub const MAX_PLAINTEXT: usize = 1 << 14;
/// The longest fragment a record may carry: the plaintext, the content
/// type that follows it, and what the cipher adds.
pub const MAX_CIPHERTEXT: usize = MAX_PLAINTEXT + 256;
/// The longest record, header included.
pub const MAX_RECORD: usize = HEADER_LEN + MAX_CIPHERTEXT;

/// The legacy version every record carries.
const LEGACY_VERSION: [u8; 2] = [0x03, 0x03];

/// What a record carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ContentType {
    /// A record that exists only so that a middlebox sees what it expects.
    ChangeCipherSpec,
    /// An alert.
    Alert,
    /// A handshake message, or part of one.
    Handshake,
    /// Application data, and everything encrypted, which is why the outer
    /// type of an encrypted record is always this one.
    ApplicationData,
}

impl ContentType {
    /// The type a byte names.
    ///
    /// # Errors
    ///
    /// [`TlsError::UnknownContentType`] for any other byte.
    pub const fn from_byte(byte: u8) -> Result<ContentType, TlsError> {
        match byte {
            20 => Ok(ContentType::ChangeCipherSpec),
            21 => Ok(ContentType::Alert),
            22 => Ok(ContentType::Handshake),
            23 => Ok(ContentType::ApplicationData),
            _ => Err(TlsError::UnknownContentType),
        }
    }

    /// The byte that names the type.
    #[must_use]
    pub const fn to_byte(self) -> u8 {
        match self {
            ContentType::ChangeCipherSpec => 20,
            ContentType::Alert => 21,
            ContentType::Handshake => 22,
            ContentType::ApplicationData => 23,
        }
    }
}

/// The header of a record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    /// What the record says it carries.
    pub content_type: ContentType,
    /// How many bytes follow the header.
    pub length: usize,
}

/// A record at the front of a buffer: its header, its body, and how many
/// bytes it occupied.
pub type Framed<'a> = (Header, &'a [u8], usize);

/// One record at the front of `bytes`. Nothing, when the record has not
/// arrived in full.
///
/// # Errors
///
/// [`TlsError::UnknownContentType`] for a type that is not one of the
/// four, and [`TlsError::RecordOverflow`] for a length beyond what the
/// protocol allows — both of which are decided before waiting for more.
pub fn read(bytes: &[u8]) -> Result<Option<Framed<'_>>, TlsError> {
    let Some((header, rest)) = bytes.split_at_checked(HEADER_LEN) else {
        return Ok(None);
    };
    let [first, _, _, high, low] = *header
        .first_chunk::<HEADER_LEN>()
        .unwrap_or(&[0; HEADER_LEN]);

    let content_type = ContentType::from_byte(first)?;
    let length = usize::from(u16::from_be_bytes([high, low]));
    if length > MAX_CIPHERTEXT {
        return Err(TlsError::RecordOverflow);
    }

    let Some((body, _)) = rest.split_at_checked(length) else {
        return Ok(None);
    };
    Ok(Some((
        Header {
            content_type,
            length,
        },
        body,
        HEADER_LEN.wrapping_add(length),
    )))
}

/// Writes a header for `length` bytes of `content_type` into `out`.
///
/// # Errors
///
/// [`TlsError::BufferTooSmall`] when `out` is shorter than a header, and
/// [`TlsError::RecordOverflow`] when the length is beyond what the
/// protocol allows.
pub fn write_header(
    content_type: ContentType,
    length: usize,
    out: &mut [u8],
) -> Result<(), TlsError> {
    if length > MAX_CIPHERTEXT {
        return Err(TlsError::RecordOverflow);
    }
    let header = out
        .first_chunk_mut::<HEADER_LEN>()
        .ok_or(TlsError::BufferTooSmall)?;
    let length = u16::try_from(length).map_err(|_| TlsError::RecordOverflow)?;
    *header = [
        content_type.to_byte(),
        LEGACY_VERSION[0],
        LEGACY_VERSION[1],
        length.to_be_bytes()[0],
        length.to_be_bytes()[1],
    ];
    Ok(())
}
