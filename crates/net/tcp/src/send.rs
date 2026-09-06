// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The send buffer: bytes the caller handed over and the peer has not
//! acknowledged yet.
//!
//! It is a byte ring over memory the caller supplied. The front of it is
//! the oldest unacknowledged byte, which is `SND.UNA`; everything in it is
//! either in flight or waiting to go, and a byte leaves only when the peer
//! says it arrived.
//!
//! A segment's payload is a borrow into this ring and never a copy, which
//! is why [`contiguous`](SendBuffer::contiguous) stops at the physical end
//! of the buffer rather than wrapping around it. What that costs is one
//! short segment each time the ring wraps; what it buys is that a
//! retransmission is the same borrow again, with nothing held anywhere
//! else and nothing copied to hold it.

/// Bytes waiting to be acknowledged.
#[derive(Debug)]
pub struct SendBuffer<'a> {
    /// The memory they lie in.
    bytes: &'a mut [u8],
    /// Where the oldest unacknowledged byte is.
    head: usize,
    /// How many bytes are held.
    len: usize,
}

impl<'a> SendBuffer<'a> {
    /// An empty buffer over `bytes`.
    #[must_use]
    pub const fn new(bytes: &'a mut [u8]) -> SendBuffer<'a> {
        SendBuffer {
            bytes,
            head: 0,
            len: 0,
        }
    }

    /// Gives the memory back.
    #[must_use]
    pub const fn into_bytes(self) -> &'a mut [u8] {
        self.bytes
    }

    /// How many bytes it holds in all.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.bytes.len()
    }

    /// How many are waiting.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether none are.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// How many more it would take.
    #[must_use]
    pub const fn free(&self) -> usize {
        self.bytes.len().saturating_sub(self.len)
    }

    /// Appends as much of `data` as there is room for, and answers how
    /// much that was.
    ///
    /// A partial write is the right answer here and not a failure: a
    /// caller with more to send than the window holds sends the rest when
    /// the window opens, and that is the normal working of a stream.
    pub fn write(&mut self, data: &[u8]) -> usize {
        let taken = data.len().min(self.free());
        let Some(source) = data.get(..taken) else {
            return 0;
        };
        let capacity = self.bytes.len();
        let start = wrap(self.head.saturating_add(self.len), capacity);
        let (first, second) = source.split_at(source.len().min(capacity.saturating_sub(start)));
        copy_into(self.bytes, start, first);
        copy_into(self.bytes, 0, second);
        self.len = self.len.saturating_add(taken);
        taken
    }

    /// The bytes at `offset` from the front, as far as they run without
    /// leaving the end of the buffer, and at most `limit` of them.
    ///
    /// This is what a segment's payload borrows. An offset at or past the
    /// end of what is held yields nothing.
    #[must_use]
    pub fn contiguous(&self, offset: usize, limit: usize) -> &[u8] {
        let Some(held) = self.len.checked_sub(offset) else {
            return &[];
        };
        let capacity = self.bytes.len();
        let start = wrap(self.head.saturating_add(offset), capacity);
        let run = held.min(capacity.saturating_sub(start)).min(limit);
        self.bytes
            .get(start..)
            .and_then(|rest| rest.get(..run))
            .unwrap_or(&[])
    }

    /// Drops `count` acknowledged bytes from the front, and answers how
    /// many were actually there to drop.
    pub fn acknowledge(&mut self, count: usize) -> usize {
        let dropped = count.min(self.len);
        self.head = wrap(self.head.saturating_add(dropped), self.bytes.len());
        self.len = self.len.saturating_sub(dropped);
        if self.len == 0 {
            self.head = 0;
        }
        dropped
    }
}

/// `value` brought back inside a ring of `capacity` bytes.
const fn wrap(value: usize, capacity: usize) -> usize {
    match value.checked_rem(capacity) {
        Some(wrapped) => wrapped,
        // A ring of no bytes holds nothing at any position.
        None => 0,
    }
}

/// Copies `data` into `bytes` at `at`, as far as it goes.
fn copy_into(bytes: &mut [u8], at: usize, data: &[u8]) {
    if let Some(room) = bytes
        .get_mut(at..)
        .and_then(|rest| rest.get_mut(..data.len()))
    {
        room.copy_from_slice(data);
    }
}
