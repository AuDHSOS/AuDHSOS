// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::blk`.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::indexing_slicing
)]

use virtio_queue::{
    DeviceError, F_VERSION_1, Phase, QueueError, STATUS_ACKNOWLEDGE, STATUS_DRIVER,
    STATUS_DRIVER_OK, STATUS_FEATURES_OK,
};

use crate::blk::{Blk, Chain, ISR_CONFIG, ISR_QUEUE, Rings, Segment, Vectors};
use crate::common::{self, NO_VECTOR};
use crate::config::SECTOR_LEN;
use crate::doubles::{RamDevice, Refusal};
use crate::error::BlkError;
use crate::features::{F_FLUSH, F_RO, F_SEG_MAX, WANTED};
use crate::registers::{Registers, Structure, Width};
use crate::request::{HEADER_LEN, Kind, Request, STATUS_LEN};

use super::support::{
    CAPACITY, MULTIPLIER, OFFERED, QUEUE_SIZE, RINGS, VECTORS, device, live, queue, refusal,
};

/// A chain over addresses that only have to arrive unchanged.
const CHAIN: Chain = Chain {
    header: 0x2_0000,
    data: Some(Segment {
        address: 0x2_1000,
        length: SECTOR_LEN,
    }),
    status: 0x2_2000,
};

/// The same chain with no data, which is what a flush carries.
const FLUSH_CHAIN: Chain = Chain {
    header: CHAIN.header,
    data: None,
    status: CHAIN.status,
};

#[test]
fn a_device_is_brought_up_in_the_order_the_specification_gives() {
    let mut registers = device();
    let blk = live(&mut registers);
    let status: Vec<u64> = registers
        .writes()
        .iter()
        .filter(|(structure, offset, _)| {
            *structure == Structure::Common && *offset == common::DEVICE_STATUS
        })
        .map(|(_, _, value)| *value)
        .collect();
    assert_eq!(
        status,
        vec![
            0,
            u64::from(STATUS_ACKNOWLEDGE),
            u64::from(STATUS_ACKNOWLEDGE | STATUS_DRIVER),
            u64::from(STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK),
            u64::from(STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK),
        ]
    );
    assert_eq!(blk.device().phase(), Phase::Live);
}

#[test]
fn what_the_device_offers_and_the_driver_wants_is_what_is_taken() {
    let mut registers = device();
    let blk = live(&mut registers);
    assert_eq!(blk.features(), F_VERSION_1 | F_FLUSH);
    assert_eq!(registers.accepted(), F_VERSION_1 | F_FLUSH);
    assert!(blk.flushes());
    assert!(!blk.is_read_only(), "the device offered no read-only bit");
    assert_eq!(crate::features::refused(OFFERED) & F_SEG_MAX, F_SEG_MAX);
}

#[test]
fn the_queue_the_rings_and_the_vectors_reach_the_device() {
    let mut registers = device();
    let blk = live(&mut registers);
    assert_eq!(blk.queue_size(), QUEUE_SIZE);
    assert_eq!(blk.capacity(), CAPACITY);
    assert_eq!(
        registers.common_field(common::QUEUE_DESC, Width::B64),
        RINGS.descriptor_table
    );
    assert_eq!(
        registers.common_field(common::QUEUE_DRIVER, Width::B64),
        RINGS.available_ring
    );
    assert_eq!(
        registers.common_field(common::QUEUE_DEVICE, Width::B64),
        RINGS.used_ring
    );
    assert_eq!(
        registers.common_field(common::QUEUE_MSIX_VECTOR, Width::B16),
        u64::from(VECTORS.queue)
    );
    assert_eq!(
        registers.common_field(common::CONFIG_MSIX_VECTOR, Width::B16),
        u64::from(VECTORS.config)
    );
    assert_eq!(
        registers.common_field(common::QUEUE_ENABLE, Width::B16),
        u64::from(common::ENABLED)
    );
}

#[test]
fn a_device_offering_a_smaller_queue_settles_the_size() {
    let mut registers = RamDevice::new(OFFERED, 4, CAPACITY);
    let mut blk = Blk::new(MULTIPLIER);
    blk.reset(&mut registers);
    blk.initialize(&mut registers, &RINGS, &VECTORS)
        .expect("the device comes up");
    assert_eq!(blk.queue_size(), 4);
    assert_eq!(registers.common_field(common::QUEUE_SIZE, Width::B16), 4);
}

#[test]
fn a_device_with_no_request_queue_is_refused() {
    let mut registers = RamDevice::new(OFFERED, 0, CAPACITY);
    let mut blk = Blk::new(MULTIPLIER);
    blk.reset(&mut registers);
    assert_eq!(
        refusal(blk.initialize(&mut registers, &RINGS, &VECTORS)),
        BlkError::NoQueue
    );
}

#[test]
fn rings_of_a_size_that_is_not_a_power_of_two_are_refused() {
    let mut registers = device();
    let mut blk = Blk::new(MULTIPLIER);
    blk.reset(&mut registers);
    let rings = Rings { size: 6, ..RINGS };
    assert_eq!(
        refusal(blk.initialize(&mut registers, &rings, &VECTORS)),
        BlkError::QueueSize(6)
    );
    let none = Rings { size: 0, ..RINGS };
    assert_eq!(
        refusal(blk.initialize(&mut registers, &none, &VECTORS)),
        BlkError::QueueSize(0)
    );
}

#[test]
fn a_device_that_was_not_reset_is_refused_rather_than_waited_for() {
    let mut registers = device();
    let mut blk = Blk::new(MULTIPLIER);
    registers.write(
        Structure::Common,
        common::DEVICE_STATUS,
        Width::B8,
        u64::from(STATUS_ACKNOWLEDGE),
    );
    assert!(!blk.is_reset(&registers));
    assert_eq!(
        refusal(blk.initialize(&mut registers, &RINGS, &VECTORS)),
        BlkError::NotReset(STATUS_ACKNOWLEDGE)
    );
}

#[test]
fn a_reset_puts_everything_the_driver_learned_back() {
    let mut registers = device();
    let mut blk = live(&mut registers);
    assert_ne!(blk.capacity(), 0);
    blk.reset(&mut registers);
    assert_eq!(blk.capacity(), 0);
    assert_eq!(blk.features(), 0);
    assert_eq!(blk.queue_size(), 0);
    assert_eq!(blk.device().phase(), Phase::Reset);
    assert!(blk.is_reset(&registers));
    assert_eq!(blk.status(&registers), 0);
}

#[test]
fn a_device_that_will_not_take_the_vector_is_refused() {
    let mut registers = RamDevice::new(OFFERED, QUEUE_SIZE, CAPACITY).refusing(Refusal::Vectors);
    let mut blk = Blk::new(MULTIPLIER);
    blk.reset(&mut registers);
    assert_eq!(
        refusal(blk.initialize(&mut registers, &RINGS, &VECTORS)),
        BlkError::NoVector(VECTORS.queue)
    );
}

#[test]
fn a_device_driven_without_message_interrupts_writes_no_vector() {
    let mut registers = RamDevice::new(OFFERED, QUEUE_SIZE, CAPACITY).refusing(Refusal::Vectors);
    let mut blk = Blk::new(MULTIPLIER);
    blk.reset(&mut registers);
    blk.initialize(&mut registers, &RINGS, &Vectors::none())
        .expect("no vector is no refusal");
    assert_eq!(Vectors::none().queue, NO_VECTOR);
    assert_eq!(Vectors::none().config, NO_VECTOR);
}

#[test]
fn a_device_that_clears_features_ok_is_refused() {
    let mut registers = RamDevice::new(OFFERED, QUEUE_SIZE, CAPACITY).refusing(Refusal::Features);
    let mut blk = Blk::new(MULTIPLIER);
    blk.reset(&mut registers);
    assert_eq!(
        refusal(blk.initialize(&mut registers, &RINGS, &VECTORS)),
        BlkError::Device(DeviceError::FeaturesRejected)
    );
}

#[test]
fn a_device_that_is_not_virtio_one_is_refused() {
    let mut registers = RamDevice::new(F_FLUSH, QUEUE_SIZE, CAPACITY);
    let mut blk = Blk::new(MULTIPLIER);
    blk.reset(&mut registers);
    assert_eq!(
        refusal(blk.initialize(&mut registers, &RINGS, &VECTORS)),
        BlkError::Device(DeviceError::Legacy)
    );
}

#[test]
fn a_read_and_a_write_are_three_buffers_in_the_order_of_the_request() {
    let mut registers = device();
    let blk = live(&mut registers);
    let (mut memory, mut queue) = queue();
    let head = blk
        .submit(&mut queue, &mut memory, &Request::read(1), &CHAIN)
        .expect("a read");
    let header = memory.descriptor(head).expect("the header");
    assert_eq!(header.address, CHAIN.header);
    assert_eq!(header.length, HEADER_LEN);
    assert_eq!(header.flags & virtio_queue::DESC_F_WRITE, 0);
    let data = memory.descriptor(header.next).expect("the data");
    assert_eq!(data.address, 0x2_1000);
    assert_ne!(
        data.flags & virtio_queue::DESC_F_WRITE,
        0,
        "the device writes what a read reads"
    );
    let status = memory.descriptor(data.next).expect("the status");
    assert_eq!(status.address, CHAIN.status);
    assert_eq!(status.length, STATUS_LEN);
    assert_ne!(status.flags & virtio_queue::DESC_F_WRITE, 0);
}

#[test]
fn a_write_hands_the_data_to_the_device_rather_than_taking_it() {
    let mut registers = device();
    let blk = live(&mut registers);
    let (mut memory, mut queue) = queue();
    let head = blk
        .submit(&mut queue, &mut memory, &Request::write(1), &CHAIN)
        .expect("a write");
    let header = memory.descriptor(head).expect("the header");
    let data = memory.descriptor(header.next).expect("the data");
    assert_eq!(data.flags & virtio_queue::DESC_F_WRITE, 0);
}

#[test]
fn a_flush_is_two_buffers_and_carries_no_data() {
    let mut registers = device();
    let blk = live(&mut registers);
    let (mut memory, mut queue) = queue();
    let head = blk
        .submit(&mut queue, &mut memory, &Request::flush(), &FLUSH_CHAIN)
        .expect("a flush");
    let header = memory.descriptor(head).expect("the header");
    let status = memory.descriptor(header.next).expect("the status");
    assert_eq!(status.address, CHAIN.status);
    assert_eq!(status.flags & virtio_queue::DESC_F_NEXT, 0);
}

#[test]
fn a_request_carrying_the_wrong_parts_is_refused() {
    let mut registers = device();
    let blk = live(&mut registers);
    let (mut memory, mut queue) = queue();
    assert_eq!(
        refusal(blk.submit(&mut queue, &mut memory, &Request::read(1), &FLUSH_CHAIN)),
        BlkError::Framing,
        "a read with no data"
    );
    assert_eq!(
        refusal(blk.submit(&mut queue, &mut memory, &Request::flush(), &CHAIN)),
        BlkError::Framing,
        "a flush with data"
    );
}

#[test]
fn data_that_is_not_whole_sectors_is_refused() {
    let mut registers = device();
    let blk = live(&mut registers);
    let (mut memory, mut queue) = queue();
    for length in [0u32, 1, 511, 513, 1023] {
        let chain = Chain {
            data: Some(Segment {
                address: 0x2_1000,
                length,
            }),
            ..CHAIN
        };
        assert_eq!(
            refusal(blk.submit(&mut queue, &mut memory, &Request::read(0), &chain)),
            BlkError::DataLength(length)
        );
    }
}

#[test]
fn a_request_past_the_last_sector_is_refused() {
    let mut registers = device();
    let blk = live(&mut registers);
    let (mut memory, mut queue) = queue();
    let chain = Chain {
        data: Some(Segment {
            address: 0x2_1000,
            length: SECTOR_LEN * 2,
        }),
        ..CHAIN
    };
    blk.submit(
        &mut queue,
        &mut memory,
        &Request::read(CAPACITY - 2),
        &chain,
    )
    .expect("the last two sectors");
    assert_eq!(
        refusal(blk.submit(
            &mut queue,
            &mut memory,
            &Request::read(CAPACITY - 1),
            &chain
        )),
        BlkError::Capacity {
            sector: CAPACITY - 1,
            sectors: 2,
            capacity: CAPACITY,
        }
    );
    assert_eq!(
        refusal(blk.submit(&mut queue, &mut memory, &Request::read(u64::MAX), &chain)),
        BlkError::Capacity {
            sector: u64::MAX,
            sectors: 2,
            capacity: CAPACITY,
        },
        "a sector that would overflow the sum"
    );
}

#[test]
fn a_write_to_a_read_only_device_is_refused_before_it_is_sent() {
    let mut registers = RamDevice::new(OFFERED | F_RO, QUEUE_SIZE, CAPACITY);
    let blk = live(&mut registers);
    assert!(blk.is_read_only());
    let (mut memory, mut queue) = queue();
    assert_eq!(
        refusal(blk.submit(&mut queue, &mut memory, &Request::write(1), &CHAIN)),
        BlkError::ReadOnly
    );
    assert_eq!(
        refusal(blk.submit(&mut queue, &mut memory, &Request::flush(), &FLUSH_CHAIN)),
        BlkError::ReadOnly
    );
    blk.submit(&mut queue, &mut memory, &Request::read(1), &CHAIN)
        .expect("a read is still allowed");
}

#[test]
fn a_flush_of_another_sector_is_refused_by_the_driver_too() {
    let mut registers = device();
    let blk = live(&mut registers);
    let (mut memory, mut queue) = queue();
    let wrong = Request {
        kind: Kind::Flush,
        sector: 3,
    };
    assert_eq!(
        refusal(blk.submit(&mut queue, &mut memory, &wrong, &FLUSH_CHAIN)),
        BlkError::FlushSector(3)
    );
}

#[test]
fn a_queue_that_is_full_refuses_and_says_so() {
    let mut registers = device();
    let blk = live(&mut registers);
    let (mut memory, mut queue) = queue();
    // Eight descriptors and three to a read, so two fit and the third
    // finds two free where it needs three.
    for _ in 0..2 {
        blk.submit(&mut queue, &mut memory, &Request::read(0), &CHAIN)
            .expect("the queue has room");
    }
    let refused = refusal(blk.submit(&mut queue, &mut memory, &Request::read(0), &CHAIN));
    assert_eq!(
        refused,
        BlkError::Queue(QueueError::QueueFull { free: 2, needed: 3 })
    );
    // A flush needs two, which is what is left.
    blk.submit(&mut queue, &mut memory, &Request::flush(), &FLUSH_CHAIN)
        .expect("two descriptors are enough for a flush");
}

#[test]
fn the_notification_goes_to_the_offset_the_multiplier_makes() {
    let mut registers = device();
    registers.write(Structure::Common, common::QUEUE_NOTIFY_OFF, Width::B16, 2);
    let mut blk = Blk::new(MULTIPLIER);
    blk.reset(&mut registers);
    blk.initialize(&mut registers, &RINGS, &VECTORS)
        .expect("the device comes up");
    blk.notify(&mut registers);
    let written: Vec<(u16, u64)> = registers
        .writes()
        .iter()
        .filter(|(structure, _, _)| *structure == Structure::Notify)
        .map(|(_, offset, value)| (*offset, *value))
        .collect();
    assert_eq!(written, vec![(2 * MULTIPLIER as u16, 0)]);
}

#[test]
fn a_notification_offset_that_does_not_fit_the_register_is_refused() {
    let mut registers = device();
    registers.write(
        Structure::Common,
        common::QUEUE_NOTIFY_OFF,
        Width::B16,
        0x4000,
    );
    let mut blk = Blk::new(4);
    blk.reset(&mut registers);
    assert_eq!(
        refusal(blk.initialize(&mut registers, &RINGS, &VECTORS)),
        BlkError::NotifyOffset(0x4000),
        "0x4000 times four is past a sixteen-bit offset"
    );
    let mut blk = Blk::new(u32::MAX);
    blk.reset(&mut registers);
    assert_eq!(
        refusal(blk.initialize(&mut registers, &RINGS, &VECTORS)),
        BlkError::NotifyOffset(0x4000),
        "and so is a multiplier that overflows the product"
    );
}

#[test]
fn a_refusal_part_way_leaves_the_driver_knowing_nothing() {
    let mut registers = RamDevice::new(OFFERED, QUEUE_SIZE, CAPACITY).refusing(Refusal::Vectors);
    let mut blk = Blk::new(MULTIPLIER);
    blk.reset(&mut registers);
    assert!(blk.initialize(&mut registers, &RINGS, &VECTORS).is_err());
    assert_eq!(
        blk.features(),
        0,
        "the features were negotiated and dropped"
    );
    assert_eq!(blk.queue_size(), 0);
    assert_eq!(blk.capacity(), 0);
}

#[test]
fn a_device_reset_and_brought_up_again_is_offered_the_whole_queue() {
    let mut registers = device();
    let mut blk = Blk::new(MULTIPLIER);
    blk.reset(&mut registers);
    let small = Rings { size: 2, ..RINGS };
    blk.initialize(&mut registers, &small, &VECTORS)
        .expect("a queue of two");
    assert_eq!(blk.queue_size(), 2);
    blk.reset(&mut registers);
    blk.initialize(&mut registers, &RINGS, &VECTORS)
        .expect("the device comes up again");
    assert_eq!(blk.queue_size(), QUEUE_SIZE, "the reset gave the size back");
}

#[test]
fn the_interrupt_status_says_what_the_device_raised() {
    let mut registers = device();
    let blk = live(&mut registers);
    registers.set_interrupt_status(ISR_QUEUE | ISR_CONFIG);
    assert_eq!(blk.interrupt_status(&registers), 0b11);
    assert_eq!(blk.interrupt_status(&registers), 0, "reading clears it");
    assert_eq!((ISR_QUEUE, ISR_CONFIG), (1, 2));
}

#[test]
fn a_device_that_asks_for_a_reset_is_noticed_by_a_poll() {
    let mut registers = device();
    let mut blk = live(&mut registers);
    blk.poll(&mut registers).expect("a live device");
    registers.write(
        Structure::Common,
        common::DEVICE_STATUS,
        Width::B8,
        u64::from(virtio_queue::STATUS_DEVICE_NEEDS_RESET),
    );
    assert_eq!(
        refusal(blk.poll(&mut registers)),
        BlkError::Device(DeviceError::NeedsReset)
    );
}

#[test]
fn the_driver_asks_for_what_it_wants_and_no_more() {
    let mut registers = device();
    let blk = live(&mut registers);
    assert_eq!(blk.features() & !WANTED, 0);
}
