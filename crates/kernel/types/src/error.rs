// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Errors of the address types.

use core::fmt;

/// Why a value could not be constructed or an operation could not be
/// performed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Error {
    /// The virtual address is not sign-extended from bit 47.
    NonCanonical(u64),
    /// The physical address does not fit into the physical address width.
    ExceedsPhysicalLimit(u64),
    /// The address is not a multiple of the required alignment.
    Unaligned {
        /// The offending address.
        address: u64,
        /// The required alignment in bytes.
        alignment: u64,
    },
    /// The alignment is zero or not a power of two.
    InvalidAlignment(u64),
    /// The result leaves the representable range.
    Overflow,
    /// The range would contain addresses of both halves of the address
    /// space.
    CrossesCanonicalHole,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NonCanonical(address) => {
                write!(f, "virtual address {address:#x} is not canonical")
            }
            Error::ExceedsPhysicalLimit(address) => {
                write!(
                    f,
                    "physical address {address:#x} exceeds the physical address width"
                )
            }
            Error::Unaligned { address, alignment } => {
                write!(
                    f,
                    "address {address:#x} is not aligned to {alignment} bytes"
                )
            }
            Error::InvalidAlignment(alignment) => {
                write!(f, "{alignment} is not a valid alignment")
            }
            Error::Overflow => f.write_str("the result leaves the representable range"),
            Error::CrossesCanonicalHole => f.write_str(
                "the range crosses the hole between the two halves of the address space",
            ),
        }
    }
}
