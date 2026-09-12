// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The vendor-specific capabilities of virtio 1.x: which base address
//! register a structure lies in, where in it, and how long it is.
//!
//! The layout is section 4.1.4 of
//! [`docs/oasis/virtio-v1.4-cs01.html`](../../../docs/oasis/README.md):
//! `cap_vndr`, `cap_next`, `cap_len`, `cfg_type`, `bar`, `id`, two bytes of
//! padding, the offset, the length, and — for the notification structure
//! alone — the multiplier that turns a queue's notification offset into an
//! address.
//!
//! Two rules of 4.1.4 this module keeps and a driver depends on: a device
//! may publish more than one structure of a type, and the order of the
//! capability list is the device's order of preference. So every structure
//! is reported, in that order, and none is chosen here.

use crate::address::Address;
use crate::capability::{Capability, ID_VENDOR, MAX_CAPABILITIES, fits};
use crate::error::PciError;
use crate::space::{ConfigSpace, read_u8, read_word};

/// The vendor every virtio device carries.
pub const VIRTIO_VENDOR: u16 = 0x1AF4;

/// The device identifier of virtio device type zero; a device of type `n`
/// carries `DEVICE_BASE + n` (virtio 4.1.2).
pub const DEVICE_BASE: u16 = 0x1040;

/// The device identifier of a virtio network device, which is device type
/// one.
pub const NETWORK_DEVICE: u16 = DEVICE_BASE + 1;

/// The device identifier of a virtio block device, which is device type
/// two (virtio 5.2.1).
pub const BLOCK_DEVICE: u16 = DEVICE_BASE + 2;

/// Number of structures this crate reports of one function.
pub const MAX_STRUCTURES: usize = 16;

/// Number of bytes of a capability that is not the notification one.
pub const CAPABILITY_LEN: u8 = 16;

/// Number of bytes of the notification capability, which carries the
/// multiplier as well.
pub const NOTIFY_CAPABILITY_LEN: u8 = 20;

/// The highest base address register a capability may name (4.1.4).
const MAX_BAR: u8 = 5;

/// Offset of `cfg_type` inside the capability.
const CFG_TYPE: u16 = 3;

/// Offset of `bar` inside the capability.
const BAR: u16 = 4;

/// Offset of `id` inside the capability.
const ID: u16 = 5;

/// Offset of the structure's offset inside the capability.
const OFFSET: u16 = 8;

/// Offset of the structure's length inside the capability.
const LENGTH: u16 = 12;

/// Offset of the notification multiplier inside the capability.
const MULTIPLIER: u16 = 16;

/// What a structure is for. All seven values the specification defines are
/// named, the four a driver of this system uses and the three it does not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// The common configuration: status, features, and the queues.
    Common,
    /// The notification structure a driver writes a queue index into.
    Notify,
    /// The interrupt status register.
    Isr,
    /// The configuration of the device type, such as a network device's
    /// MAC address.
    Device,
    /// Access to the other structures through configuration space, for a
    /// driver that cannot map them.
    PciConfig,
    /// A region the device and the driver share.
    SharedMemory,
    /// Data whose meaning the device's vendor gives it.
    Vendor,
}

impl Kind {
    /// The kind `cfg_type` names, or `None` for a value the specification
    /// reserves. A reserved value is skipped rather than refused: a device
    /// may publish what a driver does not know.
    #[must_use]
    pub const fn of(cfg_type: u8) -> Option<Kind> {
        match cfg_type {
            1 => Some(Kind::Common),
            2 => Some(Kind::Notify),
            3 => Some(Kind::Isr),
            4 => Some(Kind::Device),
            5 => Some(Kind::PciConfig),
            8 => Some(Kind::SharedMemory),
            9 => Some(Kind::Vendor),
            _ => None,
        }
    }

    /// Number of bytes a capability of this kind has to announce.
    #[must_use]
    pub const fn capability_len(self) -> u8 {
        match self {
            Kind::Notify => NOTIFY_CAPABILITY_LEN,
            Kind::Common
            | Kind::Isr
            | Kind::Device
            | Kind::PciConfig
            | Kind::SharedMemory
            | Kind::Vendor => CAPABILITY_LEN,
        }
    }
}

/// One structure a device published.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Structure {
    /// What the structure is for.
    pub kind: Kind,
    /// The base address register it lies in.
    pub bar: u8,
    /// The identifier that tells two structures of a type apart, for the
    /// device types that give it a meaning.
    pub id: u8,
    /// How far into that register's range the structure starts.
    pub offset: u32,
    /// How many bytes it covers.
    pub len: u32,
    /// The multiplier of the notification structure, and nothing for the
    /// other six kinds.
    pub multiplier: Option<u32>,
}

/// Every structure the capabilities of `address` name, in the order the
/// capability list has them.
///
/// # Errors
///
/// [`PciError::CapabilityLength`] for a capability shorter than its kind
/// needs; [`PciError::CapabilityBar`] for one naming a register a function
/// has not; [`PciError::Offset`] for one that would leave the list; the
/// errors of [`crate::space::read_word`].
pub fn structures(
    space: &impl ConfigSpace,
    address: Address,
    capabilities: &[Option<Capability>; MAX_CAPABILITIES],
) -> Result<[Option<Structure>; MAX_STRUCTURES], PciError> {
    let mut found = [None; MAX_STRUCTURES];
    let mut count = 0usize;
    for capability in capabilities.iter().flatten() {
        if capability.id != ID_VENDOR {
            continue;
        }
        let Some(structure) = read(space, address, capability.offset)? else {
            continue;
        };
        if let Some(slot) = found.get_mut(count) {
            *slot = Some(structure);
            count = count.saturating_add(1);
        }
    }
    Ok(found)
}

/// The structure the capability at `offset` names, or `None` for a
/// `cfg_type` the specification reserves.
///
/// # Errors
///
/// The errors of [`structures`].
pub fn read(
    space: &impl ConfigSpace,
    address: Address,
    offset: u16,
) -> Result<Option<Structure>, PciError> {
    fits(offset, u16::from(CAPABILITY_LEN))?;
    let announced = read_u8(space, address, offset.wrapping_add(2))?;
    let cfg_type = read_u8(space, address, offset.wrapping_add(CFG_TYPE))?;
    let Some(kind) = Kind::of(cfg_type) else {
        return Ok(None);
    };
    let needed = kind.capability_len();
    if announced < needed {
        return Err(PciError::CapabilityLength {
            id: ID_VENDOR,
            length: announced,
        });
    }
    fits(offset, u16::from(needed))?;
    let bar = read_u8(space, address, offset.wrapping_add(BAR))?;
    if bar > MAX_BAR {
        return Err(PciError::CapabilityBar(bar));
    }
    let multiplier = match kind {
        Kind::Notify => Some(read_word(space, address, offset.wrapping_add(MULTIPLIER))?),
        Kind::Common
        | Kind::Isr
        | Kind::Device
        | Kind::PciConfig
        | Kind::SharedMemory
        | Kind::Vendor => None,
    };
    Ok(Some(Structure {
        kind,
        bar,
        id: read_u8(space, address, offset.wrapping_add(ID))?,
        offset: read_word(space, address, offset.wrapping_add(OFFSET))?,
        len: read_word(space, address, offset.wrapping_add(LENGTH))?,
        multiplier,
    }))
}

/// The first structure of `kind`, which is the one the device prefers.
#[must_use]
pub fn first(structures: &[Option<Structure>; MAX_STRUCTURES], kind: Kind) -> Option<Structure> {
    structures
        .iter()
        .flatten()
        .find(|structure| structure.kind == kind)
        .copied()
}

/// `true` when the two identifiers are a virtio 1.x device of `device_type`
/// and not the transitional device of the same type (4.1.2).
#[must_use]
pub const fn is_modern(vendor: u16, device: u16, device_type: u16) -> bool {
    vendor == VIRTIO_VENDOR && device == DEVICE_BASE.saturating_add(device_type)
}
