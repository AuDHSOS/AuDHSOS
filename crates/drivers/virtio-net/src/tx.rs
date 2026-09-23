// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Transmit: draining what the device is done with, and putting one frame
//! behind a zeroed header into the available ring.
//!
//! The path of virtio 5.1.9.2. A send drains first, because a driver that
//! never takes its buffers back runs out of them while the device has
//! nothing; and a send with no free buffer is a refusal the caller
//! retries, not a wait, because a logic crate has no way to sleep.

use virtio_queue::{Buffer, Queue, QueueMemory};

use crate::error::NetError;
use crate::frames::Frames;
use crate::init::{NO_BUFFER, Net};
use crate::net::{HEADER_LEN, write_header};

impl<const SLOTS: usize> Net<SLOTS> {
    /// Takes back every transmit buffer the device is done with, and
    /// answers how many.
    ///
    /// # Errors
    ///
    /// [`NetError::UnknownBuffer`] for a used element naming a descriptor
    /// this driver did not hand out, [`NetError::Slots`] for an oversized
    /// queue, and [`NetError::Queue`] for what the queue refused. The
    /// elements taken before the refusal stay taken.
    pub fn drain<const WORDS: usize>(
        &mut self,
        queue: &mut Queue<WORDS>,
        memory: &impl QueueMemory,
    ) -> Result<u16, NetError> {
        if usize::from(queue.size()) > SLOTS {
            return Err(NetError::Slots(queue.size()));
        }
        let mut drained = 0u16;
        loop {
            let completion = match queue.next_used(&self.device, memory) {
                Err(error @ virtio_queue::QueueError::UsedLength { head, .. }) => {
                    let _released = self.release_transmit(head);
                    return Err(NetError::Queue(error));
                }
                result => result?,
            };
            let Some(completion) = completion else {
                break;
            };
            self.release_transmit(completion.head)?;
            drained = drained.saturating_add(1);
        }
        Ok(drained)
    }

    /// Clears ownership after the queue frees a transmit descriptor.
    fn release_transmit(&mut self, head: u16) -> Result<(), NetError> {
        let owner = self
            .sending
            .get_mut(usize::from(head))
            .ok_or(NetError::UnknownBuffer(head))?;
        let index = core::mem::replace(owner, NO_BUFFER);
        let busy = self
            .busy
            .get_mut(usize::from(index))
            .ok_or(NetError::UnknownBuffer(head))?;
        *busy = false;
        self.free_hint = index;
        Ok(())
    }

    /// Writes `frame` into a free transmit buffer behind a zeroed header
    /// and adds the two as one chain.
    ///
    /// Answers whether the device has to be told, which is the flag of
    /// virtio 2.7.10 it writes into the used ring; the caller then calls
    /// [`notify`](Net::notify).
    ///
    /// # Errors
    ///
    /// [`NetError::FrameTooLong`] for a frame longer than one buffer holds
    /// behind the header, [`NetError::NoBuffer`] when every buffer is with
    /// the device, [`NetError::Slots`] for a queue or area larger than this
    /// driver holds, [`NetError::Buffer`] for a buffer the area would not
    /// name, and [`NetError::Queue`] for what the queue refused.
    /// A queue refusal may leave the frame in a free buffer that the
    /// device does not hold. A retry writes the frame again.
    pub fn send<const WORDS: usize>(
        &mut self,
        queue: &mut Queue<WORDS>,
        memory: &mut impl QueueMemory,
        frames: &mut impl Frames,
        frame: &[u8],
    ) -> Result<bool, NetError> {
        if usize::from(queue.size()) > SLOTS {
            return Err(NetError::Slots(queue.size()));
        }
        let count = frames.count();
        if usize::from(count) > SLOTS {
            return Err(NetError::Slots(count));
        }
        let capacity = usize::try_from(frames.len())
            .unwrap_or(0)
            .saturating_sub(HEADER_LEN);
        if frame.len() > capacity {
            return Err(NetError::FrameTooLong {
                len: frame.len(),
                capacity,
            });
        }
        self.drain(queue, memory)?;
        let index = self.free(count).ok_or(NetError::NoBuffer)?;
        let address = frames.address(index).ok_or(NetError::Buffer(index))?;
        let bytes = frames.bytes_mut(index).ok_or(NetError::Buffer(index))?;
        write_header(bytes, index)?;
        let end = HEADER_LEN.saturating_add(frame.len());
        bytes
            .get_mut(HEADER_LEN..end)
            .ok_or(NetError::Buffer(index))?
            .copy_from_slice(frame);
        let length = u32::try_from(end).unwrap_or(u32::MAX);
        let head = queue.add(&self.device, memory, &[Buffer::to_device(address, length)])?;
        *self
            .sending
            .get_mut(usize::from(head))
            .ok_or(NetError::UnknownBuffer(head))? = index;
        *self
            .busy
            .get_mut(usize::from(index))
            .ok_or(NetError::Buffer(index))? = true;
        self.should_notify(queue, memory)
    }

    /// The first free transmit buffer, starting at the last completion.
    fn free(&self, count: u16) -> Option<u16> {
        (0..count)
            .map(|step| {
                let index = usize::from(self.free_hint)
                    .saturating_add(usize::from(step))
                    .checked_rem(usize::from(count))
                    .unwrap_or(0);
                u16::try_from(index).unwrap_or(u16::MAX)
            })
            .find(|index| !self.busy.get(usize::from(*index)).copied().unwrap_or(true))
    }
}
