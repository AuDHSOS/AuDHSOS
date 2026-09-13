// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The device configuration of a network device (virtio 5.1.4), and the
//! header every frame travels behind (`struct virtio_net_hdr`, virtio
//! 5.1.9).

use crate::common::CONFIG_GENERATION;
use crate::error::NetError;
use crate::registers::{Registers, Structure, Width, low_u8};

/// The six bytes of the MAC address, with `VIRTIO_NET_F_MAC`.
pub const MAC: u16 = 0x00;

/// The link status, with `VIRTIO_NET_F_STATUS`.
pub const STATUS: u16 = 0x06;

/// How many pairs of queues the device has, with `VIRTIO_NET_F_MQ`.
pub const MAX_QUEUE_PAIRS: u16 = 0x08;

/// The MTU of the link, with `VIRTIO_NET_F_MTU`.
pub const MTU: u16 = 0x0A;

/// Bytes of a hardware address.
pub const MAC_LEN: usize = 6;

/// Bytes of the header of virtio 5.1.9.
///
/// Twelve, because `num_buffers` is a field of every virtio 1.x header
/// whether or not `VIRTIO_NET_F_MRG_RXBUF` was negotiated; the ten-byte
/// form is the legacy interface, which this driver does not read.
pub const HEADER_LEN: usize = 12;

/// How many times the configuration is read before it is refused.
///
/// Virtio 2.5.1 has the driver read the generation, then the field, then
/// the generation again, and start over where the two disagree. The loop
/// is bounded rather than open: a device that changes its configuration
/// faster than it can be read is a device this driver reports instead of
/// spinning on.
pub const ATTEMPTS: u32 = 8;

/// The hardware address the device carries.
///
/// Six bytes is wider than virtio 2.5.1 lets a driver assume is read at
/// once, so the read is the one that section prescribes.
///
/// # Errors
///
/// [`NetError::Generation`] when the configuration changed under every one
/// of [`ATTEMPTS`] reads.
pub fn mac(registers: &impl Registers) -> Result<[u8; MAC_LEN], NetError> {
    stable(registers, || {
        let mut address = [0u8; MAC_LEN];
        for (step, byte) in address.iter_mut().enumerate() {
            let offset = MAC.saturating_add(u16::try_from(step).unwrap_or(0));
            *byte = low_u8(registers.read(Structure::Device, offset, Width::B8));
        }
        address
    })
}

/// The value `read` answers with the generation unchanged around it.
fn stable<T: PartialEq>(registers: &impl Registers, read: impl Fn() -> T) -> Result<T, NetError> {
    for _ in 0..ATTEMPTS {
        let before = registers.read(Structure::Common, CONFIG_GENERATION, Width::B8);
        let value = read();
        let after = registers.read(Structure::Common, CONFIG_GENERATION, Width::B8);
        if before == after {
            return Ok(value);
        }
    }
    Err(NetError::Generation(ATTEMPTS))
}

/// Writes the header of a frame this driver sends.
///
/// Every field is zero, and each zero is required rather than convenient.
/// `VIRTIO_NET_F_CSUM` was not negotiated, so virtio 5.1.9.2.1 has the
/// driver set `flags` to zero and hand over a fully checksummed packet,
/// which leaves `csum_start` and `csum_offset` without a meaning; no
/// segmentation was negotiated, so `gso_type` is `VIRTIO_NET_HDR_GSO_NONE`
/// and `hdr_len` and `gso_size` with it; and the same section has the
/// driver set `num_buffers` to zero, the field being unused on a
/// transmitted packet.
///
/// # Errors
///
/// [`NetError::Buffer`] when `into` is shorter than [`HEADER_LEN`]; the
/// number is `index`, so a refusal names the buffer.
pub fn write_header(into: &mut [u8], index: u16) -> Result<(), NetError> {
    let header = into.get_mut(..HEADER_LEN).ok_or(NetError::Buffer(index))?;
    header.fill(0);
    Ok(())
}
