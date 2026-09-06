// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The resource record of RFC 1035, section 4.1.3, and the three kinds of
//! body this crate reads.
//!
//! `A` (RFC 1035, section 3.4.1) and `AAAA` (RFC 3596, section 2.2) are
//! decoded to addresses and `CNAME` (RFC 1035, section 3.3.1) to the name
//! it points at. Everything else is carried as the bytes of its body: a
//! record this resolver has no use for is one it must still walk past
//! correctly, because the answer it wants may stand behind it.
//!
//! A record of a class other than `IN` is carried the same way, whatever
//! its type says. Four bytes in the Chaos class are not an internet
//! address, and reading them as one would be reading a field of another
//! protocol.

use net_wire::{Ipv4Addr, Ipv6Addr, WireError, Writer};

use crate::error::DnsError;
use crate::name::Name;

/// Which kind of record. The values are those of RFC 1035, section 3.2.2
/// and, for `AAAA`, of RFC 3596, section 2.1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RecordType(u16);

impl RecordType {
    /// A host address (RFC 1035, section 3.4.1).
    pub const A: RecordType = RecordType(1);
    /// An authoritative name server.
    pub const NS: RecordType = RecordType(2);
    /// The canonical name of an alias (RFC 1035, section 3.3.1).
    pub const CNAME: RecordType = RecordType(5);
    /// The start of a zone of authority.
    pub const SOA: RecordType = RecordType(6);
    /// Text.
    pub const TXT: RecordType = RecordType(16);
    /// An IPv6 host address (RFC 3596, section 2.1).
    pub const AAAA: RecordType = RecordType(28);

    /// The type `value` names.
    #[must_use]
    pub const fn new(value: u16) -> RecordType {
        RecordType(value)
    }

    /// The number as it goes on the wire.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// Which class. Only `IN` has a meaning here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Class(u16);

impl Class {
    /// The internet class (RFC 1035, section 3.2.4).
    pub const IN: Class = Class(1);

    /// The class `value` names.
    #[must_use]
    pub const fn new(value: u16) -> Class {
        Class(value)
    }

    /// The number as it goes on the wire.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// What a record carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[expect(
    clippy::large_enum_variant,
    reason = "a name is 255 bytes by RFC 1035, section 2.3.4, and there is no allocator to put one behind"
)]
pub enum RecordData<'a> {
    /// An IPv4 address.
    A(Ipv4Addr),
    /// An IPv6 address.
    Aaaa(Ipv6Addr),
    /// The name this one is an alias for.
    Cname(Name),
    /// A body of a type or class this crate does not read.
    Other(&'a [u8]),
}

impl RecordData<'_> {
    /// How many bytes the body takes on the wire.
    #[must_use]
    pub fn wire_len(&self) -> usize {
        match self {
            RecordData::A(_) => Ipv4Addr::LEN,
            RecordData::Aaaa(_) => Ipv6Addr::LEN,
            RecordData::Cname(name) => name.as_bytes().len(),
            RecordData::Other(bytes) => bytes.len(),
        }
    }
}

/// One resource record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Record<'a> {
    /// Whose record it is.
    pub name: Name,
    /// Which kind.
    pub record_type: RecordType,
    /// Which class.
    pub class: Class,
    /// How long it may be believed, in seconds.
    pub ttl: u32,
    /// What it carries.
    pub data: RecordData<'a>,
}

/// The fixed part behind the name: type, class, time to live, and the
/// length of the body.
const FIXED_LEN: usize = 10;

impl<'a> Record<'a> {
    /// The record that begins at `at` in `message`, and the offset just
    /// past it.
    ///
    /// # Errors
    ///
    /// Whatever [`Name::read`] returns for the owner name and for the name
    /// inside a `CNAME`; [`DnsError::Wire`] when the record reaches past
    /// the message; and [`DnsError::Rdata`] when the body is not the
    /// length its type has, or when a `CNAME` does not end where its body
    /// does.
    pub fn read(message: &'a [u8], at: usize) -> Result<(Record<'a>, usize), DnsError> {
        let (name, after_name) = Name::read(message, at)?;
        let fixed_end = after_name.saturating_add(FIXED_LEN);
        let fixed = message
            .get(after_name..)
            .and_then(<[u8]>::first_chunk::<FIXED_LEN>)
            .ok_or_else(|| out_of_bounds(message, fixed_end))?;
        let [
            type_high,
            type_low,
            class_high,
            class_low,
            ttl_a,
            ttl_b,
            ttl_c,
            ttl_d,
            length_high,
            length_low,
        ] = *fixed;
        let record_type = RecordType(u16::from_be_bytes([type_high, type_low]));
        let class = Class(u16::from_be_bytes([class_high, class_low]));
        let ttl = u32::from_be_bytes([ttl_a, ttl_b, ttl_c, ttl_d]);
        let len = usize::from(u16::from_be_bytes([length_high, length_low]));
        let end = fixed_end.saturating_add(len);
        let body = message
            .get(fixed_end..end)
            .ok_or_else(|| out_of_bounds(message, end))?;
        let data = read_data(message, record_type, class, fixed_end, body)?;
        Ok((
            Record {
                name,
                record_type,
                class,
                ttl,
                data,
            },
            end,
        ))
    }

    /// Writes the record, with the name uncompressed.
    ///
    /// # Errors
    ///
    /// [`DnsError::Wire`] when the buffer has no room, and
    /// [`DnsError::Rdata`] when the body is longer than the length field
    /// can express.
    pub fn write(&self, writer: &mut Writer<'_>) -> Result<(), DnsError> {
        self.name.write(writer)?;
        writer.write_u16(self.record_type.0)?;
        writer.write_u16(self.class.0)?;
        writer.write_u32(self.ttl)?;
        let len = self.data.wire_len();
        let Ok(field) = u16::try_from(len) else {
            return Err(DnsError::Rdata {
                record_type: self.record_type,
                len,
            });
        };
        writer.write_u16(field)?;
        match &self.data {
            RecordData::A(address) => writer.write_ipv4(*address)?,
            RecordData::Aaaa(address) => writer.write_ipv6(*address)?,
            RecordData::Cname(name) => name.write(writer)?,
            RecordData::Other(bytes) => writer.write_bytes(bytes)?,
        }
        Ok(())
    }
}

/// The body of a record, decoded where the type and the class say what it
/// is and carried whole where they do not.
fn read_data<'a>(
    message: &'a [u8],
    record_type: RecordType,
    class: Class,
    at: usize,
    body: &'a [u8],
) -> Result<RecordData<'a>, DnsError> {
    if class != Class::IN {
        return Ok(RecordData::Other(body));
    }
    match record_type {
        RecordType::A => match body.first_chunk::<{ Ipv4Addr::LEN }>() {
            Some(octets) if body.len() == Ipv4Addr::LEN => {
                Ok(RecordData::A(Ipv4Addr::from_octets(*octets)))
            }
            _ => Err(DnsError::Rdata {
                record_type,
                len: body.len(),
            }),
        },
        RecordType::AAAA => match body.first_chunk::<{ Ipv6Addr::LEN }>() {
            Some(octets) if body.len() == Ipv6Addr::LEN => {
                Ok(RecordData::Aaaa(Ipv6Addr::from_octets(*octets)))
            }
            _ => Err(DnsError::Rdata {
                record_type,
                len: body.len(),
            }),
        },
        RecordType::CNAME => {
            // The name is read against the whole message, because a
            // pointer inside a body points into it; where it ends has to
            // be exactly where the body ends, or the length field and the
            // name disagree about the same bytes.
            let (name, after) = Name::read(message, at)?;
            if after == at.saturating_add(body.len()) {
                Ok(RecordData::Cname(name))
            } else {
                Err(DnsError::Rdata {
                    record_type,
                    len: body.len(),
                })
            }
        }
        _ => Ok(RecordData::Other(body)),
    }
}

/// The error for a record that reaches past the message.
const fn out_of_bounds(message: &[u8], needed: usize) -> DnsError {
    DnsError::Wire(WireError::OutOfBounds {
        needed,
        available: message.len(),
    })
}
