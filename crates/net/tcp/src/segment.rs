// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The segment of RFC 9293, section 3.1: twenty bytes of header, an option
//! list, and the bytes behind it.
//!
//! One option is read and written, the maximum segment size of
//! section 3.2, and it is read only where it is allowed to appear — on a
//! segment that carries `SYN`. The others are stepped over by their length
//! and never interpreted (D-50: no window scaling, no selective
//! acknowledgment, no timestamps). Stepping over them is not a courtesy:
//! an option list is a length field an attacker writes, so a length below
//! the two bytes an option takes, or one that reaches past the header, is
//! an error and not a value to clamp.
//!
//! The checksum is mandatory in both families and has no omitted form, so
//! there is nothing here of the shape UDP has. It covers the
//! pseudo-header, which is what binds a segment to the addresses it was
//! addressed with, and a segment whose sum does not come out is a segment
//! that never reaches the state machine.
//!
//! A segment has no length field of its own: it is as long as the
//! internet layer says its payload is. Padding a link layer left standing
//! is therefore not something this parser can cut away, and it does not
//! try — the checksum covers what it was handed, so a padded segment is
//! refused rather than half read. The layer below has the length field
//! that settles it.
//!
//! The urgent pointer is read and never acted on. This system sends no
//! urgent data and has no interface through which a caller could ask for
//! it; a segment that carries `URG` is processed for everything else it
//! carries, which is what RFC 6093 recommends for a receiver that has no
//! use for the mechanism (D-85).

use core::fmt;

use net_wire::{IpAddr, Port, Protocol, Reader, WireError, Writer, transport};

use crate::error::TcpError;
use crate::seq::SeqNumber;

/// The fixed part of every header.
pub const HEADER_LEN: usize = 20;

/// The smallest data offset there is, in words.
const MIN_DATA_OFFSET: u8 = 5;

/// The largest, which is what the four-bit field holds.
const MAX_DATA_OFFSET: u8 = 15;

/// Where the checksum sits.
const CHECKSUM_OFFSET: usize = 16;

/// The kinds of option this crate knows by name.
mod option {
    /// End of the list; nothing behind it is read.
    pub(super) const END: u8 = 0;
    /// A single byte of padding.
    pub(super) const NOP: u8 = 1;
    /// The maximum segment size, which is four bytes long.
    pub(super) const MAX_SEGMENT_SIZE: u8 = 2;
    /// How long the maximum segment size option is.
    pub(super) const MAX_SEGMENT_SIZE_LEN: u8 = 4;
    /// The two bytes every option but [`END`] and [`NOP`] begins with.
    pub(super) const MIN_LEN: u8 = 2;
}

/// The control bits of a segment.
///
/// A set rather than an enumeration, because a segment carries several at
/// once — `SYN` with `ACK`, `FIN` with `ACK` — and because a bit this
/// implementation does not act on is a bit it must still carry through
/// unchanged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Flags(u8);

impl Flags {
    /// No bit at all.
    pub const NONE: Flags = Flags(0);
    /// No more data from the sender.
    pub const FIN: Flags = Flags(0x01);
    /// Synchronize sequence numbers.
    pub const SYN: Flags = Flags(0x02);
    /// Reset the connection.
    pub const RST: Flags = Flags(0x04);
    /// Push the data to the reader.
    pub const PSH: Flags = Flags(0x08);
    /// The acknowledgment field is meaningful.
    pub const ACK: Flags = Flags(0x10);
    /// The urgent pointer is meaningful.
    pub const URG: Flags = Flags(0x20);

    /// The bits of this number.
    #[must_use]
    pub const fn new(bits: u8) -> Flags {
        Flags(bits)
    }

    /// The number.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Whether every bit of `other` is set here.
    #[must_use]
    pub const fn has(self, other: Flags) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether any bit of `other` is set here.
    #[must_use]
    pub const fn has_any(self, other: Flags) -> bool {
        self.0 & other.0 != 0
    }

    /// These bits and those of `other`.
    #[must_use]
    pub const fn with(self, other: Flags) -> Flags {
        Flags(self.0 | other.0)
    }

    /// These bits without those of `other`.
    #[must_use]
    pub const fn without(self, other: Flags) -> Flags {
        Flags(self.0 & !other.0)
    }
}

impl fmt::Display for Flags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (bit, letter) in [
            (Flags::URG, 'U'),
            (Flags::ACK, 'A'),
            (Flags::PSH, 'P'),
            (Flags::RST, 'R'),
            (Flags::SYN, 'S'),
            (Flags::FIN, 'F'),
        ] {
            write!(f, "{}", if self.has(bit) { letter } else { '.' })?;
        }
        Ok(())
    }
}

/// One segment, borrowed from the bytes it arrived in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Segment<'a> {
    /// Where it came from.
    pub source_port: Port,
    /// Where it is going.
    pub destination_port: Port,
    /// The number of its first byte, or of its `SYN`.
    pub seq: SeqNumber,
    /// What the sender has received, when [`Flags::ACK`] is set.
    pub ack: SeqNumber,
    /// Its control bits.
    pub flags: Flags,
    /// How much room the sender has left, in bytes.
    pub window: u16,
    /// The largest segment the sender will accept, when it announced one.
    /// Only a segment with `SYN` may.
    pub max_segment_size: Option<u16>,
    /// What it carries.
    pub payload: &'a [u8],
}

impl<'a> Segment<'a> {
    /// A segment with nothing in it but its ports and its bits, which is
    /// what every segment this crate builds starts from.
    #[must_use]
    pub const fn new(source_port: Port, destination_port: Port, flags: Flags) -> Segment<'a> {
        Segment {
            source_port,
            destination_port,
            seq: SeqNumber::new(0),
            ack: SeqNumber::new(0),
            flags,
            window: 0,
            max_segment_size: None,
            payload: &[],
        }
    }

    /// How many sequence numbers the segment takes up: its payload, plus
    /// one for `SYN` and one for `FIN`, each of which occupies a number of
    /// its own (RFC 9293, section 3.4).
    #[must_use]
    pub fn sequence_len(&self) -> u32 {
        let payload = u32::try_from(self.payload.len()).unwrap_or(u32::MAX);
        let syn = u32::from(self.flags.has(Flags::SYN));
        let fin = u32::from(self.flags.has(Flags::FIN));
        payload.saturating_add(syn).saturating_add(fin)
    }

    /// The number one past the last this segment occupies.
    #[must_use]
    pub fn sequence_end(&self) -> SeqNumber {
        self.seq.add(self.sequence_len())
    }

    /// How many bytes the segment takes on the wire.
    #[must_use]
    pub fn wire_len(&self) -> usize {
        self.header_len().saturating_add(self.payload.len())
    }

    /// How long its header is, which is twenty bytes and four more when it
    /// announces a maximum segment size.
    fn header_len(&self) -> usize {
        match self.max_segment_size {
            Some(_) => HEADER_LEN.saturating_add(usize::from(option::MAX_SEGMENT_SIZE_LEN)),
            None => HEADER_LEN,
        }
    }

    /// The segment in `bytes`, which arrived from `source` and was
    /// addressed to `destination`.
    ///
    /// The two addresses are not in the segment; they come from the header
    /// below it and are needed to sum the pseudo-header.
    ///
    /// # Errors
    ///
    /// [`TcpError::MixedFamilies`] when the two addresses are not of one
    /// family, [`TcpError::Short`] when there is not even a header,
    /// [`TcpError::DataOffset`] when the offset field names no header this
    /// segment has, [`TcpError::Checksum`] when the sum does not come out,
    /// and [`TcpError::BadOption`] for an option list that is not one.
    pub fn parse(
        bytes: &'a [u8],
        source: IpAddr,
        destination: IpAddr,
    ) -> Result<Segment<'a>, TcpError> {
        if source.version() != destination.version() {
            return Err(TcpError::MixedFamilies);
        }
        if bytes.len() < HEADER_LEN {
            return Err(TcpError::Short(bytes.len()));
        }
        let mut reader = Reader::new(bytes);
        let source_port = reader.read_port()?;
        let destination_port = reader.read_port()?;
        let seq = SeqNumber::new(reader.read_u32()?);
        let ack = SeqNumber::new(reader.read_u32()?);
        let offset_and_reserved = reader.read_u8()?;
        let flags = Flags::new(reader.read_u8()?);
        let window = reader.read_u16()?;
        let carried = reader.read_u16()?;
        let _urgent = reader.read_u16()?;

        let words = offset_and_reserved >> 4;
        if !(MIN_DATA_OFFSET..=MAX_DATA_OFFSET).contains(&words) {
            return Err(TcpError::DataOffset(words));
        }
        let header_len = usize::from(words).saturating_mul(4);
        let Some(header) = bytes.get(..header_len) else {
            return Err(TcpError::DataOffset(words));
        };
        if transport(source, destination, Protocol::TCP, bytes)? != 0 {
            return Err(TcpError::Checksum(carried));
        }
        let options = header.get(HEADER_LEN..).unwrap_or(&[]);
        // The list is walked whether or not the value is kept, because an
        // option list that is not one is a reason to refuse the segment
        // and not only a reason to ignore a value.
        let announced = read_max_segment_size(options)?;
        let max_segment_size = if flags.has(Flags::SYN) {
            announced
        } else {
            None
        };
        Ok(Segment {
            source_port,
            destination_port,
            seq,
            ack,
            flags,
            window,
            max_segment_size,
            // The header ends inside the segment, checked above.
            payload: bytes.get(header_len..).unwrap_or(&[]),
        })
    }

    /// Writes the segment from `source` to `destination`.
    ///
    /// Either the whole segment is written or none of it: the room is
    /// checked before the first byte goes down.
    ///
    /// # Errors
    ///
    /// [`TcpError::MixedFamilies`] when the two addresses are not of one
    /// family, and [`TcpError::Wire`] when the buffer has no room.
    pub fn write(
        &self,
        writer: &mut Writer<'_>,
        source: IpAddr,
        destination: IpAddr,
    ) -> Result<(), TcpError> {
        if source.version() != destination.version() {
            return Err(TcpError::MixedFamilies);
        }
        let needed = self.wire_len();
        if writer.remaining() < needed {
            return Err(TcpError::Wire(WireError::OutOfBounds {
                needed,
                available: writer.remaining(),
            }));
        }
        let at = writer.position();
        let words = u8::try_from(self.header_len().saturating_div(4)).unwrap_or(MIN_DATA_OFFSET);
        writer.write_port(self.source_port)?;
        writer.write_port(self.destination_port)?;
        writer.write_u32(self.seq.get())?;
        writer.write_u32(self.ack.get())?;
        writer.write_u8(words << 4)?;
        writer.write_u8(self.flags.bits())?;
        writer.write_u16(self.window)?;
        writer.write_u16(0)?;
        writer.write_u16(0)?;
        if let Some(size) = self.max_segment_size {
            writer.write_u8(option::MAX_SEGMENT_SIZE)?;
            writer.write_u8(option::MAX_SEGMENT_SIZE_LEN)?;
            writer.write_u16(size)?;
        }
        writer.write_bytes(self.payload)?;
        // Everything from `at` on is what was just written, and its
        // checksum field is still zero.
        let segment = writer.written().get(at..).unwrap_or(&[]);
        let sum = transport(source, destination, Protocol::TCP, segment)?;
        writer.patch_u16(at.saturating_add(CHECKSUM_OFFSET), sum)?;
        Ok(())
    }
}

/// The reset that answers a segment nobody wants, or `None` when the
/// segment is itself a reset.
///
/// RFC 9293, section 3.10.7.1: a reset never answers a reset, or the two
/// ends would trade them for ever. Which numbers it carries depends on
/// whether the segment acknowledged anything — an acknowledgment names a
/// number the peer believes this end sent, so the reset carries that one
/// and nothing else; without one there is no such number, so the reset
/// acknowledges everything the segment occupied instead.
#[must_use]
pub fn reset_for(segment: &Segment<'_>) -> Option<Segment<'static>> {
    if segment.flags.has(Flags::RST) {
        return None;
    }
    let mut answer = Segment::new(segment.destination_port, segment.source_port, Flags::RST);
    if segment.flags.has(Flags::ACK) {
        answer.seq = segment.ack;
    } else {
        answer.flags = Flags::RST.with(Flags::ACK);
        answer.ack = segment.sequence_end();
    }
    Some(answer)
}

/// Walks the option list and answers the maximum segment size it
/// announces, if it announces one.
///
/// # Errors
///
/// [`TcpError::BadOption`] for a length below the two bytes an option
/// takes, or one that reaches past the list.
fn read_max_segment_size(options: &[u8]) -> Result<Option<u16>, TcpError> {
    let mut rest = options;
    let mut found = None;
    while let Some((&kind, tail)) = rest.split_first() {
        match kind {
            option::END => return Ok(found),
            option::NOP => {
                rest = tail;
                continue;
            }
            _ => {}
        }
        let Some(&length) = tail.first() else {
            return Err(TcpError::BadOption(kind));
        };
        if length < option::MIN_LEN {
            return Err(TcpError::BadOption(kind));
        }
        let Some(body) = rest.get(..usize::from(length)) else {
            return Err(TcpError::BadOption(kind));
        };
        if kind == option::MAX_SEGMENT_SIZE && length == option::MAX_SEGMENT_SIZE_LEN {
            let value = body.get(2..4).and_then(|bytes| bytes.first_chunk::<2>());
            found = value.map(|bytes| u16::from_be_bytes(*bytes));
        }
        rest = rest.get(usize::from(length)..).unwrap_or(&[]);
    }
    Ok(found)
}
