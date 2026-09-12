// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why a table was rejected. The fields these variants name are the
//! *ACPI Specification* 6.6, sections 5.2.5.3, 5.2.6 and 5.2.12.

use core::fmt;

/// What a parser of this crate found wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AcpiError {
    /// The root pointer does not start with `RSD PTR `.
    RootPointerSignature,
    /// The sum of the first twenty bytes of the root pointer is not zero,
    /// which section 5.2.5.3 requires.
    RootPointerChecksum,
    /// The sum of the bytes the root pointer's length names is not zero.
    ExtendedChecksum,
    /// The root pointer names a revision this kernel does not read.
    Revision(u8),
    /// The length field of the root pointer does not describe the bytes
    /// that were handed in.
    RootPointerLength(u32),
    /// The bytes end where a table header or an entry header has to start.
    TooShort(usize),
    /// The table announces a length the bytes do not cover, or one below
    /// the header.
    Length(u32),
    /// The sum of the bytes the table's length names is not zero, which
    /// section 5.2.6 requires.
    Checksum,
    /// The table carries a signature the caller did not ask for.
    Signature([u8; 4]),
    /// An entry announces a length of zero, which would never end the walk.
    EntryLengthZero(usize),
    /// An entry announces a length that leaves the table.
    EntryTruncated {
        /// The entry type.
        kind: u8,
        /// The length the entry announced.
        length: u8,
    },
    /// An entry of a type this parser reads has the wrong length.
    EntryLength {
        /// The entry type.
        kind: u8,
        /// The length the entry announced.
        length: u8,
    },
    /// The table names more I/O APICs than this kernel holds.
    TooManyIoApics,
    /// The table names more interrupt source overrides than this kernel
    /// holds.
    TooManyOverrides,
    /// The table names more configuration windows than this kernel holds.
    TooManyAllocations,
    /// A configuration window ends at a bus below the one it starts at.
    BusRange {
        /// The first bus the allocation names.
        first_bus: u8,
        /// The last bus it names.
        last_bus: u8,
    },
    /// A configuration window starts inside a page, which the kernel maps
    /// whole frames of.
    Unaligned(u64),
    /// An address in the table does not fit the physical address width.
    Address(u64),
}

impl fmt::Display for AcpiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AcpiError::RootPointerSignature => {
                f.write_str("the root pointer does not start with `RSD PTR `")
            }
            AcpiError::RootPointerChecksum => f.write_str("the root pointer checksum is wrong"),
            AcpiError::ExtendedChecksum => {
                f.write_str("the extended root pointer checksum is wrong")
            }
            AcpiError::Revision(revision) => {
                write!(f, "acpi revision {revision} is not one this kernel reads")
            }
            AcpiError::RootPointerLength(length) => {
                write!(f, "the root pointer announces {length} bytes")
            }
            AcpiError::TooShort(at) => {
                write!(f, "the table ends at byte {at}, inside a header")
            }
            AcpiError::Length(length) => write!(f, "the table announces {length} bytes"),
            AcpiError::Checksum => f.write_str("the table checksum is wrong"),
            AcpiError::Signature(signature) => {
                f.write_str("the table signature is ")?;
                write_signature(f, *signature)
            }
            AcpiError::EntryLengthZero(offset) => {
                write!(f, "the entry at offset {offset} announces no length")
            }
            AcpiError::EntryTruncated { kind, length } => write!(
                f,
                "the entry of type {kind} announces {length} bytes, which leave the table"
            ),
            AcpiError::EntryLength { kind, length } => write!(
                f,
                "the entry of type {kind} announces {length} bytes, which is not its length"
            ),
            AcpiError::TooManyIoApics => f.write_str("the table names more i/o apics than fit"),
            AcpiError::TooManyOverrides => {
                f.write_str("the table names more interrupt source overrides than fit")
            }
            AcpiError::TooManyAllocations => {
                f.write_str("the table names more configuration windows than fit")
            }
            AcpiError::BusRange {
                first_bus,
                last_bus,
            } => write!(
                f,
                "the configuration window covers buses {first_bus} to {last_bus}, which run backwards"
            ),
            AcpiError::Unaligned(base) => write!(
                f,
                "the configuration window starts at {base:#x}, which is no page"
            ),
            AcpiError::Address(address) => write!(
                f,
                "the table names {address:#x}, which exceeds the physical address width"
            ),
        }
    }
}

/// Writes a four-byte signature as text, with a dot for every byte that is
/// not printable.
fn write_signature(f: &mut fmt::Formatter<'_>, signature: [u8; 4]) -> fmt::Result {
    for byte in signature {
        let printable = if byte.is_ascii_graphic() { byte } else { b'.' };
        write!(f, "{}", char::from(printable))?;
    }
    Ok(())
}
