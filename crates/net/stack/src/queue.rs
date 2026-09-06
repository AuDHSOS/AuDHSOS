// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The frames that have been written and not yet handed to the driver.
//!
//! One received frame can make several go out — an answer to it, and the
//! datagram that was waiting for the neighbor it just taught the cache
//! about, cut into as many pieces as the MTU needs — and a driver with a
//! full ring cannot take any of them at that moment. So they wait here,
//! in memory the caller supplied, in the order they were written.
//!
//! A record is its length and its bytes. One that no longer fits behind
//! the last begins at the front of the buffer instead of being cut in
//! two, which is what keeps a frame one slice; the bytes it leaves at the
//! end carry a length no frame has, so the reader steps over them. A full
//! queue drops the newest frame and counts it, because the frames already
//! in it are the older ones and a link that is behind is better served in
//! order.

use core::mem::size_of;

/// How many bytes the length in front of a record takes.
const HEADER: usize = size_of::<u16>();

/// The length that stands for the unused bytes at the end of the buffer.
/// A frame is at most 1518 bytes, so no record ever carries it.
const SKIP: u16 = u16::MAX;

/// The frames waiting to go out.
#[derive(Debug)]
pub struct Queue<'a> {
    /// The memory the caller supplied.
    bytes: &'a mut [u8],
    /// Where the oldest record begins.
    head: usize,
    /// Where the next record goes.
    tail: usize,
    /// How many bytes the records and the skipped tails take together.
    used: usize,
    /// How many frames are waiting.
    count: usize,
    /// How many were dropped for want of room.
    dropped: u32,
}

impl<'a> Queue<'a> {
    /// An empty queue over `bytes`.
    #[must_use]
    pub const fn new(bytes: &'a mut [u8]) -> Queue<'a> {
        Queue {
            bytes,
            head: 0,
            tail: 0,
            used: 0,
            count: 0,
            dropped: 0,
        }
    }

    /// How many frames are waiting.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.count
    }

    /// Whether none are.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// How many frames were dropped for want of room.
    #[must_use]
    pub const fn dropped(&self) -> u32 {
        self.dropped
    }

    /// Gives the memory back.
    #[must_use]
    pub const fn into_bytes(self) -> &'a mut [u8] {
        self.bytes
    }

    /// Puts `frame` at the back, and answers whether there was room.
    pub fn push(&mut self, frame: &[u8]) -> bool {
        let Ok(len) = u16::try_from(frame.len()) else {
            self.dropped = self.dropped.saturating_add(1);
            return false;
        };
        if len == SKIP {
            self.dropped = self.dropped.saturating_add(1);
            return false;
        }
        let need = HEADER.saturating_add(frame.len());
        let capacity = self.bytes.len();
        let to_end = capacity.saturating_sub(self.tail);
        // The record goes behind the last one, or at the front when it no
        // longer fits behind it; the bytes it leaves at the end are used
        // as far as the room is concerned.
        let (at, skip) = if to_end >= need {
            (self.tail, 0)
        } else {
            (0, to_end)
        };
        if self.used.saturating_add(skip).saturating_add(need) > capacity {
            self.dropped = self.dropped.saturating_add(1);
            return false;
        }
        if skip >= HEADER {
            self.write_len(self.tail, SKIP);
        }
        self.write_len(at, len);
        let from = at.saturating_add(HEADER);
        let Some(slot) = self.bytes.get_mut(from..from.saturating_add(frame.len())) else {
            self.dropped = self.dropped.saturating_add(1);
            return false;
        };
        slot.copy_from_slice(frame);
        self.tail = at.saturating_add(need);
        self.used = self.used.saturating_add(skip).saturating_add(need);
        self.count = self.count.saturating_add(1);
        true
    }

    /// The oldest frame, without taking it out.
    #[must_use]
    pub fn peek(&self) -> Option<&[u8]> {
        let (at, len) = self.front()?;
        let from = at.saturating_add(HEADER);
        self.bytes.get(from..from.saturating_add(len))
    }

    /// Copies the oldest frame into `out` and takes it out.
    ///
    /// A frame longer than `out` stays where it is and nothing is
    /// answered, so a caller with a buffer of one MTU never loses one.
    pub fn pop_into<'o>(&mut self, out: &'o mut [u8]) -> Option<&'o [u8]> {
        let (at, len) = self.front()?;
        let from = at.saturating_add(HEADER);
        let frame = self.bytes.get(from..from.saturating_add(len))?;
        let slot = out.get_mut(..len)?;
        slot.copy_from_slice(frame);
        self.take(at, len);
        Some(slot)
    }

    /// Where the oldest record is and how long it is, with any skipped
    /// tail stepped over.
    fn front(&self) -> Option<(usize, usize)> {
        if self.count == 0 {
            return None;
        }
        let at = self.wrapped(self.head);
        let len = self.read_len(at)?;
        if len == SKIP {
            let at = 0;
            let len = self.read_len(at)?;
            return Some((at, usize::from(len)));
        }
        Some((at, usize::from(len)))
    }

    /// `at`, moved to the front when there is no room for a length there.
    const fn wrapped(&self, at: usize) -> usize {
        if self.bytes.len().saturating_sub(at) < HEADER {
            0
        } else {
            at
        }
    }

    /// Drops the record at `at`, which is `len` bytes long, and the
    /// skipped tail in front of it when there was one.
    const fn take(&mut self, at: usize, len: usize) {
        let capacity = self.bytes.len();
        let skipped = if at == 0 && self.head != 0 {
            capacity.saturating_sub(self.head)
        } else {
            0
        };
        self.head = at.saturating_add(HEADER).saturating_add(len);
        self.used = self
            .used
            .saturating_sub(skipped)
            .saturating_sub(HEADER.saturating_add(len));
        self.count = self.count.saturating_sub(1);
        if self.count == 0 {
            self.head = 0;
            self.tail = 0;
            self.used = 0;
        }
    }

    /// The length in front of the record at `at`.
    fn read_len(&self, at: usize) -> Option<u16> {
        let bytes = self
            .bytes
            .get(at..at.saturating_add(HEADER))?
            .first_chunk::<HEADER>()?;
        Some(u16::from_be_bytes(*bytes))
    }

    /// Writes the length in front of a record at `at`.
    fn write_len(&mut self, at: usize, len: u16) {
        if let Some(slot) = self.bytes.get_mut(at..at.saturating_add(HEADER)) {
            slot.copy_from_slice(&len.to_be_bytes());
        }
    }
}
