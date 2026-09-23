// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Receive: putting every buffer of the receive area into the available
//! ring, and taking a frame out of the one the device wrote into.
//!
//! The path of virtio 5.1.9.3 and 5.1.9.4 with one simplification, which
//! refusing `VIRTIO_NET_F_MRG_RXBUF` buys: one used element is one frame,
//! because the device writes a frame into one buffer or drops it.

use virtio_queue::{Buffer, Queue, QueueMemory};

use crate::error::NetError;
use crate::frames::Frames;
use crate::init::{NO_BUFFER, Net};
use crate::net::HEADER_LEN;

/// Minimum receive buffer without merged buffers or guest offloads.
const MIN_RECEIVE_BUFFER: u32 = 1526;

impl<const SLOTS: usize> Net<SLOTS> {
    /// Puts every receive buffer that is not already with the device into
    /// the available ring, and answers how many went in.
    ///
    /// Called once after [`initialize`](Net::initialize) and never again
    /// in a driver that keeps to one buffer per call: every buffer taken
    /// out by [`receive`](Net::receive) goes back in the same call. It is
    /// written to be called again all the same, because a receive that
    /// refused a frame is a buffer that came back.
    ///
    /// # Errors
    ///
    /// [`NetError::Slots`] for a queue or area larger than this driver
    /// holds, [`NetError::BufferTooShort`] for a receive buffer below the
    /// virtio minimum, [`NetError::Buffer`] for a buffer the area would
    /// not name, and [`NetError::Queue`] for what the queue refused.
    pub fn fill<const WORDS: usize>(
        &mut self,
        queue: &mut Queue<WORDS>,
        memory: &mut impl QueueMemory,
        frames: &impl Frames,
    ) -> Result<u16, NetError> {
        if usize::from(queue.size()) > SLOTS {
            return Err(NetError::Slots(queue.size()));
        }
        let count = frames.count();
        if usize::from(count) > SLOTS {
            return Err(NetError::Slots(count));
        }
        if frames.len() < MIN_RECEIVE_BUFFER {
            return Err(NetError::BufferTooShort(frames.len()));
        }
        let mut filled = 0u16;
        for index in 0..count {
            if self.posted.get(usize::from(index)).copied().unwrap_or(true) {
                continue;
            }
            self.post(queue, memory, frames, index)?;
            filled = filled.saturating_add(1);
        }
        Ok(filled)
    }

    /// Takes the next frame the device wrote, copies it into `into`, and
    /// puts the buffer it came in back into the available ring.
    ///
    /// The copy is what makes the buffer free to go back at once. A slice
    /// of the buffer itself would be memory the device may write into from
    /// the moment the buffer is available again, and the caller would be
    /// reading a frame while the next one lands on it.
    ///
    /// Answers `None` when the device has written nothing since the last
    /// call.
    ///
    /// # Errors
    ///
    /// [`NetError::ShortFrame`] for a used element that does not hold the
    /// header, [`NetError::FrameTooLong`] for one longer than `into`,
    /// [`NetError::UnknownBuffer`] for an element naming a descriptor this
    /// driver did not hand out, [`NetError::Slots`] for an oversized queue,
    /// [`NetError::Buffer`] for a buffer the area would not name, and
    /// [`NetError::Queue`] for what the queue refused.
    ///
    /// Short frames and frames larger than `into` put the buffer back
    /// before reporting the refusal. A buffer outside `frames` remains
    /// available for a later fill using the original area.
    /// A used length above the buffer clears its ownership before the
    /// refusal, so a later [`fill`](Net::fill) can post it again. Other
    /// queue refusals require a device reset with [`reset`](Net::reset).
    pub fn receive<'b, const WORDS: usize>(
        &mut self,
        queue: &mut Queue<WORDS>,
        memory: &mut impl QueueMemory,
        frames: &impl Frames,
        into: &'b mut [u8],
    ) -> Result<Option<&'b [u8]>, NetError> {
        if usize::from(queue.size()) > SLOTS {
            return Err(NetError::Slots(queue.size()));
        }
        let completion = match queue.next_used(&self.device, memory) {
            Err(error @ virtio_queue::QueueError::UsedLength { head, .. }) => {
                let _released = self.release(head);
                return Err(NetError::Queue(error));
            }
            result => result?,
        };
        let Some(completion) = completion else {
            return Ok(None);
        };
        let index = self.release(completion.head)?;
        if index >= frames.count() {
            return Err(NetError::UnknownBuffer(completion.head));
        }
        let taken = take(frames, index, completion.length, into);
        // The buffer goes back whatever the length said, so that a frame
        // this driver will not hand on does not cost the device a buffer.
        self.post(queue, memory, frames, index)?;
        let len = taken?;
        Ok(Some(into.get(..len).unwrap_or(&[])))
    }

    /// Clears ownership after the queue frees a receive descriptor.
    fn release(&mut self, head: u16) -> Result<u16, NetError> {
        let owner = self
            .taken
            .get_mut(usize::from(head))
            .ok_or(NetError::UnknownBuffer(head))?;
        let index = core::mem::replace(owner, NO_BUFFER);
        if index == NO_BUFFER {
            return Err(NetError::UnknownBuffer(head));
        }
        let posted = self
            .posted
            .get_mut(usize::from(index))
            .ok_or(NetError::Buffer(index))?;
        *posted = false;
        Ok(index)
    }

    /// Puts buffer `index` into the available ring as one device-writable
    /// descriptor, and remembers which descriptor it went out on.
    fn post<const WORDS: usize>(
        &mut self,
        queue: &mut Queue<WORDS>,
        memory: &mut impl QueueMemory,
        frames: &impl Frames,
        index: u16,
    ) -> Result<(), NetError> {
        let address = frames.address(index).ok_or(NetError::Buffer(index))?;
        let buffer = Buffer::from_device(address, frames.len());
        let head = queue.add(&self.device, memory, &[buffer])?;
        *self
            .taken
            .get_mut(usize::from(head))
            .ok_or(NetError::UnknownBuffer(head))? = index;
        *self
            .posted
            .get_mut(usize::from(index))
            .ok_or(NetError::Buffer(index))? = true;
        Ok(())
    }
}

/// Copies the frame of buffer `index` into `into` and answers how many
/// bytes it is.
fn take(
    frames: &impl Frames,
    index: u16,
    written: u32,
    into: &mut [u8],
) -> Result<usize, NetError> {
    let header = u32::try_from(HEADER_LEN).unwrap_or(u32::MAX);
    if written < header {
        return Err(NetError::ShortFrame(written));
    }
    let len = usize::try_from(written.saturating_sub(header)).unwrap_or(0);
    if len > into.len() {
        return Err(NetError::FrameTooLong {
            len,
            capacity: into.len(),
        });
    }
    let bytes = frames.bytes(index).ok_or(NetError::Buffer(index))?;
    let frame = bytes
        .get(HEADER_LEN..HEADER_LEN.saturating_add(len))
        .ok_or(NetError::Buffer(index))?;
    into.get_mut(..len)
        .ok_or(NetError::Buffer(index))?
        .copy_from_slice(frame);
    Ok(len)
}
