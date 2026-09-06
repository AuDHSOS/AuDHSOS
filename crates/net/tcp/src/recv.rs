// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The receive buffer: the window this end advertises, with the bytes
//! that have arrived in it, in order and out of it.
//!
//! The front of the buffer is the oldest byte the caller has not read.
//! From there a run of bytes has arrived in order, and its end is
//! `RCV.NXT` — the number this end asks the peer for next. Behind that run
//! there may be bytes that arrived early, with gaps between them where a
//! segment was lost or reordered.
//!
//! Those early bytes are written where they belong the moment they arrive,
//! because their sequence number says where that is, and a short list of
//! ranges remembers which parts of the window hold them. When the gap in
//! front of a range closes, the range is absorbed into the run and the
//! bytes are readable — without ever having been copied a second time.
//! That is the whole reason for the arrangement: the alternative is to
//! keep a queue of whole segments beside the buffer and copy each of them
//! in later, which costs a second copy of every reordered segment and a
//! second pool of memory whose size has nothing to do with the window.
//!
//! The list holds [`MAX_HOLES`] ranges. There is no selective
//! acknowledgment in this implementation (D-50), so a sender learns about
//! the first gap and about no other; a receiver that remembers four is
//! already remembering more than any sender can act on. A range that does
//! not fit is forgotten, its bytes with it, and the peer sends them again.
//!
//! The advertised window is what is left of the buffer beyond the run:
//! the peer may fill exactly the room there is and no more. It therefore
//! shrinks as unread bytes pile up and opens again as the caller reads,
//! and it never retracts an offer, because the run and the read position
//! only ever move forward.

use audhsos_collections::ArrayVec;

/// How many ranges of early bytes a buffer remembers.
pub const MAX_HOLES: usize = 4;

/// A run of bytes that arrived early, as offsets from the front of the
/// buffer. Half open: `start` is the first byte and `end` is one past the
/// last.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Extent {
    /// The first byte.
    start: usize,
    /// One past the last.
    end: usize,
}

/// The window, and what has arrived in it.
#[derive(Debug)]
pub struct RecvBuffer<'a> {
    /// The memory the window lies in.
    bytes: &'a mut [u8],
    /// Where the oldest unread byte is.
    head: usize,
    /// How many bytes from the front have arrived in order.
    ready: usize,
    /// The ranges of early bytes behind them, sorted and never touching.
    early: ArrayVec<Extent, MAX_HOLES>,
}

impl<'a> RecvBuffer<'a> {
    /// An empty buffer over `bytes`.
    #[must_use]
    pub const fn new(bytes: &'a mut [u8]) -> RecvBuffer<'a> {
        RecvBuffer {
            bytes,
            head: 0,
            ready: 0,
            early: ArrayVec::new(),
        }
    }

    /// Gives the memory back.
    #[must_use]
    pub const fn into_bytes(self) -> &'a mut [u8] {
        self.bytes
    }

    /// How many bytes the window is at its widest.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.bytes.len()
    }

    /// How many bytes have arrived in order and not been read.
    #[must_use]
    pub const fn ready(&self) -> usize {
        self.ready
    }

    /// Whether none have.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.ready == 0
    }

    /// What this end advertises: the room beyond the run that has
    /// arrived.
    #[must_use]
    pub const fn window(&self) -> usize {
        self.bytes.len().saturating_sub(self.ready)
    }

    /// Whether any early bytes are being held.
    #[must_use]
    pub const fn has_early_bytes(&self) -> bool {
        !self.early.is_empty()
    }

    /// The readable bytes at the front, as far as they run without
    /// leaving the end of the buffer.
    #[must_use]
    pub fn contiguous(&self) -> &[u8] {
        let run = self.ready.min(self.bytes.len().saturating_sub(self.head));
        self.bytes
            .get(self.head..)
            .and_then(|rest| rest.get(..run))
            .unwrap_or(&[])
    }

    /// Copies as much as fits into `out` and answers how much that was.
    pub fn read(&mut self, out: &mut [u8]) -> usize {
        let mut taken = 0;
        while taken < out.len() {
            let run = self.contiguous();
            let step = run.len().min(out.len().saturating_sub(taken));
            if step == 0 {
                break;
            }
            let source = run.get(..step).unwrap_or(&[]);
            if let Some(room) = out.get_mut(taken..).and_then(|rest| rest.get_mut(..step)) {
                room.copy_from_slice(source);
            }
            self.consume(step);
            taken = taken.saturating_add(step);
        }
        taken
    }

    /// Drops `count` bytes that have been read from the front.
    pub fn consume(&mut self, count: usize) {
        let dropped = count.min(self.ready);
        self.head = wrap(self.head.saturating_add(dropped), self.bytes.len());
        self.ready = self.ready.saturating_sub(dropped);
        for extent in self.early.iter_mut() {
            extent.start = extent.start.saturating_sub(dropped);
            extent.end = extent.end.saturating_sub(dropped);
        }
        if self.ready == 0 && self.early.is_empty() {
            self.head = 0;
        }
    }

    /// Puts `data` into the window at `offset` bytes from the front, and
    /// answers how far the run of bytes that arrived in order now reaches.
    ///
    /// The caller's next expected sequence number is the front of the
    /// buffer plus that answer. Bytes beyond the window are ignored, as
    /// are bytes the run has already passed; what is left is written where
    /// it belongs and remembered.
    pub fn accept(&mut self, offset: usize, data: &[u8]) -> usize {
        let capacity = self.bytes.len();
        if offset >= capacity {
            return self.ready;
        }
        let end = offset.saturating_add(data.len()).min(capacity);
        let Some(source) = data.get(..end.saturating_sub(offset)) else {
            return self.ready;
        };
        self.put(offset, source);
        if offset <= self.ready {
            if end > self.ready {
                self.ready = end;
            }
            self.absorb();
        } else {
            self.remember(offset, end);
        }
        self.ready
    }

    /// Writes `data` into the ring at `offset` from the front.
    fn put(&mut self, offset: usize, data: &[u8]) {
        let capacity = self.bytes.len();
        let start = wrap(self.head.saturating_add(offset), capacity);
        let (first, second) = data.split_at(data.len().min(capacity.saturating_sub(start)));
        copy_into(self.bytes, start, first);
        copy_into(self.bytes, 0, second);
    }

    /// Records that `start..end` has arrived early, merging it with every
    /// range it touches. A range that does not fit is forgotten.
    fn remember(&mut self, start: usize, end: usize) {
        let mut low = start;
        let mut high = end;
        let mut index = 0;
        while let Some(&extent) = self.early.get(index) {
            if extent.end < low || extent.start > high {
                index = index.saturating_add(1);
                continue;
            }
            low = low.min(extent.start);
            high = high.max(extent.end);
            self.early.remove(index);
        }
        let at = self
            .early
            .iter()
            .position(|extent| extent.start > low)
            .unwrap_or(self.early.len());
        let _ = self.early.insert(
            at,
            Extent {
                start: low,
                end: high,
            },
        );
    }

    /// Takes into the run every range it has grown up to.
    fn absorb(&mut self) {
        while let Some(&extent) = self.early.first() {
            if extent.start > self.ready {
                break;
            }
            if extent.end > self.ready {
                self.ready = extent.end;
            }
            self.early.remove(0);
        }
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
