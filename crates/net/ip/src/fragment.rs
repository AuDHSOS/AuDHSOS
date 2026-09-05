// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Cutting a datagram up on the way out, and putting one back together on
//! the way in, per RFC 791, section 3.2.
//!
//! The offset field counts in units of eight bytes, so every fragment but
//! the last carries a multiple of eight. That is the whole of the
//! arithmetic; what makes reassembly worth its own module is what happens
//! when the pieces do not agree.
//!
//! An overlap discards the datagram. RFC 791 leaves the case open — it
//! says nothing about two fragments that claim the same bytes and carry
//! different ones — and every answer but discarding lets a filter be
//! walked past: a first fragment shows one transport header, a second
//! overwrites it, and what the filter read is not what the host
//! reassembles. Discarding costs a retransmission and closes that.
//!
//! Time is an argument. A datagram whose pieces stop arriving is dropped
//! at its deadline, and the buffer it held goes back.

use audhsos_collections::ArrayVec;
use audhsos_time::{Duration, Instant};
use net_wire::{Ipv4Addr, Protocol, Writer};

use crate::error::IpError;
use crate::header::{Datagram, FRAGMENT_UNIT, Header, MIN_HEADER_LEN};

/// How long a half-assembled datagram is kept. RFC 1122, section 3.3.2
/// asks for at least sixty seconds, and this is that.
pub const REASSEMBLY_TIMEOUT: Duration = Duration::from_secs(60);

/// What a datagram is cut into for an interface of `mtu` bytes.
///
/// The iterator yields one piece at a time as an offset into the payload
/// and its length, which is what a caller needs to write the header and
/// copy the bytes. It exists so that fragmentation allocates nothing: the
/// caller owns the buffer each piece is written into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fragments {
    /// How much payload a piece may carry, a multiple of eight.
    per_fragment: usize,
    /// How long the payload is.
    payload_len: usize,
    /// Where the next piece begins.
    offset: usize,
}

impl Fragments {
    /// How a payload of `payload_len` bytes is cut for `mtu`.
    ///
    /// # Errors
    ///
    /// [`IpError::WouldFragment`] when the MTU has no room for a header
    /// and at least eight bytes behind it.
    pub const fn new(payload_len: usize, mtu: usize) -> Result<Fragments, IpError> {
        let too_small = Err(IpError::WouldFragment {
            length: payload_len,
            mtu,
        });
        let Some(room) = mtu.checked_sub(MIN_HEADER_LEN) else {
            return too_small;
        };
        // Round down to the eight-byte unit the offset field counts in.
        let per_fragment = room & !(FRAGMENT_UNIT - 1);
        if per_fragment == 0 {
            return too_small;
        }
        Ok(Fragments {
            per_fragment,
            payload_len,
            offset: 0,
        })
    }

    /// How many pieces there will be.
    #[must_use]
    pub const fn count(&self) -> usize {
        if self.payload_len == 0 {
            return 1;
        }
        self.payload_len.div_ceil(self.per_fragment)
    }

    /// Whether the datagram fits in one piece.
    #[must_use]
    pub const fn is_whole(&self) -> bool {
        self.payload_len <= self.per_fragment
    }
}

impl Fragments {
    /// The next piece: where it begins in the payload, how long it is, and
    /// whether another follows.
    ///
    /// This is a `next` and not an [`Iterator`], because a `Fragments` is
    /// `Copy` and an iterator that is copied silently starts again — the
    /// one shape of this type that would surprise a reader.
    pub const fn next(&mut self) -> Option<(usize, usize, bool)> {
        if self.offset > self.payload_len {
            return None;
        }
        if self.offset == self.payload_len && self.offset > 0 {
            return None;
        }
        let remaining = self.payload_len.saturating_sub(self.offset);
        let length = if remaining < self.per_fragment {
            remaining
        } else {
            self.per_fragment
        };
        let more = remaining > length;
        let at = self.offset;
        // An empty payload still makes one piece, so the walk advances by
        // at least one byte and ends.
        let step = if length == 0 { 1 } else { length };
        self.offset = self.offset.saturating_add(step);
        Some((at, length, more))
    }
}

/// Writes one datagram, cut into as many pieces as `mtu` requires.
///
/// Each piece is handed to `emit` as a complete datagram in the buffer the
/// caller supplied. The don't-fragment bit is honored: a payload that does
/// not fit and may not be cut is an error and not a truncation.
///
/// # Errors
///
/// [`IpError::WouldFragment`] when the datagram is too long for the MTU
/// and carries the don't-fragment bit, and whatever `emit` returns.
pub fn fragment<E>(
    header: &Header,
    payload: &[u8],
    mtu: usize,
    buffer: &mut [u8],
    mut emit: E,
) -> Result<usize, IpError>
where
    E: FnMut(&[u8]) -> Result<(), IpError>,
{
    let pieces = Fragments::new(payload.len(), mtu)?;
    if header.dont_fragment && !pieces.is_whole() {
        return Err(IpError::WouldFragment {
            length: payload.len(),
            mtu,
        });
    }
    let mut written = 0usize;
    let mut pieces = pieces;
    while let Some((at, length, more)) = pieces.next() {
        let piece = payload
            .get(at..at.saturating_add(length))
            .ok_or(IpError::TooLarge(payload.len()))?;
        let mut writer = Writer::new(buffer);
        let mut piece_header = *header;
        piece_header.payload_len = length;
        piece_header.more_fragments = more;
        piece_header.fragment_offset = at;
        piece_header.write(&mut writer)?;
        writer.write_bytes(piece)?;
        emit(writer.written())?;
        written = written.saturating_add(1);
    }
    Ok(written)
}

/// One datagram being put back together.
#[derive(Debug)]
struct Buffer<const BYTES: usize> {
    /// Which datagram this is: the four fields RFC 791 says identify one.
    key: Key,
    /// When it is given up on.
    deadline: Instant,
    /// The bytes so far.
    data: [u8; BYTES],
    /// Which bytes have arrived, as a list of the ranges that are filled.
    /// A datagram of 65535 bytes arrives in at most 45 pieces over an
    /// Ethernet, and a sender that produces more than this holds is one
    /// this host declines to reassemble for.
    filled: ArrayVec<(usize, usize), 64>,
    /// How long the whole datagram is, once the last piece has arrived.
    total: Option<usize>,
    /// The header of the first piece, which is the one the reassembled
    /// datagram keeps.
    header: [u8; MIN_HEADER_LEN],
    /// Whether that header has arrived.
    have_header: bool,
}

/// What names one datagram, per RFC 791, section 3.2.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Key {
    /// Where it came from.
    source: Ipv4Addr,
    /// Where it goes.
    destination: Ipv4Addr,
    /// What it carries.
    protocol: Protocol,
    /// Which datagram of that pair it is.
    identification: u16,
}

/// The reassembly buffers.
///
/// `SLOTS` datagrams at a time, each up to `BYTES` long. A new datagram
/// evicts the one closest to its deadline when every slot is busy, which
/// is the oldest — a datagram that has been waiting longest is the one
/// least likely to complete.
#[derive(Debug)]
pub struct Reassembler<const SLOTS: usize, const BYTES: usize> {
    /// The buffers in use.
    buffers: ArrayVec<Buffer<BYTES>, SLOTS>,
    /// How long a datagram is kept.
    timeout: Duration,
}

impl<const SLOTS: usize, const BYTES: usize> Default for Reassembler<SLOTS, BYTES> {
    fn default() -> Reassembler<SLOTS, BYTES> {
        Reassembler::new()
    }
}

impl<const SLOTS: usize, const BYTES: usize> Reassembler<SLOTS, BYTES> {
    /// An empty reassembler on the timeout of RFC 1122.
    #[must_use]
    pub const fn new() -> Reassembler<SLOTS, BYTES> {
        Reassembler {
            buffers: ArrayVec::new(),
            timeout: REASSEMBLY_TIMEOUT,
        }
    }

    /// An empty reassembler that gives up after `timeout`.
    #[must_use]
    pub const fn with_timeout(timeout: Duration) -> Reassembler<SLOTS, BYTES> {
        Reassembler {
            buffers: ArrayVec::new(),
            timeout,
        }
    }

    /// How many datagrams are half assembled.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.buffers.len()
    }

    /// Whether none are.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.buffers.is_empty()
    }

    /// When the next half-assembled datagram is given up on.
    #[must_use]
    pub fn poll_at(&self) -> Option<Instant> {
        self.buffers.iter().map(|buffer| buffer.deadline).min()
    }

    /// Drops every datagram whose deadline has passed, and answers how
    /// many went.
    pub fn poll(&mut self, now: Instant) -> usize {
        let before = self.buffers.len();
        let mut index = 0;
        while index < self.buffers.len() {
            let expired = self
                .buffers
                .get(index)
                .is_some_and(|buffer| buffer.deadline <= now);
            if expired {
                self.buffers.remove(index);
            } else {
                index = index.saturating_add(1);
            }
        }
        before.saturating_sub(self.buffers.len())
    }

    /// Takes one fragment in, and answers the whole datagram when this
    /// piece completed it.
    ///
    /// A datagram that is not a fragment is not copied at all: the answer
    /// borrows the input, so the common case costs nothing.
    ///
    /// # Errors
    ///
    /// [`IpError::OverlappingFragment`] when two pieces claim the same
    /// bytes and disagree, [`IpError::TooLarge`] when the datagram would
    /// not fit `BYTES`, and [`IpError::NoRoute`] when there is no slot and
    /// nothing to evict, which is a reassembler of no slots at all.
    pub fn accept<'a>(
        &'a mut self,
        datagram: Datagram<'a>,
        now: Instant,
    ) -> Result<Option<Assembled<'a>>, IpError> {
        if !datagram.is_fragment() {
            return Ok(Some(Assembled::Whole(datagram)));
        }
        let key = Key {
            source: datagram.source(),
            destination: datagram.destination(),
            protocol: datagram.protocol(),
            identification: datagram.identification(),
        };
        let at = datagram.fragment_offset();
        let end = at.saturating_add(datagram.payload().len());
        if end > BYTES {
            return Err(IpError::TooLarge(end));
        }
        let deadline = now.saturating_add(self.timeout);
        let index = self.slot_for(key, deadline)?;
        let buffer = self.buffers.get_mut(index).ok_or(IpError::NoRoute)?;
        buffer.insert(datagram, at, end)?;
        if !buffer.is_complete() {
            return Ok(None);
        }
        let total = buffer.total.unwrap_or(0);
        let header = buffer.header;
        let data = buffer.data.get(..total).ok_or(IpError::TooLarge(total))?;
        Ok(Some(Assembled::Reassembled { header, data }))
    }

    /// Forgets the datagram this fragment belongs to, which a caller does
    /// once it has read a reassembled one out.
    pub fn release(&mut self, datagram: Datagram<'_>) {
        let key = Key {
            source: datagram.source(),
            destination: datagram.destination(),
            protocol: datagram.protocol(),
            identification: datagram.identification(),
        };
        if let Some((index, _)) = self
            .buffers
            .iter()
            .enumerate()
            .find(|(_, buffer)| buffer.key == key)
        {
            self.buffers.remove(index);
        }
    }

    /// The slot for `key`, making one if there is none.
    fn slot_for(&mut self, key: Key, deadline: Instant) -> Result<usize, IpError> {
        if let Some((index, _)) = self
            .buffers
            .iter()
            .enumerate()
            .find(|(_, buffer)| buffer.key == key)
        {
            return Ok(index);
        }
        if self.buffers.is_full() {
            let (index, _) = self
                .buffers
                .iter()
                .enumerate()
                .min_by_key(|(_, buffer)| buffer.deadline)
                .ok_or(IpError::NoRoute)?;
            self.buffers.remove(index);
        }
        let buffer = Buffer {
            key,
            deadline,
            data: [0; BYTES],
            filled: ArrayVec::new(),
            total: None,
            header: [0; MIN_HEADER_LEN],
            have_header: false,
        };
        self.buffers.push(buffer).map_err(|_| IpError::NoRoute)?;
        Ok(self.buffers.len().saturating_sub(1))
    }
}

/// What came out of the reassembler.
#[derive(Clone, Copy, Debug)]
pub enum Assembled<'a> {
    /// The datagram was never fragmented and is borrowed where it landed.
    Whole(Datagram<'a>),
    /// The datagram was put back together in the reassembler's buffer.
    Reassembled {
        /// The header of the first piece, twenty bytes without options.
        header: [u8; MIN_HEADER_LEN],
        /// The payload, whole.
        data: &'a [u8],
    },
}

impl<'a> Assembled<'a> {
    /// The payload of the datagram, whichever way it arrived.
    #[must_use]
    pub const fn payload(&self) -> &'a [u8] {
        match self {
            Assembled::Whole(datagram) => datagram.payload(),
            Assembled::Reassembled { data, .. } => data,
        }
    }
}

impl<const BYTES: usize> Buffer<BYTES> {
    /// Puts one fragment's bytes in.
    fn insert(&mut self, datagram: Datagram<'_>, at: usize, end: usize) -> Result<(), IpError> {
        for (start, stop) in self.filled.iter().copied() {
            if at < stop && start < end {
                // They touch. Identical bytes are a duplicate and are
                // dropped; anything else is an overlap and the datagram
                // goes.
                let same = at == start
                    && end == stop
                    && self.data.get(at..end) == Some(datagram.payload());
                if same {
                    return Ok(());
                }
                return Err(IpError::OverlappingFragment);
            }
        }
        let room = self.data.get_mut(at..end).ok_or(IpError::TooLarge(end))?;
        room.copy_from_slice(datagram.payload());
        self.filled
            .push((at, end))
            .map_err(|_| IpError::TooLarge(end))?;
        if !datagram.more_fragments() {
            self.total = Some(end);
        }
        if datagram.is_initial_fragment() && !self.have_header {
            let header = datagram
                .header()
                .get(..MIN_HEADER_LEN)
                .ok_or(IpError::TooLarge(end))?;
            self.header.copy_from_slice(header);
            self.have_header = true;
        }
        Ok(())
    }

    /// Whether every byte from zero to the total length has arrived.
    fn is_complete(&self) -> bool {
        let Some(total) = self.total else {
            return false;
        };
        if !self.have_header {
            return false;
        }
        let mut reached = 0usize;
        let mut progressed = true;
        while progressed {
            progressed = false;
            for (start, stop) in self.filled.iter().copied() {
                if start <= reached && stop > reached {
                    reached = stop;
                    progressed = true;
                }
            }
        }
        reached >= total
    }
}
