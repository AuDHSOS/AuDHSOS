// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The address resolution protocol of RFC 826, for Ethernet and IPv4.
//!
//! This is the only shape of ARP packet the crate reads or writes:
//! hardware type 1, protocol type `0x0800`, six bytes of hardware address
//! and four of protocol address. Every field is fixed width and the whole
//! packet is 28 bytes, so it is parsed into a value rather than borrowed
//! as a view — a view would buy nothing here and cost an accessor per
//! field. Nothing follows an ARP packet, so nothing is left to borrow.
//!
//! IPv6 has no ARP. Its resolution is Neighbor Discovery, which is
//! `ICMPv6` and lives a layer up (D-69); what the two share is the cache in
//! [`crate::neighbor`].

use net_wire::{Ipv4Addr, MacAddr, Reader, Writer};

use crate::error::EthError;

/// The hardware type of a 48-bit Ethernet address.
pub const HARDWARE_ETHERNET: u16 = 1;

/// The protocol type of an IPv4 address, which is the ether type of IPv4.
pub const PROTOCOL_IPV4: u16 = 0x0800;

/// The length of a hardware address, as the packet declares it.
pub const HARDWARE_LEN: u8 = 6;

/// The length of a protocol address, as the packet declares it.
pub const PROTOCOL_LEN: u8 = 4;

/// How many bytes an ARP packet for Ethernet and IPv4 occupies.
pub const PACKET_LEN: usize = 28;

/// What the packet asks or answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Operation {
    /// Who has this protocol address?
    Request,
    /// This hardware address has it.
    Reply,
}

impl Operation {
    /// The number on the wire.
    #[must_use]
    pub const fn get(self) -> u16 {
        match self {
            Operation::Request => 1,
            Operation::Reply => 2,
        }
    }

    /// The operation of this number.
    ///
    /// # Errors
    ///
    /// [`EthError::UnknownOperation`] for anything but one and two. RFC 826
    /// leaves the field open and later documents put other operations in
    /// it; none of them is one this system answers, so the packet is
    /// refused rather than half read.
    pub const fn new(number: u16) -> Result<Operation, EthError> {
        match number {
            1 => Ok(Operation::Request),
            2 => Ok(Operation::Reply),
            _ => Err(EthError::UnknownOperation(number)),
        }
    }
}

/// One ARP packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Packet {
    /// Whether it asks or answers.
    pub operation: Operation,
    /// The hardware address of the station that sent it.
    pub sender_hardware: MacAddr,
    /// The protocol address of that station.
    pub sender_protocol: Ipv4Addr,
    /// The hardware address being asked for, which a request leaves
    /// unset.
    pub target_hardware: MacAddr,
    /// The protocol address the packet is about.
    pub target_protocol: Ipv4Addr,
}

impl Packet {
    /// A request asking who holds `target_protocol`.
    ///
    /// The target hardware address is left unspecified, which RFC 826
    /// says is the field the sender is trying to learn.
    #[must_use]
    pub const fn request(
        sender_hardware: MacAddr,
        sender_protocol: Ipv4Addr,
        target_protocol: Ipv4Addr,
    ) -> Packet {
        Packet {
            operation: Operation::Request,
            sender_hardware,
            sender_protocol,
            target_hardware: MacAddr::UNSPECIFIED,
            target_protocol,
        }
    }

    /// The reply to `request`, sent by the station that holds the address
    /// it asks for.
    #[must_use]
    pub const fn reply_to(request: &Packet, hardware: MacAddr) -> Packet {
        Packet {
            operation: Operation::Reply,
            sender_hardware: hardware,
            sender_protocol: request.target_protocol,
            target_hardware: request.sender_hardware,
            target_protocol: request.sender_protocol,
        }
    }

    /// Whether the packet announces its sender's own address rather than
    /// asking about someone else's, which is what a gratuitous ARP is.
    #[must_use]
    pub fn is_gratuitous(&self) -> bool {
        self.sender_protocol == self.target_protocol
    }

    /// The packet in `bytes`.
    ///
    /// # Errors
    ///
    /// [`EthError::NotEthernetIpv4`] when the packet describes some other
    /// pair of address spaces, [`EthError::UnknownOperation`] for an
    /// operation this system does not answer, and [`EthError::Wire`] when
    /// there are fewer than 28 bytes.
    pub fn parse(bytes: &[u8]) -> Result<Packet, EthError> {
        let mut reader = Reader::new(bytes);
        let hardware_type = reader.read_u16()?;
        let protocol_type = reader.read_u16()?;
        let hardware_len = reader.read_u8()?;
        let protocol_len = reader.read_u8()?;
        if hardware_type != HARDWARE_ETHERNET
            || protocol_type != PROTOCOL_IPV4
            || hardware_len != HARDWARE_LEN
            || protocol_len != PROTOCOL_LEN
        {
            return Err(EthError::NotEthernetIpv4);
        }
        let operation = Operation::new(reader.read_u16()?)?;
        Ok(Packet {
            operation,
            sender_hardware: reader.read_mac()?,
            sender_protocol: reader.read_ipv4()?,
            target_hardware: reader.read_mac()?,
            target_protocol: reader.read_ipv4()?,
        })
    }

    /// Writes the packet in the field order of RFC 826.
    ///
    /// # Errors
    ///
    /// [`EthError::Wire`] when fewer than 28 bytes are free.
    pub fn write(&self, writer: &mut Writer<'_>) -> Result<(), EthError> {
        if writer.remaining() < PACKET_LEN {
            return Err(EthError::Wire(net_wire::WireError::OutOfBounds {
                needed: PACKET_LEN,
                available: writer.remaining(),
            }));
        }
        writer.write_u16(HARDWARE_ETHERNET)?;
        writer.write_u16(PROTOCOL_IPV4)?;
        writer.write_u8(HARDWARE_LEN)?;
        writer.write_u8(PROTOCOL_LEN)?;
        writer.write_u16(self.operation.get())?;
        writer.write_mac(self.sender_hardware)?;
        writer.write_ipv4(self.sender_protocol)?;
        writer.write_mac(self.target_hardware)?;
        writer.write_ipv4(self.target_protocol)?;
        Ok(())
    }
}

/// The reply a station owes for `packet`, if any.
///
/// A station answers exactly one question: who holds *its* address. A
/// request for another address is not answered — RFC 826 has the owner of
/// the address reply and nobody else, and a station that answered for its
/// neighbors would be doing proxy ARP, which this system does not do.
/// A reply is never answered at all.
#[must_use]
pub fn respond(packet: &Packet, interface: MacAddr, address: Ipv4Addr) -> Option<Packet> {
    if packet.operation != Operation::Request || packet.target_protocol != address {
        return None;
    }
    Some(Packet::reply_to(packet, interface))
}
