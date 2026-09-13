// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The frame buffers of one side of the device, and the seam through
//! which this crate reaches them.
//!
//! The buffers are ordinary memory the device reaches by bus master
//! access, as the rings are, and the crate reaches them the way it reaches
//! the rings: an implementation hands over the bytes and the address the
//! device knows them by, and this crate writes the header and the frame
//! into them.
//!
//! One buffer holds one whole frame behind the header, which is what
//! refusing `VIRTIO_NET_F_MRG_RXBUF` buys.
//!
//! Invariant: every accessor answers `None` for a buffer the area does not
//! hold, so an index out of range is a refusal and not a wild access.

/// The buffers of one side.
pub trait Frames {
    /// How many buffers the area holds.
    fn count(&self) -> u16;

    /// Bytes of one buffer, header included.
    fn len(&self) -> u32;

    /// `true` for an area of no buffers.
    fn is_empty(&self) -> bool {
        self.count() == 0
    }

    /// The address the device reads or writes buffer `index` at.
    fn address(&self, index: u16) -> Option<u64>;

    /// The bytes of buffer `index`.
    fn bytes(&self, index: u16) -> Option<&[u8]>;

    /// The same, for writing.
    fn bytes_mut(&mut self, index: u16) -> Option<&mut [u8]>;
}
