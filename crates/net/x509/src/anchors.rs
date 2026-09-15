// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The table of trust anchors an image carries (D-42, D-148).
//!
//! An anchor is a certificate whose subject and key the system trusts
//! without asking who signed it. [`TrustAnchor::from_certificate`] reads
//! those two fields out of one certificate; this module is how several of
//! them travel as one file: the tool that writes the image calls
//! [`write()`], and the program that reads the file calls [`Anchors::parse`]
//! and then [`Anchors::read_into`].
//!
//! The format, all integers little-endian, is
//!
//! | Bytes | Field |
//! |-------|-------|
//! | 0..4 | [`MAGIC`] |
//! | 4..6 | the version, [`VERSION`] |
//! | 6..8 | the number of records |
//! | 8.. | that many records, each a `u32` length and that many bytes of DER |
//!
//! Neither a record of no bytes nor a byte after the last record is
//! accepted, so a truncated file and a file with something appended are
//! two refusals and not one table with a surprise in it.

use crate::error::X509Error;
use crate::path::TrustAnchor;

/// The first four bytes of a table.
pub const MAGIC: [u8; 4] = *b"ANCH";

/// The version this crate writes and reads.
pub const VERSION: u16 = 1;

/// Bytes before the first record.
pub const HEADER_LEN: usize = 8;

/// Bytes of the length that precedes each record.
const LENGTH_LEN: usize = 4;

/// The anchors of one table, as the bytes they were written as.
#[derive(Clone, Copy, Debug)]
pub struct Anchors<'a> {
    /// The records, with no header before them.
    records: &'a [u8],
    /// How many records there are.
    count: usize,
}

impl<'a> Anchors<'a> {
    /// The table `bytes` holds.
    ///
    /// # Errors
    ///
    /// [`X509Error::BadAnchorTable`] for a header this crate did not
    /// write, a length that reaches past the end, a record of no bytes,
    /// and a byte behind the last record.
    pub fn parse(bytes: &'a [u8]) -> Result<Anchors<'a>, X509Error> {
        let header = bytes.get(..HEADER_LEN).ok_or(X509Error::BadAnchorTable)?;
        if header.get(..4) != Some(&MAGIC) {
            return Err(X509Error::BadAnchorTable);
        }
        if read_u16(header, 4)? != VERSION {
            return Err(X509Error::BadAnchorTable);
        }
        let count = usize::from(read_u16(header, 6)?);
        let records = bytes.get(HEADER_LEN..).ok_or(X509Error::BadAnchorTable)?;

        // The records are walked here rather than at every read, so that a
        // caller that parsed a table holds one whose every length is known
        // to fit.
        let mut rest = records;
        for _ in 0..count {
            let (_, tail) = split_record(rest)?;
            rest = tail;
        }
        if !rest.is_empty() {
            return Err(X509Error::BadAnchorTable);
        }
        Ok(Anchors { records, count })
    }

    /// How many anchors the table holds.
    #[must_use]
    pub const fn len(self) -> usize {
        self.count
    }

    /// Whether the table holds none.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.count == 0
    }

    /// The certificates, in the order they were written.
    #[must_use]
    pub const fn certificates(self) -> Certificates<'a> {
        Certificates {
            rest: self.records,
            left: self.count,
        }
    }

    /// Reads every anchor into `out` and answers how many there are.
    ///
    /// # Errors
    ///
    /// [`X509Error::BufferTooSmall`] when `out` is shorter than the table,
    /// and whatever [`TrustAnchor::from_certificate`] answers for a record
    /// that is no certificate this system can use.
    pub fn read_into(self, out: &mut [TrustAnchor<'a>]) -> Result<usize, X509Error> {
        if out.len() < self.count {
            return Err(X509Error::BufferTooSmall);
        }
        let mut filled = 0usize;
        for certificate in self.certificates() {
            let anchor = TrustAnchor::from_certificate(certificate)?;
            *out.get_mut(filled).ok_or(X509Error::BufferTooSmall)? = anchor;
            filled = filled.saturating_add(1);
        }
        Ok(filled)
    }
}

/// The certificates of a table, one record at a time.
#[derive(Clone, Debug)]
pub struct Certificates<'a> {
    /// What is left of the records.
    rest: &'a [u8],
    /// How many records are left.
    left: usize,
}

impl<'a> Iterator for Certificates<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        if self.left == 0 {
            return None;
        }
        // `Anchors::parse` walked every record, so this split succeeds.
        let (body, tail) = split_record(self.rest).ok()?;
        self.rest = tail;
        self.left = self.left.saturating_sub(1);
        Some(body)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.left, Some(self.left))
    }
}

impl ExactSizeIterator for Certificates<'_> {}

/// How many bytes the table of `certificates` takes.
///
/// # Errors
///
/// [`X509Error::BadAnchorTable`] for more certificates than a `u16` counts,
/// one of no bytes, and one longer than a `u32` measures.
pub fn table_len(certificates: &[&[u8]]) -> Result<usize, X509Error> {
    if u16::try_from(certificates.len()).is_err() {
        return Err(X509Error::BadAnchorTable);
    }
    let mut len = HEADER_LEN;
    for certificate in certificates {
        if certificate.is_empty() || u32::try_from(certificate.len()).is_err() {
            return Err(X509Error::BadAnchorTable);
        }
        len = len
            .checked_add(LENGTH_LEN)
            .and_then(|len| len.checked_add(certificate.len()))
            .ok_or(X509Error::BadAnchorTable)?;
    }
    Ok(len)
}

/// Writes the table of `certificates` into `out` and answers how many
/// bytes it takes.
///
/// # Errors
///
/// [`X509Error::BufferTooSmall`] when `out` is shorter than
/// [`table_len`], and what [`table_len`] answers for a list it refuses.
pub fn write(certificates: &[&[u8]], out: &mut [u8]) -> Result<usize, X509Error> {
    let len = table_len(certificates)?;
    let table = out.get_mut(..len).ok_or(X509Error::BufferTooSmall)?;
    let count = u16::try_from(certificates.len()).map_err(|_| X509Error::BadAnchorTable)?;

    put(table, 0, &MAGIC)?;
    put(table, 4, &VERSION.to_le_bytes())?;
    put(table, 6, &count.to_le_bytes())?;

    let mut at = HEADER_LEN;
    for certificate in certificates {
        let length = u32::try_from(certificate.len()).map_err(|_| X509Error::BadAnchorTable)?;
        put(table, at, &length.to_le_bytes())?;
        at = at.saturating_add(LENGTH_LEN);
        put(table, at, certificate)?;
        at = at.saturating_add(certificate.len());
    }
    Ok(len)
}

/// The first record of `rest` and what follows it.
fn split_record(rest: &[u8]) -> Result<(&[u8], &[u8]), X509Error> {
    let length = rest.get(..LENGTH_LEN).ok_or(X509Error::BadAnchorTable)?;
    let mut bytes = [0u8; LENGTH_LEN];
    bytes.copy_from_slice(length);
    let len = usize::try_from(u32::from_le_bytes(bytes)).map_err(|_| X509Error::BadAnchorTable)?;
    if len == 0 {
        return Err(X509Error::BadAnchorTable);
    }
    let body = rest
        .get(LENGTH_LEN..LENGTH_LEN.saturating_add(len))
        .ok_or(X509Error::BadAnchorTable)?;
    let tail = rest
        .get(LENGTH_LEN.saturating_add(len)..)
        .ok_or(X509Error::BadAnchorTable)?;
    Ok((body, tail))
}

/// The `u16` at `at` of `header`.
fn read_u16(header: &[u8], at: usize) -> Result<u16, X509Error> {
    let bytes = header
        .get(at..at.saturating_add(2))
        .ok_or(X509Error::BadAnchorTable)?;
    let mut value = [0u8; 2];
    value.copy_from_slice(bytes);
    Ok(u16::from_le_bytes(value))
}

/// Writes `bytes` at `at`.
fn put(table: &mut [u8], at: usize, bytes: &[u8]) -> Result<(), X509Error> {
    let room = table
        .get_mut(at..at.saturating_add(bytes.len()))
        .ok_or(X509Error::BufferTooSmall)?;
    room.copy_from_slice(bytes);
    Ok(())
}
