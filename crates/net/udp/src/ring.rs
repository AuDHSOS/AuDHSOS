// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The receive ring: datagrams held as self-describing records in memory
//! the caller supplied.
//!
//! A record carries the two addresses the datagram arrived between, the
//! port it came from, and its payload. It carries them because a socket
//! bound to the unspecified address serves every address of the host, and
//! a reply that leaves from the wrong one is a reply the peer discards.
//!
//! A record is never cut in two. When one no longer fits behind the last,
//! the end of the used region is marked and the record begins at the front
//! of the buffer instead, which is what lets [`Ring::peek`] hand out a
//! payload as one slice rather than as two. The mark is dropped again as
//! soon as the reader has walked past it.
//!
//! A ring with no room drops the newest datagram and counts it. The other
//! choice — dropping the oldest to make room — would serve a reader that
//! is behind by handing it the newest datagrams out of order, and a
//! protocol above that expects a stream of what arrived would have to
//! detect that itself.

use net_wire::{IpAddr, IpVersion, Port, Reader, Writer};

/// The tag, the payload length, and the source port, which every record
/// begins with whatever family it holds.
const PREFIX_LEN: usize = 5;

/// The tag of a record between two IPv4 addresses.
const TAG_V4: u8 = 4;

/// The tag of a record between two IPv6 addresses.
const TAG_V6: u8 = 6;

/// How many bytes an address of `version` takes in a record.
const fn address_len(version: IpVersion) -> usize {
    match version {
        IpVersion::V4 => 4,
        IpVersion::V6 => 16,
    }
}

/// How much room a datagram of `payload_len` bytes between two addresses
/// of `version` takes in a ring.
///
/// This is what a caller sizes a buffer with: a ring holds a datagram when
/// its own length is at least this much, and holds several when it is a
/// multiple of it.
#[must_use]
pub const fn record_len(version: IpVersion, payload_len: usize) -> usize {
    PREFIX_LEN
        .saturating_add(address_len(version).saturating_mul(2))
        .saturating_add(payload_len)
}

/// Why a datagram did not go into the ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DropReason {
    /// There is not enough free room at the moment. A reader that catches
    /// up makes room.
    Full,
    /// The datagram would not fit an empty ring either, or its payload is
    /// longer than the length field of a record.
    TooLarge,
    /// The two addresses are not of one family, so there is no record
    /// shape that holds them.
    MixedFamilies,
}

/// One datagram taken out of a ring, borrowed from the ring's memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Received<'a> {
    /// Which host it came from.
    pub source: IpAddr,
    /// Which of this host's addresses it was addressed to.
    pub destination: IpAddr,
    /// Which port it came from.
    pub port: Port,
    /// What it carries.
    pub payload: &'a [u8],
}

/// A queue of received datagrams over a buffer the caller owns.
#[derive(Debug)]
pub struct Ring<'a> {
    /// The memory the records lie in.
    bytes: &'a mut [u8],
    /// Where the oldest record begins.
    head: usize,
    /// Where the next record goes.
    tail: usize,
    /// Where the used region ends while the ring is wrapped. Meaningless
    /// otherwise, and reset to the length of the buffer.
    limit: usize,
    /// Whether the records run past the end of the buffer and continue at
    /// its front.
    wrapped: bool,
    /// How many records are in it.
    count: usize,
    /// How many datagrams it refused.
    dropped: u32,
}

impl<'a> Ring<'a> {
    /// An empty ring over `bytes`.
    #[must_use]
    pub const fn new(bytes: &'a mut [u8]) -> Ring<'a> {
        let limit = bytes.len();
        Ring {
            bytes,
            head: 0,
            tail: 0,
            limit,
            wrapped: false,
            count: 0,
            dropped: 0,
        }
    }

    /// Gives the memory back, which is what closing a socket does.
    #[must_use]
    pub const fn into_bytes(self) -> &'a mut [u8] {
        self.bytes
    }

    /// How many bytes the ring has in all, records and their headers
    /// together.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.bytes.len()
    }

    /// How many datagrams are waiting.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.count
    }

    /// Whether none are.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// How many datagrams the ring refused since it was made.
    #[must_use]
    pub const fn dropped(&self) -> u32 {
        self.dropped
    }

    /// The oldest datagram, without taking it out.
    ///
    /// A caller reads it here and then calls [`discard`](Ring::discard),
    /// which is what keeps the payload a borrow into the ring rather than
    /// a copy.
    #[must_use]
    pub fn peek(&self) -> Option<Received<'_>> {
        self.front().map(|(received, _)| received)
    }

    /// Drops the oldest datagram and answers whether there was one.
    pub fn discard(&mut self) -> bool {
        let Some((_, len)) = self.front() else {
            return false;
        };
        self.head = self.head.saturating_add(len);
        self.count = self.count.saturating_sub(1);
        if self.wrapped && self.head >= self.limit {
            self.head = 0;
            self.wrapped = false;
            self.limit = self.bytes.len();
        }
        if self.count == 0 {
            self.head = 0;
            self.tail = 0;
            self.wrapped = false;
            self.limit = self.bytes.len();
        }
        true
    }

    /// Puts a datagram in.
    ///
    /// # Errors
    ///
    /// [`DropReason::MixedFamilies`] when the two addresses are not of one
    /// family, [`DropReason::TooLarge`] when the record would not fit an
    /// empty ring, and [`DropReason::Full`] when it does not fit this one
    /// now. Every one of them counts towards
    /// [`dropped`](Ring::dropped).
    pub fn push(
        &mut self,
        source: IpAddr,
        destination: IpAddr,
        port: Port,
        payload: &[u8],
    ) -> Result<(), DropReason> {
        if source.version() != destination.version() {
            return Err(self.refuse(DropReason::MixedFamilies));
        }
        let len = record_len(source.version(), payload.len());
        if len > self.bytes.len() || u16::try_from(payload.len()).is_err() {
            return Err(self.refuse(DropReason::TooLarge));
        }
        let Some((at, wrap)) = self.room_for(len) else {
            return Err(self.refuse(DropReason::Full));
        };
        let end = at.saturating_add(len);
        let written = self
            .bytes
            .get_mut(at..end)
            .and_then(|slot| write_record(slot, source, destination, port, payload));
        if written.is_none() {
            return Err(self.refuse(DropReason::Full));
        }
        if wrap {
            self.limit = self.tail;
            self.wrapped = true;
        }
        self.tail = end;
        self.count = self.count.saturating_add(1);
        Ok(())
    }

    /// The oldest record and how many bytes it takes.
    fn front(&self) -> Option<(Received<'_>, usize)> {
        if self.count == 0 {
            return None;
        }
        // `head` never leaves the buffer, and the record it points at is
        // whole, because a record is never written across the end.
        read_record(self.bytes.get(self.head..).unwrap_or(&[]))
    }

    /// Where a record of `len` bytes goes, and whether putting it there
    /// wraps the ring; `None` when it does not fit.
    const fn room_for(&self, len: usize) -> Option<(usize, bool)> {
        if self.wrapped {
            if len <= self.head.saturating_sub(self.tail) {
                Some((self.tail, false))
            } else {
                None
            }
        } else if len <= self.bytes.len().saturating_sub(self.tail) {
            Some((self.tail, false))
        } else if len <= self.head {
            Some((0, true))
        } else {
            None
        }
    }

    /// Counts a refused datagram and hands the reason back.
    const fn refuse(&mut self, reason: DropReason) -> DropReason {
        self.dropped = self.dropped.saturating_add(1);
        reason
    }
}

/// Writes one record into `slot`, or answers `None` when it does not fit.
pub(crate) fn write_record(
    slot: &mut [u8],
    source: IpAddr,
    destination: IpAddr,
    port: Port,
    payload: &[u8],
) -> Option<()> {
    let tag = match source.version() {
        IpVersion::V4 => TAG_V4,
        IpVersion::V6 => TAG_V6,
    };
    let payload_len = u16::try_from(payload.len()).ok()?;
    let mut writer = Writer::new(slot);
    writer.write_u8(tag).ok()?;
    writer.write_u16(payload_len).ok()?;
    writer.write_port(port).ok()?;
    writer.write_ip(source).ok()?;
    writer.write_ip(destination).ok()?;
    writer.write_bytes(payload).ok()?;
    Some(())
}

/// Reads the record at the front of `bytes`, and how many bytes it takes.
pub(crate) fn read_record(bytes: &[u8]) -> Option<(Received<'_>, usize)> {
    let mut reader = Reader::new(bytes);
    let tag = reader.read_u8().ok()?;
    let payload_len = usize::from(reader.read_u16().ok()?);
    let port = reader.read_port().ok()?;
    let (source, destination) = match tag {
        TAG_V4 => (
            IpAddr::V4(reader.read_ipv4().ok()?),
            IpAddr::V4(reader.read_ipv4().ok()?),
        ),
        TAG_V6 => (
            IpAddr::V6(reader.read_ipv6().ok()?),
            IpAddr::V6(reader.read_ipv6().ok()?),
        ),
        _ => return None,
    };
    let payload = reader.read_bytes(payload_len).ok()?;
    Some((
        Received {
            source,
            destination,
            port,
            payload,
        },
        reader.position(),
    ))
}
