// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The feature bits of a network device (virtio 5.1.3), what this driver
//! takes, and what it refuses.
//!
//! Every bit the device may offer is named here even where the driver
//! refuses it, so that an omission reads as a decision and a refusal can
//! say which feature it turned down.

use virtio_queue::F_VERSION_1;

/// `VIRTIO_NET_F_CSUM`: the device takes partially checksummed packets.
pub const F_CSUM: u64 = 1 << 0;

/// `VIRTIO_NET_F_GUEST_CSUM`: the driver takes them.
pub const F_GUEST_CSUM: u64 = 1 << 1;

/// `VIRTIO_NET_F_CTRL_GUEST_OFFLOADS`: the offloads can be switched
/// through the control queue.
pub const F_CTRL_GUEST_OFFLOADS: u64 = 1 << 2;

/// `VIRTIO_NET_F_MTU`: the device names the MTU of the link.
pub const F_MTU: u64 = 1 << 3;

/// `VIRTIO_NET_F_MAC`: the device configuration carries a MAC address.
pub const F_MAC: u64 = 1 << 5;

/// `VIRTIO_NET_F_GUEST_TSO4`: the driver takes segmented IPv4 packets.
pub const F_GUEST_TSO4: u64 = 1 << 7;

/// `VIRTIO_NET_F_GUEST_TSO6`: the same for IPv6.
pub const F_GUEST_TSO6: u64 = 1 << 8;

/// `VIRTIO_NET_F_GUEST_ECN`: the driver takes segmented packets with the
/// explicit congestion bits.
pub const F_GUEST_ECN: u64 = 1 << 9;

/// `VIRTIO_NET_F_GUEST_UFO`: the driver takes fragmented UDP.
pub const F_GUEST_UFO: u64 = 1 << 10;

/// `VIRTIO_NET_F_HOST_TSO4`: the device takes segmented IPv4 packets.
pub const F_HOST_TSO4: u64 = 1 << 11;

/// `VIRTIO_NET_F_HOST_TSO6`: the same for IPv6.
pub const F_HOST_TSO6: u64 = 1 << 12;

/// `VIRTIO_NET_F_HOST_ECN`: the device takes them with the explicit
/// congestion bits.
pub const F_HOST_ECN: u64 = 1 << 13;

/// `VIRTIO_NET_F_HOST_UFO`: the device takes fragmented UDP.
pub const F_HOST_UFO: u64 = 1 << 14;

/// `VIRTIO_NET_F_MRG_RXBUF`: one frame may span several receive buffers.
pub const F_MRG_RXBUF: u64 = 1 << 15;

/// `VIRTIO_NET_F_STATUS`: the device configuration carries a link status.
pub const F_STATUS: u64 = 1 << 16;

/// `VIRTIO_NET_F_CTRL_VQ`: the device has a control queue.
pub const F_CTRL_VQ: u64 = 1 << 17;

/// `VIRTIO_NET_F_CTRL_RX`: the receive filters are settable.
pub const F_CTRL_RX: u64 = 1 << 18;

/// `VIRTIO_NET_F_CTRL_VLAN`: the VLAN filter is settable.
pub const F_CTRL_VLAN: u64 = 1 << 19;

/// `VIRTIO_NET_F_CTRL_RX_EXTRA`: the extra receive modes are settable.
pub const F_CTRL_RX_EXTRA: u64 = 1 << 20;

/// `VIRTIO_NET_F_GUEST_ANNOUNCE`: the driver answers a gratuitous packet
/// on request.
pub const F_GUEST_ANNOUNCE: u64 = 1 << 21;

/// `VIRTIO_NET_F_MQ`: more than one pair of queues.
pub const F_MQ: u64 = 1 << 22;

/// `VIRTIO_NET_F_CTRL_MAC_ADDR`: the MAC address is settable through the
/// control queue.
pub const F_CTRL_MAC_ADDR: u64 = 1 << 23;

/// `VIRTIO_NET_F_DEVICE_STATS`: the device counts, through the control
/// queue.
pub const F_DEVICE_STATS: u64 = 1 << 50;

/// `VIRTIO_NET_F_HASH_TUNNEL`: the device hashes the inner header of an
/// encapsulated packet.
pub const F_HASH_TUNNEL: u64 = 1 << 51;

/// `VIRTIO_NET_F_VQ_NOTF_COAL`: notifications are coalesced per queue.
pub const F_VQ_NOTF_COAL: u64 = 1 << 52;

/// `VIRTIO_NET_F_NOTF_COAL`: notifications are coalesced.
pub const F_NOTF_COAL: u64 = 1 << 53;

/// `VIRTIO_NET_F_GUEST_USO4`: the driver takes segmented IPv4 UDP.
pub const F_GUEST_USO4: u64 = 1 << 54;

/// `VIRTIO_NET_F_GUEST_USO6`: the same for IPv6.
pub const F_GUEST_USO6: u64 = 1 << 55;

/// `VIRTIO_NET_F_HOST_USO`: the device takes segmented UDP.
pub const F_HOST_USO: u64 = 1 << 56;

/// `VIRTIO_NET_F_HASH_REPORT`: the device reports a hash of each packet.
pub const F_HASH_REPORT: u64 = 1 << 57;

/// `VIRTIO_NET_F_GUEST_HDRLEN`: the driver states the header length it
/// writes.
pub const F_GUEST_HDRLEN: u64 = 1 << 59;

/// `VIRTIO_NET_F_RSS`: receive-side scaling.
pub const F_RSS: u64 = 1 << 60;

/// `VIRTIO_NET_F_RSC_EXT`: the device coalesces received segments.
pub const F_RSC_EXT: u64 = 1 << 61;

/// `VIRTIO_NET_F_STANDBY`: the device is the standby half of a pair.
pub const F_STANDBY: u64 = 1 << 62;

/// `VIRTIO_NET_F_SPEED_DUPLEX`: the device reports speed and duplex.
pub const F_SPEED_DUPLEX: u64 = 1 << 63;

/// What this driver asks the device for.
///
/// `VIRTIO_F_VERSION_1` because nothing here reads a legacy device.
/// `VIRTIO_NET_F_MAC` because a host that has to invent its own hardware
/// address is a host two of which collide on one link.
///
/// Every other bit is refused. `VIRTIO_NET_F_MRG_RXBUF` is the one that
/// changes the receive path: without it one buffer holds one whole frame
/// and one used element is one frame (D-114). Each offload would put a
/// field of the header under the device's control that this driver writes
/// as zero, the control queue would be a third queue with a protocol of
/// its own, and `VIRTIO_NET_F_MQ` would be more of both.
pub const WANTED: u64 = F_VERSION_1 | F_MAC;

/// Every bit this module has a name for.
const NAMED: [(u64, &str); 35] = [
    (F_CSUM, "VIRTIO_NET_F_CSUM"),
    (F_GUEST_CSUM, "VIRTIO_NET_F_GUEST_CSUM"),
    (F_CTRL_GUEST_OFFLOADS, "VIRTIO_NET_F_CTRL_GUEST_OFFLOADS"),
    (F_MTU, "VIRTIO_NET_F_MTU"),
    (F_MAC, "VIRTIO_NET_F_MAC"),
    (F_GUEST_TSO4, "VIRTIO_NET_F_GUEST_TSO4"),
    (F_GUEST_TSO6, "VIRTIO_NET_F_GUEST_TSO6"),
    (F_GUEST_ECN, "VIRTIO_NET_F_GUEST_ECN"),
    (F_GUEST_UFO, "VIRTIO_NET_F_GUEST_UFO"),
    (F_HOST_TSO4, "VIRTIO_NET_F_HOST_TSO4"),
    (F_HOST_TSO6, "VIRTIO_NET_F_HOST_TSO6"),
    (F_HOST_ECN, "VIRTIO_NET_F_HOST_ECN"),
    (F_HOST_UFO, "VIRTIO_NET_F_HOST_UFO"),
    (F_MRG_RXBUF, "VIRTIO_NET_F_MRG_RXBUF"),
    (F_STATUS, "VIRTIO_NET_F_STATUS"),
    (F_CTRL_VQ, "VIRTIO_NET_F_CTRL_VQ"),
    (F_CTRL_RX, "VIRTIO_NET_F_CTRL_RX"),
    (F_CTRL_VLAN, "VIRTIO_NET_F_CTRL_VLAN"),
    (F_CTRL_RX_EXTRA, "VIRTIO_NET_F_CTRL_RX_EXTRA"),
    (F_GUEST_ANNOUNCE, "VIRTIO_NET_F_GUEST_ANNOUNCE"),
    (F_MQ, "VIRTIO_NET_F_MQ"),
    (F_CTRL_MAC_ADDR, "VIRTIO_NET_F_CTRL_MAC_ADDR"),
    (F_DEVICE_STATS, "VIRTIO_NET_F_DEVICE_STATS"),
    (F_HASH_TUNNEL, "VIRTIO_NET_F_HASH_TUNNEL"),
    (F_VQ_NOTF_COAL, "VIRTIO_NET_F_VQ_NOTF_COAL"),
    (F_NOTF_COAL, "VIRTIO_NET_F_NOTF_COAL"),
    (F_GUEST_USO4, "VIRTIO_NET_F_GUEST_USO4"),
    (F_GUEST_USO6, "VIRTIO_NET_F_GUEST_USO6"),
    (F_HOST_USO, "VIRTIO_NET_F_HOST_USO"),
    (F_HASH_REPORT, "VIRTIO_NET_F_HASH_REPORT"),
    (F_GUEST_HDRLEN, "VIRTIO_NET_F_GUEST_HDRLEN"),
    (F_RSS, "VIRTIO_NET_F_RSS"),
    (F_RSC_EXT, "VIRTIO_NET_F_RSC_EXT"),
    (F_STANDBY, "VIRTIO_NET_F_STANDBY"),
    (F_SPEED_DUPLEX, "VIRTIO_NET_F_SPEED_DUPLEX"),
];

/// What `bit` is called, for a bit of the network device's own range.
///
/// A bit of the transport's range, which virtio 6 reserves from 24 to 40,
/// has no name here: those belong to no device type and this driver takes
/// none of them beyond `VIRTIO_F_VERSION_1`. Above 63 there is no name
/// either, and no bit: the driver reads two windows of thirty-two, so a
/// feature of virtio 1.4 numbered 64 or higher is one it never sees.
#[must_use]
pub fn name(bit: u64) -> Option<&'static str> {
    NAMED
        .iter()
        .find(|(value, _)| *value == bit)
        .map(|(_, name)| *name)
}

/// The bits `offered` carries that this driver does not take.
///
/// What comes back is a set, and a caller that reports it walks the bits
/// with [`name`]. It is not an error: virtio 2.2.1 leaves a feature the
/// driver did not want to the driver's judgement, and the judgement here
/// is that the two queues of 5.1.2 need none of them.
#[must_use]
pub const fn refused(offered: u64) -> u64 {
    offered & !WANTED
}
