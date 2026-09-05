// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The root system description pointer, which the firmware leaves behind
//! and the loader passes on.
//!
//! Invariants: a [`Rsdp`] this module hands out has the signature, the
//! checksum, and, from revision two on, the extended checksum of the
//! specification; the address it names fits the physical address width.

use kernel_types::PhysAddr;

use crate::error::AcpiError;
use crate::raw::sum_of;

/// Number of bytes of the revision two root pointer, which is the largest
/// one the specification defines and the length this parser reads.
pub const RSDP_LEN: usize = 36;

/// Number of bytes the first checksum covers, which is also the length of
/// the revision zero structure.
pub const RSDP_V1_LEN: usize = 20;

/// The eight bytes every root pointer starts with.
pub const RSDP_SIGNATURE: [u8; 8] = *b"RSD PTR ";

/// The revision of ACPI 1.0, which names an RSDT and nothing else.
pub const REVISION_V1: u8 = 0;

/// The revision of ACPI 2.0 and later, which adds the XSDT.
pub const REVISION_V2: u8 = 2;

/// Offset of the extended checksum byte, which the announced length of a
/// revision two pointer has to cover.
const EXTENDED_CHECKSUM_OFFSET: usize = 32;

/// What the firmware left behind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rsdp {
    /// The revision the pointer announces: zero or two.
    pub revision: u8,
    /// The address of the root system description table, whose entries are
    /// four bytes wide.
    pub rsdt: PhysAddr,
    /// The address of the extended root system description table, whose
    /// entries are eight bytes wide. Only a revision two pointer names one.
    pub xsdt: Option<PhysAddr>,
}

impl Rsdp {
    /// The root table the kernel reads: the XSDT where the firmware offers
    /// one, the RSDT otherwise.
    #[must_use]
    pub const fn root(&self) -> PhysAddr {
        match self.xsdt {
            Some(xsdt) => xsdt,
            None => self.rsdt,
        }
    }
}

/// Reads the root pointer out of the bytes the firmware wrote.
///
/// The first checksum covers the first [`RSDP_V1_LEN`] bytes. A revision
/// two pointer adds a length and a second checksum over that many bytes;
/// because the structure the specification defines is exactly [`RSDP_LEN`]
/// bytes long, a length that does not reach the extended checksum byte or
/// that leaves the structure describes something this kernel cannot check
/// and is rejected.
///
/// # Errors
///
/// [`AcpiError::RootPointerSignature`], [`AcpiError::RootPointerChecksum`],
/// [`AcpiError::Revision`], [`AcpiError::RootPointerLength`],
/// [`AcpiError::ExtendedChecksum`], or [`AcpiError::Address`].
pub fn parse_rsdp(bytes: &[u8; RSDP_LEN]) -> Result<Rsdp, AcpiError> {
    // The layout of the structure, field by field, so that no read of it
    // can be out of bounds and no offset constant can drift.
    let [
        sig0,
        sig1,
        sig2,
        sig3,
        sig4,
        sig5,
        sig6,
        sig7,
        _checksum,
        _oem0,
        _oem1,
        _oem2,
        _oem3,
        _oem4,
        _oem5,
        revision,
        rsdt0,
        rsdt1,
        rsdt2,
        rsdt3,
        len0,
        len1,
        len2,
        len3,
        xsdt0,
        xsdt1,
        xsdt2,
        xsdt3,
        xsdt4,
        xsdt5,
        xsdt6,
        xsdt7,
        ..,
    ] = *bytes;
    if [sig0, sig1, sig2, sig3, sig4, sig5, sig6, sig7] != RSDP_SIGNATURE {
        return Err(AcpiError::RootPointerSignature);
    }
    if sum_of(bytes, RSDP_V1_LEN) != 0 {
        return Err(AcpiError::RootPointerChecksum);
    }
    let rsdt = address(u64::from(u32::from_le_bytes([rsdt0, rsdt1, rsdt2, rsdt3])))?;
    let xsdt = match revision {
        REVISION_V1 => None,
        REVISION_V2 => {
            check_extended(bytes, u32::from_le_bytes([len0, len1, len2, len3]))?;
            Some(address(u64::from_le_bytes([
                xsdt0, xsdt1, xsdt2, xsdt3, xsdt4, xsdt5, xsdt6, xsdt7,
            ]))?)
        }
        other => return Err(AcpiError::Revision(other)),
    };
    Ok(Rsdp {
        revision,
        rsdt,
        xsdt,
    })
}

/// Checks the announced length and the second checksum of a revision two
/// pointer.
fn check_extended(bytes: &[u8; RSDP_LEN], announced: u32) -> Result<(), AcpiError> {
    let length = usize::try_from(announced).unwrap_or(usize::MAX);
    if length <= EXTENDED_CHECKSUM_OFFSET || length > RSDP_LEN {
        return Err(AcpiError::RootPointerLength(announced));
    }
    if sum_of(bytes, length) != 0 {
        return Err(AcpiError::ExtendedChecksum);
    }
    Ok(())
}

/// The address a field names, if it is one the machine can have.
fn address(raw: u64) -> Result<PhysAddr, AcpiError> {
    PhysAddr::new(raw).map_err(|_| AcpiError::Address(raw))
}
