// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The header every system description table starts with, and the root
//! table that names the others.
//!
//! The layouts are the *ACPI Specification* 6.6: section 5.2.6 for the
//! header and the checksum over the whole table, sections 5.2.7 and 5.2.8
//! for the RSDT and the XSDT, whose entry arrays hold four- and eight-byte
//! pointers.
//!
//! Invariants: a [`SdtHeader`] this module hands out announces a length
//! that the bytes cover and that is at least the header itself, and the
//! bytes of that length sum to zero; a [`RootTable`] hands out only
//! addresses that fit the physical address width.

use kernel_types::PhysAddr;

use crate::error::AcpiError;
use crate::raw::{array_at, sum_of, u32_at, u64_at};

/// Number of bytes of the header every table starts with, from
/// section 5.2.6.
pub const SDT_HEADER_LEN: usize = 36;

/// The signature of the root table with four-byte entries,
/// from section 5.2.7.
pub const RSDT_SIGNATURE: [u8; 4] = *b"RSDT";

/// The signature of the root table with eight-byte entries,
/// from section 5.2.8.
pub const XSDT_SIGNATURE: [u8; 4] = *b"XSDT";

/// What the header of a table says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SdtHeader {
    /// The four bytes that name the table.
    pub signature: [u8; 4],
    /// Number of bytes of the whole table, header included.
    pub length: u32,
    /// The revision of the table's own layout.
    pub revision: u8,
}

impl SdtHeader {
    /// Reads and checks the header of the table `bytes` starts with.
    ///
    /// # Errors
    ///
    /// [`AcpiError::TooShort`] if the bytes do not even hold a header;
    /// [`AcpiError::Length`] if the announced length is below the header or
    /// beyond the bytes; [`AcpiError::Checksum`] if the bytes the length
    /// names do not sum to zero.
    pub fn parse(bytes: &[u8]) -> Result<SdtHeader, AcpiError> {
        let header: [u8; SDT_HEADER_LEN] =
            array_at(bytes, 0).ok_or(AcpiError::TooShort(bytes.len()))?;
        let [sig0, sig1, sig2, sig3, len0, len1, len2, len3, revision, ..] = header;
        let announced = u32::from_le_bytes([len0, len1, len2, len3]);
        let length = usize::try_from(announced).unwrap_or(usize::MAX);
        if length < SDT_HEADER_LEN || length > bytes.len() {
            return Err(AcpiError::Length(announced));
        }
        if sum_of(bytes, length) != 0 {
            return Err(AcpiError::Checksum);
        }
        Ok(SdtHeader {
            signature: [sig0, sig1, sig2, sig3],
            length: announced,
            revision,
        })
    }

    /// The header, if the table carries `signature`.
    ///
    /// # Errors
    ///
    /// [`AcpiError::Signature`] if it carries another one.
    pub const fn require(self, signature: [u8; 4]) -> Result<SdtHeader, AcpiError> {
        if matches_signature(self.signature, signature) {
            Ok(self)
        } else {
            Err(AcpiError::Signature(self.signature))
        }
    }

    /// Number of bytes the table holds after the header.
    #[must_use]
    #[expect(
        clippy::as_conversions,
        reason = "a `u32` fits a `usize` on every target of this workspace, and the function is `const`"
    )]
    pub const fn body_len(self) -> usize {
        (self.length as usize).saturating_sub(SDT_HEADER_LEN)
    }
}

/// The length a table announces, read out of its header and checked
/// against nothing. An adapter that copies a table out of memory has to
/// know how much to copy before it can hand the whole table to
/// [`SdtHeader::parse`], which is the one that checks.
#[must_use]
pub const fn announced_length(header: &[u8; SDT_HEADER_LEN]) -> u32 {
    let [_, _, _, _, len0, len1, len2, len3, ..] = *header;
    u32::from_le_bytes([len0, len1, len2, len3])
}

/// Whether two signatures are the same, as a `const fn`, because `[u8; 4]`
/// has no `const` comparison.
const fn matches_signature(left: [u8; 4], right: [u8; 4]) -> bool {
    let [l0, l1, l2, l3] = left;
    let [r0, r1, r2, r3] = right;
    l0 == r0 && l1 == r1 && l2 == r2 && l3 == r3
}

/// The table that names every other table, together with the bytes it was
/// read from.
#[derive(Clone, Copy, Debug)]
pub struct RootTable<'a> {
    bytes: &'a [u8],
    header: SdtHeader,
    width: usize,
    count: usize,
}

impl<'a> RootTable<'a> {
    /// Reads the root table `bytes` starts with. The signature decides how
    /// wide its entries are: `RSDT` names four-byte addresses, `XSDT`
    /// eight-byte ones.
    ///
    /// # Errors
    ///
    /// The errors of [`SdtHeader::parse`], and [`AcpiError::Signature`] for
    /// a table that is neither an RSDT nor an XSDT.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, AcpiError> {
        let header = SdtHeader::parse(bytes)?;
        let body = header.body_len();
        let (width, count) = if matches_signature(header.signature, RSDT_SIGNATURE) {
            (4, body >> 2)
        } else if matches_signature(header.signature, XSDT_SIGNATURE) {
            (8, body >> 3)
        } else {
            return Err(AcpiError::Signature(header.signature));
        };
        Ok(RootTable {
            bytes,
            header,
            width,
            count,
        })
    }

    /// The header of the root table.
    #[must_use]
    pub const fn header(&self) -> SdtHeader {
        self.header
    }

    /// Number of addresses the table names. A trailing partial entry is
    /// not one.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.count
    }

    /// `true` if the table names no other table.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The address in slot `index`, or `None` beyond the last slot or for a
    /// value that does not fit the physical address width.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<PhysAddr> {
        if index >= self.len() {
            return None;
        }
        let offset = SDT_HEADER_LEN.saturating_add(index.saturating_mul(self.width));
        let raw = if self.width == 8 {
            u64_at(self.bytes, offset)?
        } else {
            u64::from(u32_at(self.bytes, offset)?)
        };
        PhysAddr::new(raw).ok()
    }

    /// Every address the table names, in order, without the ones that do
    /// not fit the physical address width.
    pub fn addresses(&self) -> impl Iterator<Item = PhysAddr> + '_ {
        (0..self.len()).filter_map(|index| self.get(index))
    }
}
