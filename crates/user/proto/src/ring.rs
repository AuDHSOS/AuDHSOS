// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The byte ring a socket's data travels through, and the page that holds
//! the two of them.
//!
//! The input protocol's ring carries records of a fixed width, because an
//! event is one. A connection carries a stream, so this ring carries bytes:
//! a writer puts as many as fit and says how many it put, a reader takes as
//! many as it asked for and says how many it took, and neither blocks.
//!
//! One page holds two rings. The inbound one is written by the server and
//! read by the client; the outbound one the other way round. Which is which
//! is fixed by the layout and not by a field, so a client that maps the page
//! needs nothing told to it.
//!
//! Only atomic accesses are permitted after the page is shared. Exactly one
//! cooperating writer and one reader make it a queue; a client that writes
//! the reader's sequence number of the ring it does not own makes its own
//! bytes wrong and nothing else, and no access of the server can leave the
//! page.
//!
//! Invariants: the writer never passes the reader, so a byte that was
//! written is read once; a sequence number that names no byte of the ring
//! is refused rather than indexed with.

use core::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering};

/// Bytes of the header of one ring.
pub const RING_HEADER_LEN: usize = 24;

/// Bytes of one ring, header included.
pub const RING_LEN: usize = 4096;

/// Bytes of data one ring holds.
pub const RING_CAPACITY: u32 = 4072;

/// The same number as a length, which is what the array of bytes is sized
/// by.
#[expect(
    clippy::as_conversions,
    reason = "a capacity of four thousand widened to the type an array length has, in a const"
)]
const RING_DATA: usize = RING_CAPACITY as usize;

/// Bytes of the object that holds both rings of one socket.
pub const SOCKET_PAGE_LEN: usize = 2 * RING_LEN;

/// The header of a ring, as it stands in the first twenty-four bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RingHeader {
    /// How many bytes the writer has put in since the ring was made.
    pub write_seq: u64,
    /// How many the reader has taken.
    pub read_seq: u64,
    /// How many bytes the writer could not put in.
    pub dropped: u32,
    /// How many bytes the ring holds.
    pub capacity: u32,
}

/// One shared ring.
#[derive(Debug)]
#[repr(C, align(8))]
pub struct Ring {
    /// Published byte count; stored with release ordering after the bytes.
    pub write_seq: AtomicU64,
    /// Consumed byte count; stored with release ordering after the read.
    pub read_seq: AtomicU64,
    /// Bytes that did not fit.
    pub dropped: AtomicU32,
    /// The fixed capacity; another value makes the ring invalid.
    pub capacity: AtomicU32,
    /// The bytes.
    pub data: [AtomicU8; RING_DATA],
}

/// The two rings of one socket, in the order the layout fixes.
#[derive(Debug)]
#[repr(C, align(8))]
pub struct SocketPage {
    /// What the server writes and the client reads.
    pub inbound: Ring,
    /// What the client writes and the server reads.
    pub outbound: Ring,
}

const _: () = assert!(core::mem::size_of::<Ring>() == RING_LEN);
const _: () = assert!(core::mem::offset_of!(Ring, data) == RING_HEADER_LEN);
const _: () = assert!(core::mem::size_of::<SocketPage>() == SOCKET_PAGE_LEN);
const _: () = assert!(core::mem::offset_of!(SocketPage, outbound) == RING_LEN);

impl Default for Ring {
    fn default() -> Self {
        Self::new()
    }
}

impl Ring {
    /// An empty ring, ready to share.
    #[must_use]
    pub const fn new() -> Self {
        Ring {
            write_seq: AtomicU64::new(0),
            read_seq: AtomicU64::new(0),
            dropped: AtomicU32::new(0),
            capacity: AtomicU32::new(RING_CAPACITY),
            data: [const { AtomicU8::new(0) }; RING_DATA],
        }
    }

    /// Writes the header of a private, zero-filled ring before its page is
    /// shared.
    pub fn initialize(&self) {
        self.write_seq.store(0, Ordering::Relaxed);
        self.read_seq.store(0, Ordering::Relaxed);
        self.dropped.store(0, Ordering::Relaxed);
        self.capacity.store(RING_CAPACITY, Ordering::Release);
    }

    /// A snapshot of the header.
    #[must_use]
    pub fn header(&self) -> RingHeader {
        RingHeader {
            write_seq: self.write_seq.load(Ordering::Acquire),
            read_seq: self.read_seq.load(Ordering::Acquire),
            dropped: self.dropped.load(Ordering::Relaxed),
            capacity: self.capacity.load(Ordering::Relaxed),
        }
    }

    /// How many bytes are waiting to be read.
    #[must_use]
    pub fn held(&self) -> u32 {
        let header = self.header();
        let held = header.write_seq.wrapping_sub(header.read_seq);
        u32::try_from(held)
            .unwrap_or(RING_CAPACITY)
            .min(RING_CAPACITY)
    }

    /// How many bytes may still be written.
    #[must_use]
    pub fn free(&self) -> u32 {
        RING_CAPACITY.saturating_sub(self.held())
    }

    /// `true` when nothing is waiting.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.held() == 0
    }

    /// Writes as many bytes of `bytes` as fit and answers how many.
    ///
    /// A ring that took fewer counts the rest as dropped, which a client
    /// that writes more than the connection can take reads back rather
    /// than discovering by a byte that never arrives.
    pub fn write(&self, bytes: &[u8]) -> usize {
        let header = self.header();
        if header.capacity != RING_CAPACITY {
            return 0;
        }
        let free = usize::try_from(self.free()).unwrap_or(0);
        let taken = bytes.len().min(free);
        for (step, byte) in bytes.iter().take(taken).enumerate() {
            let seq = header
                .write_seq
                .wrapping_add(u64::try_from(step).unwrap_or(0));
            if let Some(slot) = self.at(seq) {
                slot.store(*byte, Ordering::Relaxed);
            }
        }
        if taken < bytes.len() {
            let lost = u32::try_from(bytes.len().saturating_sub(taken)).unwrap_or(u32::MAX);
            let _previous =
                self.dropped
                    .try_update(Ordering::Relaxed, Ordering::Relaxed, |before| {
                        Some(before.saturating_add(lost))
                    });
        }
        self.write_seq.store(
            header
                .write_seq
                .wrapping_add(u64::try_from(taken).unwrap_or(0)),
            Ordering::Release,
        );
        taken
    }

    /// Takes as many bytes as `into` holds and answers how many.
    pub fn read(&self, into: &mut [u8]) -> usize {
        let header = self.header();
        if header.capacity != RING_CAPACITY {
            return 0;
        }
        let held = usize::try_from(self.held()).unwrap_or(0);
        let taken = into.len().min(held);
        for (step, slot) in into.iter_mut().take(taken).enumerate() {
            let seq = header
                .read_seq
                .wrapping_add(u64::try_from(step).unwrap_or(0));
            *slot = self.at(seq).map_or(0, |byte| byte.load(Ordering::Relaxed));
        }
        self.read_seq.store(
            header
                .read_seq
                .wrapping_add(u64::try_from(taken).unwrap_or(0)),
            Ordering::Release,
        );
        taken
    }

    /// How many bytes the writer could not put in since this was last
    /// asked, which also forgets them.
    pub fn take_dropped(&self) -> u32 {
        self.dropped.swap(0, Ordering::Relaxed)
    }

    /// The byte sequence number `seq` names.
    fn at(&self, seq: u64) -> Option<&AtomicU8> {
        let index = usize::try_from(seq.checked_rem(u64::from(RING_CAPACITY))?).ok()?;
        self.data.get(index)
    }
}

impl Default for SocketPage {
    fn default() -> Self {
        Self::new()
    }
}

impl SocketPage {
    /// A page of two empty rings.
    #[must_use]
    pub const fn new() -> Self {
        SocketPage {
            inbound: Ring::new(),
            outbound: Ring::new(),
        }
    }

    /// Writes both headers of a private, zero-filled page before it is
    /// shared.
    pub fn initialize(&self) {
        self.inbound.initialize();
        self.outbound.initialize();
    }
}
