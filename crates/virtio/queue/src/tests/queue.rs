// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The ring: what a chain looks like in memory, what the free set does
//! when it runs out, and what the driver refuses to believe about the
//! used ring.

use crate::descriptor::{Buffer, DESC_F_NEXT, DESC_F_WRITE};
use crate::device::{Device, F_VERSION_1, Phase};
use crate::doubles::{RamQueue, ScriptedRegisters};
use crate::error::{Area, QueueError};
use crate::memory::{QueueMemory, available_ring_bytes, descriptor_table_bytes, used_ring_bytes};
use crate::queue::{AVAIL_F_NO_INTERRUPT, Completion, Queue, USED_F_NO_NOTIFY};
use crate::tests::support::{chain_of, live_device, to_device};

/// A queue of eight descriptors over memory sized for it, and a device
/// that has reached `DRIVER_OK`.
fn ready() -> (Device, RamQueue, Queue<1>) {
    let (device, _) = live_device();
    let mut memory = RamQueue::new(8);
    let queue = Queue::<1>::new(&mut memory, 8).expect("queue of eight");
    (device, memory, queue)
}

#[test]
fn a_queue_starts_with_every_descriptor_free_and_an_empty_ring() {
    let (_, memory, queue) = ready();
    assert_eq!(queue.size(), 8);
    assert_eq!(queue.free_count(), 8);
    assert_eq!(queue.in_flight(), 0);
    assert_eq!(queue.available_index(), 0);
    assert_eq!(queue.last_used(), 0);
    assert_eq!(memory.available_index(), 0);
    assert_eq!(memory.available_flags(), 0);
}

#[test]
fn a_size_that_is_not_a_power_of_two_is_refused() {
    let mut memory = RamQueue::new(8);
    assert_eq!(Queue::<1>::new(&mut memory, 0), Err(QueueError::Size(0)));
    assert_eq!(Queue::<1>::new(&mut memory, 3), Err(QueueError::Size(3)));
    assert_eq!(
        Queue::<1>::new(&mut memory, u16::MAX),
        Err(QueueError::Size(u16::MAX))
    );
}

#[test]
fn a_size_wider_than_the_free_set_is_refused() {
    let mut memory = RamQueue::new(128);
    assert_eq!(
        Queue::<1>::new(&mut memory, 128),
        Err(QueueError::Size(128))
    );
    assert!(Queue::<2>::new(&mut memory, 128).is_ok());
}

#[test]
fn the_largest_queue_a_u16_allows_is_the_one_the_specification_allows() {
    let mut memory = RamQueue::new(32768);
    let queue = Queue::<512>::new(&mut memory, 32768).expect("the largest queue");
    assert_eq!(queue.size(), 32768);
    assert_eq!(queue.free_count(), 32768);
}

#[test]
fn a_single_descriptor_request_is_one_descriptor_and_one_ring_entry() {
    let (device, mut memory, mut queue) = ready();
    let head = queue
        .add(&device, &mut memory, &to_device(1))
        .expect("one buffer");

    let descriptor = memory.descriptor(head).expect("written");
    assert_eq!(descriptor.address, 0x1000);
    assert_eq!(descriptor.length, 64);
    assert_eq!(descriptor.flags, 0);
    assert!(!descriptor.has_next());

    assert_eq!(memory.available_entry(0), head);
    assert_eq!(memory.available_index(), 1);
    assert_eq!(queue.available_index(), 1);
    assert_eq!(queue.free_count(), 7);
    assert_eq!(queue.in_flight(), 1);
}

#[test]
fn a_chain_of_three_links_every_descriptor_but_the_last() {
    let (device, mut memory, mut queue) = ready();
    let buffers = [
        Buffer::to_device(0x1000, 16),
        Buffer::to_device(0x2000, 32),
        Buffer::from_device(0x3000, 64),
    ];
    let head = queue.add(&device, &mut memory, &buffers).expect("three");

    let chain = chain_of(&memory, head);
    assert_eq!(chain.len(), 3);
    assert_eq!(queue.free_count(), 5);

    for (index, (position, buffer)) in chain.iter().zip(buffers.iter()).enumerate() {
        let descriptor = memory.descriptor(*position).expect("written");
        assert_eq!(descriptor.address, buffer.address);
        assert_eq!(descriptor.length, buffer.length);
        assert_eq!(descriptor.flags & DESC_F_WRITE, buffer.direction.flag());
        assert_eq!(descriptor.has_next(), index < 2);
    }
    assert_eq!(memory.available_entry(0), head);
    assert_eq!(memory.available_index(), 1);
}

#[test]
fn a_chain_that_takes_every_descriptor_leaves_none_for_the_next() {
    let (device, mut memory, mut queue) = ready();
    let head = queue
        .add(&device, &mut memory, &to_device(8))
        .expect("all eight");
    assert_eq!(chain_of(&memory, head).len(), 8);
    assert_eq!(queue.free_count(), 0);

    assert_eq!(
        queue.add(&device, &mut memory, &to_device(1)),
        Err(QueueError::QueueFull { free: 0, needed: 1 })
    );
    assert_eq!(queue.in_flight(), 1);
}

#[test]
fn a_chain_that_exhausts_the_free_list_gives_back_what_it_had_taken() {
    let (device, mut memory, mut queue) = ready();
    queue
        .add(&device, &mut memory, &to_device(5))
        .expect("five of eight");
    assert_eq!(queue.free_count(), 3);

    assert_eq!(
        queue.add(&device, &mut memory, &to_device(5)),
        Err(QueueError::QueueFull { free: 3, needed: 5 }),
        "the count is the one after the half-built chain came back"
    );
    assert_eq!(queue.free_count(), 3, "the half-built chain came back");
    assert_eq!(queue.in_flight(), 1);
    assert_eq!(memory.available_index(), 1, "nothing was published");

    queue
        .add(&device, &mut memory, &to_device(3))
        .expect("the rest still fits");
    assert_eq!(queue.free_count(), 0);
}

#[test]
fn a_chain_longer_than_the_queue_is_refused_before_a_descriptor_is_taken() {
    let (device, mut memory, mut queue) = ready();
    assert_eq!(
        queue.add(&device, &mut memory, &to_device(9)),
        Err(QueueError::ChainTooLong {
            buffers: 9,
            size: 8
        })
    );
    assert_eq!(queue.free_count(), 8);
    assert_eq!(queue.in_flight(), 0);
}

#[test]
fn a_buffer_the_device_writes_may_not_stand_before_one_it_reads() {
    let (device, mut memory, mut queue) = ready();
    let wrong = [
        Buffer::from_device(0x1000, 16),
        Buffer::to_device(0x2000, 16),
    ];
    assert_eq!(
        queue.add(&device, &mut memory, &wrong),
        Err(QueueError::BufferOrder)
    );
    assert_eq!(queue.free_count(), 8, "nothing was taken");

    let right = [
        Buffer::to_device(0x2000, 16),
        Buffer::from_device(0x1000, 16),
        Buffer::from_device(0x3000, 16),
    ];
    let head = queue.add(&device, &mut memory, &right).expect("in order");
    let chain = chain_of(&memory, head);
    let flags: Vec<u16> = chain
        .iter()
        .filter_map(|index| memory.descriptor(*index))
        .map(|descriptor| descriptor.flags & DESC_F_WRITE)
        .collect();
    assert_eq!(flags, [0, DESC_F_WRITE, DESC_F_WRITE]);
}

#[test]
fn a_chain_of_no_buffers_is_refused() {
    let (device, mut memory, mut queue) = ready();
    assert_eq!(
        queue.add(&device, &mut memory, &[]),
        Err(QueueError::EmptyChain)
    );
    assert_eq!(queue.free_count(), 8);
}

#[test]
fn a_completed_chain_comes_back_whole_and_its_descriptors_are_free_again() {
    let (device, mut memory, mut queue) = ready();
    let buffers = [
        Buffer::to_device(0x1000, 16),
        Buffer::from_device(0x2000, 64),
        Buffer::from_device(0x3000, 1),
    ];
    let head = queue.add(&device, &mut memory, &buffers).expect("three");
    assert_eq!(queue.next_used(&device, &memory), Ok(None));

    memory.complete(u32::from(head), 48);
    assert_eq!(
        queue.next_used(&device, &memory),
        Ok(Some(Completion { head, length: 48 }))
    );
    assert_eq!(queue.free_count(), 8);
    assert_eq!(queue.in_flight(), 0);
    assert_eq!(queue.last_used(), 1);
    assert_eq!(queue.next_used(&device, &memory), Ok(None));
}

#[test]
fn chains_may_come_back_in_an_order_the_driver_did_not_add_them_in() {
    let (device, mut memory, mut queue) = ready();
    let first = queue
        .add(&device, &mut memory, &to_device(2))
        .expect("first");
    let second = queue
        .add(&device, &mut memory, &to_device(2))
        .expect("second");
    assert_eq!(queue.free_count(), 4);

    memory.complete(u32::from(second), 0);
    memory.complete(u32::from(first), 0);
    assert_eq!(
        queue.next_used(&device, &memory),
        Ok(Some(Completion {
            head: second,
            length: 0
        }))
    );
    assert_eq!(
        queue.next_used(&device, &memory),
        Ok(Some(Completion {
            head: first,
            length: 0
        }))
    );
    assert_eq!(queue.free_count(), 8);
}

#[test]
fn the_available_index_wraps_at_two_to_the_sixteen_and_the_slot_at_the_size() {
    let (device, mut memory, mut queue) = ready();
    for round in 0..70_000u32 {
        let head = queue
            .add(&device, &mut memory, &to_device(1))
            .expect("one buffer");
        let expected = u16::try_from(round.wrapping_add(1) & 0xFFFF).unwrap_or(0);
        assert_eq!(queue.available_index(), expected);
        assert_eq!(memory.available_index(), expected);
        let slot = u16::try_from(round & 0x7).unwrap_or(0);
        assert_eq!(memory.available_entry(slot), head);

        memory.complete(u32::from(head), 0);
        assert_eq!(
            queue.next_used(&device, &memory),
            Ok(Some(Completion { head, length: 0 }))
        );
    }
    assert_eq!(
        queue.available_index(),
        4_464,
        "70_000 past the wrap at 65_536"
    );
    assert_eq!(queue.free_count(), 8);
    assert_eq!(queue.in_flight(), 0);
}

#[test]
fn a_used_element_naming_a_descriptor_outside_the_table_is_refused() {
    let (device, mut memory, mut queue) = ready();
    queue.add(&device, &mut memory, &to_device(1)).expect("one");

    memory.complete(8, 0);
    assert_eq!(
        queue.next_used(&device, &memory),
        Err(QueueError::UnknownDescriptor(8))
    );
    memory.set_used_index(0);
    memory.complete(0x1_0000, 0);
    assert_eq!(
        queue.next_used(&device, &memory),
        Err(QueueError::UnknownDescriptor(0x1_0000))
    );
}

#[test]
fn a_used_element_naming_a_descriptor_that_is_already_free_is_refused() {
    let (device, mut memory, mut queue) = ready();
    let head = queue.add(&device, &mut memory, &to_device(1)).expect("one");
    queue.add(&device, &mut memory, &to_device(1)).expect("two");

    memory.complete(u32::from(head), 0);
    memory.complete(u32::from(head), 0);
    assert_eq!(
        queue.next_used(&device, &memory),
        Ok(Some(Completion { head, length: 0 }))
    );
    assert_eq!(
        queue.next_used(&device, &memory),
        Err(QueueError::UnknownDescriptor(u32::from(head)))
    );
}

/// The shape a block request has: a header the device reads, a payload it
/// writes, and a status byte it writes. Sixty-five bytes are writable.
fn block_request() -> [Buffer; 3] {
    [
        Buffer::to_device(0x1000, 16),
        Buffer::from_device(0x2000, 64),
        Buffer::from_device(0x3000, 1),
    ]
}

#[test]
fn a_device_reporting_no_more_than_it_was_given_room_for_is_believed() {
    let (device, mut memory, mut queue) = ready();

    // Exactly the writable bytes of the chain.
    let head = queue
        .add(&device, &mut memory, &block_request())
        .expect("a request");
    memory.complete(u32::from(head), 65);
    assert_eq!(
        queue.next_used(&device, &memory),
        Ok(Some(Completion { head, length: 65 }))
    );

    // Fewer than it wrote, which section 2.7.8.2 permits a device that
    // failed part way and does not know how far it got.
    let head = queue
        .add(&device, &mut memory, &block_request())
        .expect("a request");
    memory.complete(u32::from(head), 12);
    assert_eq!(
        queue.next_used(&device, &memory),
        Ok(Some(Completion { head, length: 12 }))
    );
    assert_eq!(queue.free_count(), 8);
}

#[test]
fn a_device_reporting_more_than_it_was_given_room_for_is_refused() {
    let (device, mut memory, mut queue) = ready();
    let head = queue
        .add(&device, &mut memory, &block_request())
        .expect("a request");

    memory.complete(u32::from(head), 66);
    assert_eq!(
        queue.next_used(&device, &memory),
        Err(QueueError::UsedLength {
            reported: 66,
            writable: 65,
        })
    );
    assert_eq!(
        queue.free_count(),
        8,
        "the chain is done either way; it is the number that is refused"
    );
    assert_eq!(queue.in_flight(), 0);
    assert_eq!(queue.last_used(), 1, "the queue does not stall on it");
}

#[test]
fn a_chain_the_device_only_reads_may_report_nothing_and_nothing_else() {
    let (device, mut memory, mut queue) = ready();
    let head = queue.add(&device, &mut memory, &to_device(2)).expect("two");
    memory.complete(u32::from(head), 1);
    assert_eq!(
        queue.next_used(&device, &memory),
        Err(QueueError::UsedLength {
            reported: 1,
            writable: 0,
        })
    );

    let head = queue.add(&device, &mut memory, &to_device(2)).expect("two");
    memory.complete(u32::from(head), 0);
    assert_eq!(
        queue.next_used(&device, &memory),
        Ok(Some(Completion { head, length: 0 }))
    );
}

#[test]
fn only_the_writable_descriptors_of_a_chain_count_towards_the_room() {
    let (device, mut memory, mut queue) = ready();
    // Two kilobytes the device reads, sixty-four bytes it writes.
    let buffers = [
        Buffer::to_device(0x1000, 2048),
        Buffer::from_device(0x2000, 64),
    ];
    let head = queue.add(&device, &mut memory, &buffers).expect("two");

    memory.complete(u32::from(head), 100);
    assert_eq!(
        queue.next_used(&device, &memory),
        Err(QueueError::UsedLength {
            reported: 100,
            writable: 64,
        })
    );
}

#[test]
fn a_used_index_that_went_backwards_is_refused() {
    let (device, mut memory, mut queue) = ready();
    let head = queue.add(&device, &mut memory, &to_device(1)).expect("one");
    memory.complete(u32::from(head), 0);
    queue.next_used(&device, &memory).expect("the completion");

    memory.set_used_index(0);
    assert_eq!(
        queue.next_used(&device, &memory),
        Err(QueueError::UsedIndex {
            last: 1,
            now: 0,
            in_flight: 0,
        })
    );
}

#[test]
fn a_used_index_past_the_outstanding_chains_is_refused() {
    let (device, mut memory, mut queue) = ready();
    queue.add(&device, &mut memory, &to_device(1)).expect("one");

    memory.set_used_index(5);
    assert_eq!(
        queue.next_used(&device, &memory),
        Err(QueueError::UsedIndex {
            last: 0,
            now: 5,
            in_flight: 1,
        })
    );
}

#[test]
fn the_no_notify_flag_stops_notifications_and_clearing_it_resumes_them() {
    let (device, mut memory, queue) = ready();
    assert!(queue.should_notify(&device, &memory).expect("read"));

    memory.set_no_notify(true);
    assert!(!queue.should_notify(&device, &memory).expect("read"));
    assert_eq!(
        crate::memory::read_u16(memory.used_ring(), Area::UsedRing, 0),
        Ok(USED_F_NO_NOTIFY)
    );

    memory.set_no_notify(false);
    assert!(queue.should_notify(&device, &memory).expect("read"));
}

#[test]
fn the_driver_can_ask_the_device_not_to_interrupt_it() {
    let (device, mut memory, queue) = ready();
    queue
        .suppress_interrupts(&device, &mut memory, true)
        .expect("write");
    assert_eq!(memory.available_flags(), AVAIL_F_NO_INTERRUPT);

    queue
        .suppress_interrupts(&device, &mut memory, false)
        .expect("write");
    assert_eq!(memory.available_flags(), 0);
}

#[test]
fn publishing_and_reaping_ask_for_a_barrier() {
    let (device, mut memory, mut queue) = ready();
    let head = queue.add(&device, &mut memory, &to_device(1)).expect("one");
    assert_eq!(memory.barriers(), 1, "between the entry and the index");

    memory.complete(u32::from(head), 0);
    queue.next_used(&device, &memory).expect("the completion");
    assert_eq!(memory.barriers(), 2, "before the used index is read");
}

#[test]
fn nothing_reaches_a_queue_before_the_feature_set_is_settled() {
    let device = Device::new();
    let mut memory = RamQueue::new(8);
    let mut queue = Queue::<1>::new(&mut memory, 8).expect("queue");

    assert_eq!(
        queue.add(&device, &mut memory, &to_device(1)),
        Err(QueueError::NotConfigured)
    );
    assert_eq!(
        queue.suppress_interrupts(&device, &mut memory, true),
        Err(QueueError::NotConfigured)
    );
    assert_eq!(queue.next_used(&device, &memory), Err(QueueError::NotLive));
    assert_eq!(
        queue.should_notify(&device, &memory),
        Err(QueueError::NotLive)
    );
    assert_eq!(queue.free_count(), 8);
}

#[test]
fn a_queue_is_filled_in_step_seven_and_read_only_once_the_device_is_live() {
    let mut registers = ScriptedRegisters::new(F_VERSION_1);
    let mut device = Device::new();
    device.acknowledge(&mut registers).expect("acknowledge");
    device.driver(&mut registers).expect("driver");
    device.negotiate(&mut registers, 0).expect("negotiate");
    assert_eq!(device.phase(), Phase::FeaturesOk);
    assert!(!device.is_live());

    let mut memory = RamQueue::new(8);
    let mut queue = Queue::<1>::new(&mut memory, 8).expect("queue");

    // Step 7 of section 3.1.1: the receive queue is filled before the
    // device is live.
    let head = queue
        .add(&device, &mut memory, &[Buffer::from_device(0x4000, 2048)])
        .expect("a buffer goes in before DRIVER_OK");
    assert_eq!(memory.available_index(), 1);
    assert_eq!(memory.available_entry(0), head);
    queue
        .suppress_interrupts(&device, &mut memory, true)
        .expect("interrupts are settled before DRIVER_OK");

    // The notification and the used ring stay behind DRIVER_OK.
    assert_eq!(
        queue.should_notify(&device, &memory),
        Err(QueueError::NotLive)
    );
    assert_eq!(queue.next_used(&device, &memory), Err(QueueError::NotLive));

    device.driver_ok(&mut registers).expect("driver ok");
    assert!(queue.should_notify(&device, &memory).expect("now readable"));
    memory.complete(u32::from(head), 128);
    assert_eq!(
        queue.next_used(&device, &memory),
        Ok(Some(Completion { head, length: 128 }))
    );
}

#[test]
fn a_chain_that_loops_back_on_itself_is_refused_after_what_it_reached_came_back() {
    let (device, mut memory, mut queue) = ready();
    let head = queue.add(&device, &mut memory, &to_device(2)).expect("two");
    let chain = chain_of(&memory, head);
    let tail = *chain.last().expect("a tail");
    memory.scribble_link(tail, DESC_F_NEXT, head);

    memory.complete(u32::from(head), 0);
    assert_eq!(
        queue.next_used(&device, &memory),
        Err(QueueError::CorruptChain(head))
    );
    assert_eq!(queue.in_flight(), 0, "the element was consumed");
    assert_eq!(queue.last_used(), 1);
    assert_eq!(
        queue.free_count(),
        8,
        "the walk reached both descriptors before it found the loop"
    );
}

#[test]
fn a_chain_that_leaves_the_table_is_refused() {
    let (device, mut memory, mut queue) = ready();
    let head = queue.add(&device, &mut memory, &to_device(1)).expect("one");
    memory.scribble_link(head, DESC_F_NEXT, 200);

    memory.complete(u32::from(head), 0);
    assert_eq!(
        queue.next_used(&device, &memory),
        Err(QueueError::CorruptChain(head))
    );
}

#[test]
fn a_chain_leaving_the_queue_is_refused_even_when_the_free_set_is_wider() {
    let (device, _) = live_device();
    let mut memory = RamQueue::new(8);
    let mut queue = Queue::<2>::new(&mut memory, 8).expect("eight of a hundred and twenty-eight");
    let head = queue.add(&device, &mut memory, &to_device(1)).expect("one");

    // 100 is a bit the free set has and a descriptor the queue has not.
    memory.scribble_link(head, DESC_F_NEXT, 100);
    memory.complete(u32::from(head), 0);
    assert_eq!(
        queue.next_used(&device, &memory),
        Err(QueueError::CorruptChain(head))
    );
}

#[test]
fn a_descriptor_table_too_short_for_the_queue_costs_no_descriptor() {
    let (device, _) = live_device();
    let mut memory = RamQueue::with_lengths(8, 16, available_ring_bytes(8), used_ring_bytes(8));
    let mut queue = Queue::<1>::new(&mut memory, 8).expect("queue");

    assert_eq!(
        queue.add(&device, &mut memory, &to_device(1)),
        Err(QueueError::Region {
            area: Area::DescriptorTable,
            needed: descriptor_table_bytes(8),
            given: 16,
        })
    );
    assert_eq!(queue.free_count(), 8);
}

#[test]
fn an_available_ring_that_cannot_hold_its_own_header_is_refused_at_construction() {
    let mut memory = RamQueue::with_lengths(8, descriptor_table_bytes(8), 1, used_ring_bytes(8));
    assert_eq!(
        Queue::<1>::new(&mut memory, 8),
        Err(QueueError::Region {
            area: Area::AvailableRing,
            needed: 2,
            given: 1,
        })
    );

    let mut memory = RamQueue::with_lengths(8, descriptor_table_bytes(8), 3, used_ring_bytes(8));
    assert_eq!(
        Queue::<1>::new(&mut memory, 8),
        Err(QueueError::Region {
            area: Area::AvailableRing,
            needed: 4,
            given: 3,
        })
    );
}

#[test]
fn an_available_ring_that_shrinks_after_construction_is_refused_when_it_is_written() {
    let (device, mut memory, mut queue) = ready();
    memory.truncate_available(5);
    assert_eq!(
        queue.add(&device, &mut memory, &to_device(1)),
        Err(QueueError::Region {
            area: Area::AvailableRing,
            needed: 6,
            given: 5,
        })
    );

    let (device, mut memory, queue) = ready();
    memory.truncate_available(1);
    assert_eq!(
        queue.suppress_interrupts(&device, &mut memory, true),
        Err(QueueError::Region {
            area: Area::AvailableRing,
            needed: 2,
            given: 1,
        })
    );
}

#[test]
fn a_chain_that_could_not_be_published_costs_neither_a_descriptor_nor_a_count() {
    let (device, mut memory, mut queue) = ready();
    memory.truncate_available(5);
    assert_eq!(
        queue.add(&device, &mut memory, &to_device(2)),
        Err(QueueError::Region {
            area: Area::AvailableRing,
            needed: 6,
            given: 5,
        })
    );
    assert_eq!(queue.free_count(), 8, "the chain came back");
    assert_eq!(queue.in_flight(), 0);
    assert_eq!(queue.available_index(), 0, "nothing was published");
}

#[test]
fn a_used_ring_shorter_than_the_field_being_read_is_refused() {
    let (device, _) = live_device();
    let mut memory = RamQueue::with_lengths(8, descriptor_table_bytes(8), 6, 0);
    let queue = Queue::<1>::new(&mut memory, 8).expect("queue");
    assert_eq!(
        queue.should_notify(&device, &memory),
        Err(QueueError::Region {
            area: Area::UsedRing,
            needed: 2,
            given: 0,
        })
    );

    let mut memory =
        RamQueue::with_lengths(8, descriptor_table_bytes(8), available_ring_bytes(8), 3);
    let mut queue = Queue::<1>::new(&mut memory, 8).expect("queue");
    assert_eq!(
        queue.next_used(&device, &memory),
        Err(QueueError::Region {
            area: Area::UsedRing,
            needed: 4,
            given: 3,
        })
    );
}

#[test]
fn a_used_ring_that_holds_the_index_but_not_the_element_is_refused() {
    let (device, _) = live_device();
    let mut memory =
        RamQueue::with_lengths(8, descriptor_table_bytes(8), available_ring_bytes(8), 8);
    let mut queue = Queue::<1>::new(&mut memory, 8).expect("queue");
    queue.add(&device, &mut memory, &to_device(1)).expect("one");
    memory.set_used_index(1);

    assert_eq!(
        queue.next_used(&device, &memory),
        Err(QueueError::Region {
            area: Area::UsedRing,
            needed: 12,
            given: 8,
        })
    );

    let mut memory =
        RamQueue::with_lengths(8, descriptor_table_bytes(8), available_ring_bytes(8), 10);
    let mut queue = Queue::<1>::new(&mut memory, 8).expect("queue");
    queue.add(&device, &mut memory, &to_device(1)).expect("one");
    memory.set_used_index(1);
    assert_eq!(
        queue.next_used(&device, &memory),
        Err(QueueError::Region {
            area: Area::UsedRing,
            needed: 12,
            given: 10,
        })
    );
}

#[test]
fn a_descriptor_table_that_shrinks_before_the_chain_is_freed_is_refused() {
    let (device, mut memory, mut queue) = ready();
    let head = queue.add(&device, &mut memory, &to_device(1)).expect("one");
    memory.complete(u32::from(head), 0);
    memory.truncate_descriptors(4);

    assert!(matches!(
        queue.next_used(&device, &memory),
        Err(QueueError::Region {
            area: Area::DescriptorTable,
            ..
        })
    ));
}
