// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The split virtqueue: the descriptor table, the available ring, and the
//! used ring, driven from the driver's side.
//!
//! `struct virtq_avail` is virtio 1.4, section 2.7.6 and `struct
//! virtq_used` section 2.7.8; adding a chain is the seven steps of
//! section 2.7.13 and taking one back is section 2.7.14.
//!
//! Invariants: a descriptor is either free or in exactly one chain, and
//! the free set says which; the chain walk terminates, because every step
//! frees one descriptor that was in use and a step onto one that is
//! already free is refused; no operation indexes without a bound.

use audhsos_collections::BitSet;

use crate::descriptor::{Buffer, DESC_F_NEXT, DESC_F_WRITE, Descriptor};
use crate::device::Device;
use crate::error::{Area, QueueError};
use crate::memory::{
    QueueMemory, RING_HEADER_BYTES, USED_ELEMENT_BYTES, descriptor_table_bytes, read_u16, read_u32,
    write_u16,
};

/// Available ring flag: the driver wants no interrupt when the device
/// consumes a buffer (`VIRTQ_AVAIL_F_NO_INTERRUPT`, section 2.7.6). The
/// device reads it; the driver writes it. Section 2.7.7 calls this the
/// crude mechanism that stands in for `used_event` without `EVENT_IDX`.
pub const AVAIL_F_NO_INTERRUPT: u16 = 1 << 0;

/// Used ring flag: the device wants no notification when the driver adds
/// a buffer (`VIRTQ_USED_F_NO_NOTIFY`, section 2.7.8). The driver reads
/// it; the device writes it. Section 2.7.10.1 makes the reading a rule:
/// without `EVENT_IDX`, a driver that finds the flag at one should not
/// notify and a driver that finds it at zero must.
pub const USED_F_NO_NOTIFY: u16 = 1 << 0;

/// Offset of the flags in the available and in the used ring.
const FLAGS_AT: usize = 0;

/// Offset of the index in the available and in the used ring.
const INDEX_AT: usize = 2;

/// Offset of the written length inside a used element.
const USED_LENGTH_AT: usize = 4;

/// One chain the device has given back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Completion {
    /// The descriptor the chain began at, as [`Queue::add`] returned it.
    pub head: u16,
    /// How many bytes the device says it wrote into the chain. A device
    /// that only read from it reports zero.
    ///
    /// This is the device's number, and what can be checked about it is
    /// checked: it is never larger than the device-writable descriptors
    /// of the chain add up to, or [`Queue::next_used`] would have
    /// refused it with [`QueueError::UsedLength`].
    ///
    /// It may be smaller than what the device actually wrote. Section
    /// 2.7.8.2 permits that, because a device that failed part way may
    /// not know how far it got, and section 2.7.8.3 has the driver make
    /// no assumption about the bytes past this number. So it bounds what
    /// may be read, not what was touched.
    pub length: u32,
}

/// One split virtqueue.
///
/// `WORDS` sizes the free set, which holds one bit per descriptor in
/// words of sixty-four: a queue of up to 256 descriptors needs `WORDS =
/// 4`. The actual size is given to [`Queue::new`] and may be smaller.
///
/// The free set is the queue's own memory, not a list threaded through
/// the `next` fields of the shared descriptor table. The device cannot
/// write that table, but it writes the used ring, and a used element
/// naming a descriptor that is already free is what would tear a shared
/// free list. Here it is one bit test.
///
/// This is deliberately not [`Copy`]. A queue is the record of which
/// descriptors are out; two records of one table would each believe the
/// descriptors the other handed out are free, which is the one mistake
/// the free set exists to make impossible. [`Clone`] is left, because
/// a copy asked for by name is a copy the caller meant.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Queue<const WORDS: usize> {
    /// Number of descriptors, a power of two.
    size: u16,
    /// One bit per descriptor: set means free.
    free: BitSet<WORDS>,
    /// The next available index the driver will publish.
    avail_idx: u16,
    /// The used index the driver has read up to.
    last_used: u16,
    /// Chains added and not yet taken back.
    in_flight: u16,
}

impl<const WORDS: usize> Queue<WORDS> {
    /// A queue of `size` descriptors over `memory`, with every descriptor
    /// free and the available ring's flags and index at zero.
    ///
    /// What this writes is the two fields of the available ring the
    /// driver owns, and nothing else.
    ///
    /// The used ring belongs to the device, and this crate has no
    /// mutable accessor for it, so the caller zeroes all three regions
    /// before it tells the device where they are. That is not only
    /// tidiness: section 2.7.10.1 requires the driver to initialize the
    /// flags of the used ring to zero when it allocates the ring, and
    /// zeroing the region is how that requirement is met here. A used
    /// index that starts at something other than zero is refused by
    /// [`Queue::next_used`] rather than believed — but it is refused for
    /// the rest of the queue's life.
    ///
    /// A region is checked where it is used and not here, so that every
    /// refusal names the byte it wanted; the one exception is the
    /// descriptor table, which [`Queue::add`] measures before it takes a
    /// descriptor, so that a short table costs none.
    ///
    /// # Errors
    ///
    /// [`QueueError::Size`] when `size` is zero, is not a power of two,
    /// or is more descriptors than `WORDS` has bits for. A `u16` bounds
    /// the size from above on its own: the largest power of two it holds
    /// is 32768, which is [`MAX_QUEUE_SIZE`](crate::memory::MAX_QUEUE_SIZE).
    /// [`QueueError::Region`] when the available ring cannot hold its
    /// flags and its index.
    pub fn new(memory: &mut impl QueueMemory, size: u16) -> Result<Queue<WORDS>, QueueError> {
        if size == 0 || !size.is_power_of_two() {
            return Err(QueueError::Size(size));
        }
        let mut free = BitSet::<WORDS>::new();
        for index in 0..size {
            free.set(usize::from(index))
                .map_err(|_| QueueError::Size(size))?;
        }
        let queue = Queue {
            size,
            free,
            avail_idx: 0,
            last_used: 0,
            in_flight: 0,
        };
        let available = memory.available_ring_mut();
        write_u16(available, Area::AvailableRing, FLAGS_AT, 0)?;
        write_u16(available, Area::AvailableRing, INDEX_AT, 0)?;
        Ok(queue)
    }

    /// Number of descriptors.
    #[must_use]
    pub const fn size(&self) -> u16 {
        self.size
    }

    /// How many descriptors are free.
    #[must_use]
    pub fn free_count(&self) -> u16 {
        u16::try_from(self.free.count()).unwrap_or(u16::MAX)
    }

    /// How many chains have been added and not yet taken back.
    #[must_use]
    pub const fn in_flight(&self) -> u16 {
        self.in_flight
    }

    /// The available index the driver has published, which wraps at
    /// `2^16` and not at the size of the queue, as section 2.7.13.3 has
    /// it. It never decreases, which section 2.7.6.1 requires: there is
    /// no way to take a buffer back.
    #[must_use]
    pub const fn available_index(&self) -> u16 {
        self.avail_idx
    }

    /// The used index the driver has read up to.
    #[must_use]
    pub const fn last_used(&self) -> u16 {
        self.last_used
    }

    /// Adds one chain of buffers and publishes it, returning the
    /// descriptor it begins at.
    ///
    /// The descriptors of a chain are whichever ones were free; they are
    /// neither contiguous nor in any order the caller can rely on. What
    /// the caller gets back is the head, which is what the used ring
    /// names when the device is done.
    ///
    /// The buffers themselves are in the order the caller gave them, and
    /// that order has one rule, from section 2.7.4.2: every
    /// device-writable element comes after every device-readable one. A
    /// slice that breaks it is refused and not quietly sorted, because
    /// the order of the buffers is the order of the bytes in the
    /// request.
    ///
    /// This is allowed from `FEATURES_OK` and not only from `DRIVER_OK`,
    /// because populating a virtqueue is step 7 of section 3.1.1 and
    /// `DRIVER_OK` is step 8: a receive queue is filled before the
    /// device is live and consumed after. What may not happen before
    /// `DRIVER_OK` is the notification, and
    /// [`Queue::should_notify`] is what refuses that.
    ///
    /// # Errors
    ///
    /// [`QueueError::NotConfigured`] before `FEATURES_OK`.
    /// [`QueueError::Region`]
    /// when the descriptor table or the available ring is too short.
    /// [`QueueError::EmptyChain`] for no buffers,
    /// [`QueueError::ChainTooLong`] for more than the queue has
    /// descriptors, which no completion would make fit, and
    /// [`QueueError::BufferOrder`] for a buffer the device writes that
    /// stands before one it reads.
    /// [`QueueError::QueueFull`] when the free descriptors run out; the
    /// part of the chain already taken is given back first, and so is a
    /// whole chain that could not be published.
    pub fn add(
        &mut self,
        device: &Device,
        memory: &mut impl QueueMemory,
        buffers: &[Buffer],
    ) -> Result<u16, QueueError> {
        if !device.may_populate_queues() {
            return Err(QueueError::NotConfigured);
        }
        if buffers.len() > usize::from(self.size) {
            return Err(QueueError::ChainTooLong {
                buffers: buffers.len(),
                size: self.size,
            });
        }
        if !buffers.is_sorted_by_key(|buffer| buffer.direction) {
            return Err(QueueError::BufferOrder);
        }
        self.check_table(memory)?;

        let mut head: Option<u16> = None;
        let mut previous: Option<u16> = None;
        for buffer in buffers {
            let Some(index) = self.take_free() else {
                if let Some(started) = head {
                    self.free_chain(memory, started).map(drop)?;
                }
                return Err(QueueError::QueueFull {
                    free: self.free_count(),
                    needed: buffers.len(),
                });
            };
            let table = memory.descriptor_table_mut();
            Descriptor {
                address: buffer.address,
                length: buffer.length,
                flags: buffer.direction.flag(),
                next: 0,
            }
            .write(table, index)?;
            if let Some(before) = previous {
                Descriptor::link_to(table, before, index)?;
            }
            head = head.or(Some(index));
            previous = Some(index);
        }
        let Some(head) = head else {
            return Err(QueueError::EmptyChain);
        };
        match self.publish(memory, head) {
            Ok(()) => Ok(head),
            // A chain that could not be published is a chain the device
            // will never see, so its descriptors go back rather than
            // being lost. If giving them back finds the chain torn, that
            // is the worse news and the one reported.
            Err(error) => self.free_chain(memory, head).map(drop).and(Err(error)),
        }
    }

    /// Whether the driver has to notify the device after adding.
    ///
    /// Without `EVENT_IDX` this is the one flag the device writes into
    /// the used ring (section 2.7.10, D-52). It is step 7 of section
    /// 2.7.13, and the barrier this takes first is step 6: section
    /// 2.7.13.4.1 requires it, so that a driver cannot read a stale flag
    /// and stay silent about a buffer the device is waiting for.
    ///
    /// # Errors
    ///
    /// [`QueueError::NotLive`] before `DRIVER_OK`, which section 3.1.1
    /// requires in as many words: the driver must send no buffer
    /// available notification before it. [`QueueError::Region`] when the
    /// used ring cannot hold its flags.
    pub fn should_notify(
        &self,
        device: &Device,
        memory: &impl QueueMemory,
    ) -> Result<bool, QueueError> {
        if !device.is_live() {
            return Err(QueueError::NotLive);
        }
        memory.barrier();
        let flags = read_u16(memory.used_ring(), Area::UsedRing, FLAGS_AT)?;
        Ok(flags & USED_F_NO_NOTIFY == 0)
    }

    /// Tells the device whether to interrupt when it consumes a buffer.
    ///
    /// Like [`Queue::add`], this belongs to the setup of step 7 and is
    /// allowed from `FEATURES_OK`: what a queue is to do about
    /// interrupts is settled before the device is live, not after.
    ///
    /// # Errors
    ///
    /// [`QueueError::NotConfigured`] before `FEATURES_OK`, and
    /// [`QueueError::Region`] when the available ring cannot hold its
    /// flags.
    pub fn suppress_interrupts(
        &self,
        device: &Device,
        memory: &mut impl QueueMemory,
        suppress: bool,
    ) -> Result<(), QueueError> {
        if !device.may_populate_queues() {
            return Err(QueueError::NotConfigured);
        }
        let flags = if suppress { AVAIL_F_NO_INTERRUPT } else { 0 };
        write_u16(
            memory.available_ring_mut(),
            Area::AvailableRing,
            FLAGS_AT,
            flags,
        )
    }

    /// Takes the next chain the device has given back, freeing its
    /// descriptors, or `None` when there is none.
    ///
    /// # Errors
    ///
    /// [`QueueError::NotLive`] before `DRIVER_OK`, section 2.1.2
    /// forbidding the device to have used anything before then.
    /// [`QueueError::Region`] when the used ring or the descriptor table
    /// is too short. [`QueueError::UsedIndex`] when the used index moved
    /// backwards or past the outstanding chains.
    /// [`QueueError::UnknownDescriptor`] when the element names a
    /// descriptor outside the table or one that is already free.
    /// [`QueueError::CorruptChain`] when the chain does not end where it
    /// should; the element is consumed either way, so the queue does not
    /// stall on it, and what the walk reached is back in the free set.
    /// [`QueueError::UsedLength`] when the device reports more bytes
    /// written than the chain gave it room for; the chain is freed and
    /// the element consumed, and only the number is refused.
    pub fn next_used(
        &mut self,
        device: &Device,
        memory: &impl QueueMemory,
    ) -> Result<Option<Completion>, QueueError> {
        if !device.is_live() {
            return Err(QueueError::NotLive);
        }
        let used = memory.used_ring();
        let index = read_u16(used, Area::UsedRing, INDEX_AT)?;
        let pending = index.wrapping_sub(self.last_used);
        if pending == 0 {
            return Ok(None);
        }
        if pending > self.in_flight {
            return Err(QueueError::UsedIndex {
                last: self.last_used,
                now: index,
                in_flight: self.in_flight,
            });
        }
        // The barrier belongs between the index and the element it
        // names, not before the index: what has to be ordered is the
        // device's write of the element against its write of the index,
        // so a driver that has seen the index has to see the element.
        memory.barrier();
        let at = used_element_at(self.last_used & self.mask());
        let id = read_u32(used, Area::UsedRing, at)?;
        let length = read_u32(used, Area::UsedRing, at.saturating_add(USED_LENGTH_AT))?;
        let head = u16::try_from(id).unwrap_or(u16::MAX);
        if head >= self.size || self.free.test(usize::from(head)).unwrap_or(true) {
            return Err(QueueError::UnknownDescriptor(id));
        }
        self.last_used = self.last_used.wrapping_add(1);
        self.in_flight = self.in_flight.saturating_sub(1);
        let writable = self.free_chain(memory, head)?;
        if length > writable {
            return Err(QueueError::UsedLength {
                reported: length,
                writable,
            });
        }
        Ok(Some(Completion { head, length }))
    }

    /// Writes the head into the available ring and publishes it.
    fn publish(&mut self, memory: &mut impl QueueMemory, head: u16) -> Result<(), QueueError> {
        // Steps 2 and 5 of section 2.7.13, with the barrier of step 4
        // between them.
        let slot = self.avail_idx & self.mask();
        let available = memory.available_ring_mut();
        write_u16(
            available,
            Area::AvailableRing,
            available_element_at(slot),
            head,
        )?;
        memory.barrier();
        let next = self.avail_idx.wrapping_add(1);
        // The counters move with the write and not before it: a queue
        // that failed to publish must not believe it did.
        write_u16(
            memory.available_ring_mut(),
            Area::AvailableRing,
            INDEX_AT,
            next,
        )
        .map(|()| {
            self.avail_idx = next;
            self.in_flight = self.in_flight.saturating_add(1);
        })
    }

    /// Takes one free descriptor, or `None` when none is left.
    fn take_free(&mut self) -> Option<u16> {
        let index = self.free.first_set()?;
        let taken = u16::try_from(index).unwrap_or(u16::MAX);
        self.free.clear(index).ok().map(|_| taken)
    }

    /// Gives every descriptor of the chain at `head` back to the free
    /// set, and adds up how many bytes of it the device was given to
    /// write.
    ///
    /// The walk needs no step counter. Every step frees one descriptor
    /// that was in use, there are at most `size` of those, and a step
    /// onto one that is already free is refused, so the loop cannot run
    /// longer than the queue is wide.
    ///
    /// The sum saturates. A chain whose writable descriptors add up past
    /// four gigabytes bounds nothing in practice, and saturating there
    /// says so without a branch that no queue can reach.
    fn free_chain(&mut self, memory: &impl QueueMemory, head: u16) -> Result<u32, QueueError> {
        let table = memory.descriptor_table();
        let mut current = head;
        let mut writable: u32 = 0;
        loop {
            if current >= self.size || self.free.set(usize::from(current)).unwrap_or(true) {
                return Err(QueueError::CorruptChain(head));
            }
            let (length, flags, next) = Descriptor::step_of(table, current)?;
            if flags & DESC_F_WRITE != 0 {
                writable = writable.saturating_add(length);
            }
            if flags & DESC_F_NEXT == 0 {
                return Ok(writable);
            }
            current = next;
        }
    }

    /// The mask that turns a free-running index into a ring slot. The
    /// size is a power of two, so the wrap of the index at `2^16` and the
    /// wrap of the slot at the size agree.
    const fn mask(&self) -> u16 {
        self.size.wrapping_sub(1)
    }

    /// Refuses a descriptor table shorter than the queue.
    fn check_table(&self, memory: &impl QueueMemory) -> Result<(), QueueError> {
        let needed = descriptor_table_bytes(self.size);
        let given = memory.descriptor_table().len();
        if given < needed {
            return Err(QueueError::Region {
                area: Area::DescriptorTable,
                needed,
                given,
            });
        }
        Ok(())
    }
}

/// The byte the available ring entry in `slot` begins at.
#[expect(clippy::as_conversions, reason = "widening a u16 in a const fn")]
const fn available_element_at(slot: u16) -> usize {
    RING_HEADER_BYTES.saturating_add((slot as usize).saturating_mul(2))
}

/// The byte the used element in `slot` begins at.
#[expect(clippy::as_conversions, reason = "widening a u16 in a const fn")]
const fn used_element_at(slot: u16) -> usize {
    RING_HEADER_BYTES.saturating_add((slot as usize).saturating_mul(USED_ELEMENT_BYTES))
}
