// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The memory the stack under this server writes into, cut out of one
//! region the process mapped.
//!
//! Every buffer `net-stack` needs is the caller's: the queue frames leave
//! through, the window of each connection, and the buffer of each datagram
//! socket. They are one region here, cut once at the start, because a
//! server with no allocator cannot cut one later and because the sizes are
//! fixed by the constants below.
//!
//! Invariant: the pieces do not overlap, and a region too short for all of
//! them yields nothing rather than pieces of the wrong length.

use net_stack::FRAME_LEN;

/// How many UDP sockets the stack holds. One is the resolver's; the rest
/// are clients'.
pub const SOCKETS: usize = 4;

/// How many TCP connections it holds.
pub const CONNECTIONS: usize = 4;

/// Bytes of the window of one direction of one connection.
pub const WINDOW: usize = 4096;

/// Bytes of the receive buffer of one datagram socket.
pub const DATAGRAMS: usize = 2048;

/// How many frames wait between being written and being handed to the
/// driver.
pub const OUTGOING: usize = 4;

/// Bytes the region comes to.
pub const REGION_BYTES: usize =
    FRAME_LEN * OUTGOING + CONNECTIONS * 2 * WINDOW + SOCKETS * DATAGRAMS;

/// The pieces of the region that are handed out one at a time.
#[derive(Debug)]
pub struct Pool<'a> {
    /// The windows of the connections: the send half and the receive half
    /// of each, in that order.
    pub windows: [Option<(&'a mut [u8], &'a mut [u8])>; CONNECTIONS],
    /// The receive buffers of the datagram sockets.
    pub datagrams: [Option<&'a mut [u8]>; SOCKETS],
}

impl<'a> Pool<'a> {
    /// Cuts `bytes` into the queue the stack writes frames into and the
    /// pool of everything handed out later, or answers `None` for a
    /// region shorter than [`REGION_BYTES`].
    #[must_use]
    pub fn split(bytes: &'a mut [u8]) -> Option<(&'a mut [u8], Pool<'a>)> {
        if bytes.len() < REGION_BYTES {
            return None;
        }
        let (outgoing, mut rest) = bytes.split_at_mut(FRAME_LEN * OUTGOING);
        let mut windows = [const { None }; CONNECTIONS];
        for slot in &mut windows {
            let (send, tail) = rest.split_at_mut(WINDOW);
            let (receive, tail) = tail.split_at_mut(WINDOW);
            *slot = Some((send, receive));
            rest = tail;
        }
        let mut datagrams = [const { None }; SOCKETS];
        for slot in &mut datagrams {
            let (buffer, tail) = rest.split_at_mut(DATAGRAMS);
            *slot = Some(buffer);
            rest = tail;
        }
        Some((outgoing, Pool { windows, datagrams }))
    }

    /// Takes the windows of one connection, or `None` when they are out.
    pub fn take_window(&mut self) -> Option<(&'a mut [u8], &'a mut [u8])> {
        self.windows.iter_mut().find_map(Option::take)
    }

    /// Gives a pair of windows back, and answers whether the pool had a
    /// slot for it.
    ///
    /// A pool that has none is a pool something else came out of, which is
    /// a fault of the caller and not of the pool; the windows are let go
    /// rather than doubling one of the slots.
    pub fn put_window(&mut self, window: (&'a mut [u8], &'a mut [u8])) -> bool {
        let Some(slot) = self.windows.iter_mut().find(|slot| slot.is_none()) else {
            return false;
        };
        *slot = Some(window);
        true
    }

    /// Takes the buffer of one datagram socket, or `None` when they are
    /// out.
    pub fn take_datagram(&mut self) -> Option<&'a mut [u8]> {
        self.datagrams.iter_mut().find_map(Option::take)
    }

    /// Gives a datagram buffer back, and answers whether the pool had a
    /// slot for it.
    pub fn put_datagram(&mut self, buffer: &'a mut [u8]) -> bool {
        let Some(slot) = self.datagrams.iter_mut().find(|slot| slot.is_none()) else {
            return false;
        };
        *slot = Some(buffer);
        true
    }
}
