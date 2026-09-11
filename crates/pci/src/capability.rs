// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The capability list: a chain of structures in the two hundred and
//! fifty-six bytes below the extended configuration space, each naming the
//! offset of the next.
//!
//! The layout is the *PCI Express Base Specification* 6.0, section 7.5.1.1:
//! the pointer at offset `0x34`, an identifier and the next pointer at the
//! head of every entry, and a pointer of zero at the end.
//!
//! Invariant: the walk is bounded by the number of entries that fit below
//! the extended space, so a list that points back into itself is
//! [`PciError::CapabilityLoop`] and never a hang.

use crate::address::Address;
use crate::error::PciError;
use crate::header::Header;
use crate::space::{ConfigSpace, read_u8};

/// The first offset a capability may lie at, which is the first byte above
/// the header.
pub const FIRST_OFFSET: u8 = 0x40;

/// Number of bytes the capability list lies in, which is the
/// configuration space the first mechanism reached.
pub const LIST_LEN: u16 = 256;

/// Number of capabilities that fit below the extended space, which is what
/// bounds the walk: an entry is four bytes and none lies in the header.
pub const MAX_CAPABILITIES: usize = 48;

/// Capability identifier: MSI-X.
pub const ID_MSIX: u8 = 0x11;

/// Capability identifier: vendor-specific, which is what virtio 1.x uses.
pub const ID_VENDOR: u8 = 0x09;

/// One entry of the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Capability {
    /// What kind of capability it is.
    pub id: u8,
    /// Where it lies in the configuration space.
    pub offset: u16,
}

/// The capabilities of `address`, in the order the list has them.
///
/// A function without a capability list has none, which is an answer and
/// not an error.
///
/// # Errors
///
/// [`PciError::CapabilityPointer`] for a pointer into the header;
/// [`PciError::CapabilityLoop`] for a list longer than the space it lies
/// in; the errors of [`crate::space::read_word`].
pub fn walk(
    space: &impl ConfigSpace,
    address: Address,
    header: &Header,
) -> Result<[Option<Capability>; MAX_CAPABILITIES], PciError> {
    let mut found = [None; MAX_CAPABILITIES];
    if !header.has_capabilities() {
        return Ok(found);
    }
    let mut pointer = aligned(header.capabilities);
    let mut count = 0usize;
    while pointer != 0 {
        if pointer < FIRST_OFFSET {
            return Err(PciError::CapabilityPointer(pointer));
        }
        let offset = u16::from(pointer);
        let id = read_u8(space, address, offset)?;
        let slot = found.get_mut(count).ok_or(PciError::CapabilityLoop)?;
        *slot = Some(Capability { id, offset });
        count = count.saturating_add(1);
        pointer = aligned(read_u8(space, address, offset.wrapping_add(1))?);
    }
    Ok(found)
}

/// The first capability of `id`, or `None` when the list has none.
#[must_use]
pub fn find(capabilities: &[Option<Capability>; MAX_CAPABILITIES], id: u8) -> Option<Capability> {
    capabilities
        .iter()
        .flatten()
        .find(|capability| capability.id == id)
        .copied()
}

/// A pointer with its low two bits cleared, which the specification
/// reserves and every list writes as zero.
const fn aligned(pointer: u8) -> u8 {
    pointer & !0b11
}

/// Checks that a capability of `len` bytes at `offset` stays inside the
/// list.
///
/// # Errors
///
/// [`PciError::Offset`] for one that leaves it.
pub(crate) const fn fits(offset: u16, len: u16) -> Result<(), PciError> {
    if offset.saturating_add(len) > LIST_LEN {
        return Err(PciError::Offset(offset));
    }
    Ok(())
}
