// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The datagram of RFC 768: four fields, and the two checksum rules the
//! address families do not share.
//!
//! A datagram is a borrowed view over the bytes it arrived in. The length
//! field is what decides where the payload ends: the layer below may hand
//! up more bytes than the datagram claims — an Ethernet frame is padded to
//! sixty bytes and a short datagram inside one arrives with the padding
//! still on it — and those bytes are cut away rather than counted. A
//! length that reaches past what arrived is an error, because there is
//! nothing to read there.
//!
//! The checksum covers a pseudo-header of the two addresses, which is what
//! binds a datagram to the addresses it was addressed with. Over IPv4 the
//! field may be zero, meaning the sender computed none; over IPv6 a zero
//! is a datagram to discard, because RFC 8200, section 8.1 removed the
//! header checksum that used to catch a corrupted address underneath it. A
//! sum that comes out zero goes on the wire as all ones in both families,
//! so that the omitted form keeps a spelling of its own.

use net_wire::{IpAddr, Port, Protocol, WireError, Writer, transport};

use crate::error::UdpError;

/// The four fields of the header: two ports, the length, and the checksum.
pub const HEADER_LEN: usize = 8;

/// The header length as the field itself carries it.
const HEADER_LEN_U16: u16 = 8;

/// The longest payload the length field can express.
pub const MAX_PAYLOAD_LEN: usize = 65_527;

/// Where the checksum field sits inside the header.
const CHECKSUM_OFFSET: usize = 6;

/// The value a sum of zero goes on the wire as, so that it is not read as
/// an omitted checksum. RFC 768 states it for IPv4 and RFC 8200,
/// section 8.1 repeats it for IPv6.
const ZERO_ON_THE_WIRE: u16 = 0xFFFF;

/// Whether a datagram carries a checksum.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ChecksumPolicy {
    /// Sum the pseudo-header and the datagram. This is the only choice
    /// over IPv6.
    #[default]
    Computed,
    /// Leave the field zero, which RFC 768 permits over IPv4 and RFC 8200,
    /// section 8.1 forbids over IPv6.
    Omitted,
}

/// One datagram, borrowed from the bytes it arrived in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Datagram<'a> {
    /// Where it came from.
    pub source_port: Port,
    /// Where it is going.
    pub destination_port: Port,
    /// What it carries.
    pub payload: &'a [u8],
}

impl<'a> Datagram<'a> {
    /// The datagram in `bytes`, which arrived from `source` and was
    /// addressed to `destination`.
    ///
    /// The two addresses are not in the datagram; they come from the
    /// header below it and are needed to sum the pseudo-header. The
    /// payload ends where the length field says, whatever else arrived
    /// behind it.
    ///
    /// # Errors
    ///
    /// [`UdpError::MixedFamilies`] when the two addresses are not of one
    /// family, [`UdpError::Wire`] when there is not even a header,
    /// [`UdpError::Length`] when the length field is below the header or
    /// beyond the bytes, [`UdpError::ChecksumRequired`] for a zero
    /// checksum over IPv6, and [`UdpError::Checksum`] when a non-zero one
    /// does not verify.
    pub fn parse(
        bytes: &'a [u8],
        source: IpAddr,
        destination: IpAddr,
    ) -> Result<Datagram<'a>, UdpError> {
        if source.version() != destination.version() {
            return Err(UdpError::MixedFamilies);
        }
        let Some(header) = bytes.first_chunk::<HEADER_LEN>() else {
            return Err(UdpError::Wire(WireError::OutOfBounds {
                needed: HEADER_LEN,
                available: bytes.len(),
            }));
        };
        let [
            source_high,
            source_low,
            destination_high,
            destination_low,
            length_high,
            length_low,
            carried_high,
            carried_low,
        ] = *header;
        let length = u16::from_be_bytes([length_high, length_low]);
        let carried = u16::from_be_bytes([carried_high, carried_low]);
        let claimed = usize::from(length);
        if claimed < HEADER_LEN {
            return Err(UdpError::Length(length));
        }
        let Some(datagram) = bytes.get(..claimed) else {
            return Err(UdpError::Length(length));
        };
        verify(datagram, carried, source, destination)?;
        Ok(Datagram {
            source_port: Port::new(u16::from_be_bytes([source_high, source_low])),
            destination_port: Port::new(u16::from_be_bytes([destination_high, destination_low])),
            // The length field was checked against the header length
            // above, so the payload begins inside the datagram.
            payload: datagram.get(HEADER_LEN..).unwrap_or(&[]),
        })
    }

    /// Writes the datagram from `source` to `destination`, with or without
    /// a checksum.
    ///
    /// Either the whole datagram is written or none of it: the room is
    /// checked before the first byte goes down, so a buffer that is too
    /// small leaves the writer where it was.
    ///
    /// # Errors
    ///
    /// [`UdpError::MixedFamilies`] when the two addresses are not of one
    /// family, [`UdpError::ChecksumRequired`] when a checksum is to be
    /// omitted over IPv6, [`UdpError::TooLarge`] when the payload is
    /// longer than the length field can express, and [`UdpError::Wire`]
    /// when the buffer has no room.
    pub fn write(
        &self,
        writer: &mut Writer<'_>,
        source: IpAddr,
        destination: IpAddr,
        checksum: ChecksumPolicy,
    ) -> Result<(), UdpError> {
        if source.version() != destination.version() {
            return Err(UdpError::MixedFamilies);
        }
        if checksum == ChecksumPolicy::Omitted && source.is_v6() {
            return Err(UdpError::ChecksumRequired);
        }
        let Ok(payload_len) = u16::try_from(self.payload.len()) else {
            return Err(UdpError::TooLarge(self.payload.len()));
        };
        let Some(length) = payload_len.checked_add(HEADER_LEN_U16) else {
            return Err(UdpError::TooLarge(self.payload.len()));
        };
        let needed = usize::from(length);
        if writer.remaining() < needed {
            return Err(UdpError::Wire(WireError::OutOfBounds {
                needed,
                available: writer.remaining(),
            }));
        }
        let at = writer.position();
        let [source_high, source_low] = self.source_port.get().to_be_bytes();
        let [destination_high, destination_low] = self.destination_port.get().to_be_bytes();
        let [length_high, length_low] = length.to_be_bytes();
        writer.write_bytes(&[
            source_high,
            source_low,
            destination_high,
            destination_low,
            length_high,
            length_low,
            0,
            0,
        ])?;
        writer.write_bytes(self.payload)?;
        if checksum == ChecksumPolicy::Computed {
            // Everything from `at` on is what was just written, and the
            // checksum field in it is still zero.
            let segment = writer.written().get(at..).unwrap_or(&[]);
            let sum = transport(source, destination, Protocol::UDP, segment)?;
            let carried = if sum == 0 { ZERO_ON_THE_WIRE } else { sum };
            writer.patch_u16(at.saturating_add(CHECKSUM_OFFSET), carried)?;
        }
        Ok(())
    }
}

/// Checks the checksum `carried` against `datagram`, under the rule of the
/// family the two addresses belong to.
fn verify(
    datagram: &[u8],
    carried: u16,
    source: IpAddr,
    destination: IpAddr,
) -> Result<(), UdpError> {
    if carried == 0 {
        return if source.is_v6() {
            Err(UdpError::ChecksumRequired)
        } else {
            Ok(())
        };
    }
    // Summing a block that holds its own checksum yields zero when the
    // checksum is right, whichever field it sits in: RFC 1071, section 1.
    if transport(source, destination, Protocol::UDP, datagram)? == 0 {
        Ok(())
    } else {
        Err(UdpError::Checksum(carried))
    }
}
