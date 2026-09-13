// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Bringing the device up: the sequence, and each step that can refuse.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::indexing_slicing
)]

use virtio_queue::{
    DeviceError, F_VERSION_1, Phase, STATUS_ACKNOWLEDGE, STATUS_DRIVER, STATUS_DRIVER_OK,
    STATUS_FEATURES_OK,
};

use crate::common::{self, NO_VECTOR};
use crate::doubles::{RamDevice, Refusal};
use crate::error::NetError;
use crate::features::{F_MAC, WANTED};
use crate::init::Net;
use crate::queues::{Queues, Rings, Side, Vectors};
use crate::registers::{Registers, Structure, Width};

use super::support::{
    ADDRESS, MULTIPLIER, OFFERED, QUEUE_SIZE, QUEUES, SLOTS, VECTORS, device, live, refusal,
};

#[test]
fn the_sequence_reaches_driver_ok_and_takes_the_two_wanted_bits() {
    let mut registers = device();
    let net = live(&mut registers);
    assert_eq!(net.device().phase(), Phase::Live);
    assert_eq!(net.features(), WANTED);
    assert_eq!(registers.accepted(), WANTED);
    assert_eq!(
        registers.status(),
        STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK
    );
}

#[test]
fn the_status_bits_are_written_in_the_order_of_section_3_1_1() {
    let mut registers = device();
    let _net = live(&mut registers);
    let written: std::vec::Vec<u64> = registers
        .writes()
        .into_iter()
        .filter(|(structure, offset, _)| {
            *structure == Structure::Common && *offset == common::DEVICE_STATUS
        })
        .map(|(_, _, value)| value)
        .collect();
    assert_eq!(
        written,
        std::vec![
            0,
            u64::from(STATUS_ACKNOWLEDGE),
            u64::from(STATUS_ACKNOWLEDGE | STATUS_DRIVER),
            u64::from(STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK),
            u64::from(STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK),
        ]
    );
}

#[test]
fn both_queues_are_told_where_their_rings_are_and_are_enabled() {
    let mut registers = device();
    let _net = live(&mut registers);
    let writes = registers.writes();
    for (side, rings) in [
        (Side::Receive, QUEUES.receive),
        (Side::Transmit, QUEUES.transmit),
    ] {
        let selected = writes
            .iter()
            .position(|(structure, offset, value)| {
                *structure == Structure::Common
                    && *offset == common::QUEUE_SELECT
                    && *value == u64::from(side.index())
            })
            .unwrap_or_else(|| panic!("{side} was never selected"));
        let after = &writes[selected..];
        let next = after
            .iter()
            .skip(1)
            .position(|(structure, offset, _)| {
                *structure == Structure::Common && *offset == common::QUEUE_SELECT
            })
            .map_or(after.len(), |at| at + 1);
        let block = &after[..next];
        for (offset, wanted) in [
            (common::QUEUE_DESC, rings.descriptor_table),
            (common::QUEUE_DRIVER, rings.available_ring),
            (common::QUEUE_DEVICE, rings.used_ring),
            (common::QUEUE_ENABLE, u64::from(common::ENABLED)),
        ] {
            assert!(
                block
                    .iter()
                    .any(|(_, at, value)| *at == offset && *value == wanted),
                "{side} was not told {offset:#x}"
            );
        }
    }
}

#[test]
fn each_queue_is_given_its_own_vector_and_the_configuration_its_own() {
    let mut registers = device();
    let _net = live(&mut registers);
    assert_eq!(
        registers.common_field(common::CONFIG_MSIX_VECTOR, Width::B16),
        u64::from(VECTORS.config)
    );
    // The last vector written is the transmit queue's, the queues being
    // configured in the order the device numbers them.
    assert_eq!(
        registers.common_field(common::QUEUE_MSIX_VECTOR, Width::B16),
        u64::from(VECTORS.transmit)
    );
}

#[test]
fn the_notification_offset_of_each_queue_carries_the_multiplier() {
    let mut registers = device();
    let net = live(&mut registers);
    assert_eq!(net.queue(Side::Receive).notify, 0);
    assert_eq!(
        net.queue(Side::Transmit).notify,
        u16::try_from(MULTIPLIER).unwrap_or(0)
    );
}

#[test]
fn a_notify_writes_the_queue_index_at_the_queues_own_offset() {
    let mut registers = device();
    let net = live(&mut registers);
    net.notify(&mut registers, Side::Transmit);
    let last = registers.writes().pop().expect("a write");
    assert_eq!(
        last,
        (
            Structure::Notify,
            u16::try_from(MULTIPLIER).unwrap_or(0),
            u64::from(Side::Transmit.index())
        )
    );
}

#[test]
fn the_address_is_the_six_bytes_the_device_carries() {
    let mut registers = device();
    let net = live(&mut registers);
    assert_eq!(net.mac(), Some(ADDRESS));
}

#[test]
fn a_device_that_offered_no_address_yields_none_rather_than_six_zeroes() {
    let mut registers = RamDevice::new(F_VERSION_1, QUEUE_SIZE, [0; 6]);
    let net = live(&mut registers);
    assert_eq!(net.features() & F_MAC, 0);
    assert_eq!(net.mac(), None);
}

#[test]
fn a_legacy_device_is_refused_before_a_queue_is_touched() {
    let mut registers = RamDevice::new(F_MAC, QUEUE_SIZE, ADDRESS);
    let mut net = Net::<SLOTS>::new(MULTIPLIER);
    net.reset(&mut registers);
    let error = refusal(net.initialize(&mut registers, &QUEUES, &VECTORS));
    assert_eq!(error, NetError::Device(DeviceError::Legacy));
    assert_eq!(net.device().phase(), Phase::Failed(DeviceError::Legacy));
}

#[test]
fn a_device_that_clears_features_ok_fails_at_that_step() {
    let mut registers = device().refusing(Refusal::Features);
    let mut net = Net::<SLOTS>::new(MULTIPLIER);
    net.reset(&mut registers);
    let error = refusal(net.initialize(&mut registers, &QUEUES, &VECTORS));
    assert_eq!(error, NetError::Device(DeviceError::FeaturesRejected));
}

#[test]
fn a_device_that_will_take_no_vector_is_refused_at_the_queue() {
    let mut registers = device().refusing(Refusal::Vectors);
    let mut net = Net::<SLOTS>::new(MULTIPLIER);
    net.reset(&mut registers);
    let error = refusal(net.initialize(&mut registers, &QUEUES, &VECTORS));
    assert_eq!(error, NetError::NoVector(VECTORS.receive));
}

#[test]
fn a_driver_that_asks_for_no_vector_at_all_comes_up() {
    let mut registers = device().refusing(Refusal::Vectors);
    let mut net = Net::<SLOTS>::new(MULTIPLIER);
    net.reset(&mut registers);
    net.initialize(&mut registers, &QUEUES, &Vectors::none())
        .expect("a device without message interrupts comes up");
    assert_eq!(net.device().phase(), Phase::Live);
}

#[test]
fn one_vector_for_both_queues_is_written_to_both() {
    assert_eq!(Vectors::shared(3).receive, 3);
    assert_eq!(Vectors::shared(3).transmit, 3);
    assert_eq!(Vectors::shared(3).config, NO_VECTOR);
}

#[test]
fn a_device_without_a_transmit_queue_is_refused_and_says_which() {
    let mut registers = RamDevice::with_queues(OFFERED, [QUEUE_SIZE, 0], ADDRESS);
    let mut net = Net::<SLOTS>::new(MULTIPLIER);
    net.reset(&mut registers);
    let error = refusal(net.initialize(&mut registers, &QUEUES, &VECTORS));
    assert_eq!(error, NetError::NoQueue(Side::Transmit));
}

#[test]
fn a_device_offering_fewer_descriptors_settles_the_size() {
    let mut registers = RamDevice::new(OFFERED, 4, ADDRESS);
    let mut net = Net::<SLOTS>::new(MULTIPLIER);
    net.reset(&mut registers);
    net.initialize(&mut registers, &QUEUES, &VECTORS)
        .expect("a smaller queue is still a queue");
    assert_eq!(net.queue(Side::Receive).size, 4);
    assert_eq!(registers.queue_size(Side::Receive.index()), 4);
}

#[test]
fn a_queue_of_more_descriptors_than_the_driver_holds_is_refused() {
    let mut registers = RamDevice::new(OFFERED, 16, ADDRESS);
    let mut net = Net::<SLOTS>::new(MULTIPLIER);
    net.reset(&mut registers);
    let rings = Rings {
        size: 16,
        ..QUEUES.receive
    };
    let queues = Queues {
        receive: rings,
        transmit: Rings {
            size: 16,
            ..QUEUES.transmit
        },
    };
    let error = refusal(net.initialize(&mut registers, &queues, &VECTORS));
    assert_eq!(error, NetError::Slots(16));
}

#[test]
fn rings_of_a_size_that_is_not_a_power_of_two_are_refused_before_anything_is_written() {
    let mut registers = device();
    let mut net = Net::<SLOTS>::new(MULTIPLIER);
    net.reset(&mut registers);
    let before = registers.writes().len();
    let queues = Queues {
        receive: Rings {
            size: 6,
            ..QUEUES.receive
        },
        transmit: QUEUES.transmit,
    };
    let error = refusal(net.initialize(&mut registers, &queues, &VECTORS));
    assert_eq!(error, NetError::QueueSize(6));
    assert_eq!(registers.writes().len(), before);
}

#[test]
fn a_device_that_was_not_reset_is_refused() {
    let mut registers = device();
    let mut net = Net::<SLOTS>::new(MULTIPLIER);
    net.reset(&mut registers);
    net.initialize(&mut registers, &QUEUES, &VECTORS)
        .expect("the first time");
    let mut again = Net::<SLOTS>::new(MULTIPLIER);
    let error = refusal(again.initialize(&mut registers, &QUEUES, &VECTORS));
    assert_eq!(
        error,
        NetError::NotReset(
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK
        )
    );
}

#[test]
fn a_configuration_that_will_not_stand_still_is_reported_rather_than_spun_on() {
    let mut registers = device().refusing(Refusal::Generation);
    let mut net = Net::<SLOTS>::new(MULTIPLIER);
    net.reset(&mut registers);
    let error = refusal(net.initialize(&mut registers, &QUEUES, &VECTORS));
    assert_eq!(error, NetError::Generation(crate::net::ATTEMPTS));
}

#[test]
fn a_notification_offset_that_does_not_fit_a_structure_offset_is_refused() {
    let mut registers = device();
    let mut net = Net::<SLOTS>::new(u32::MAX);
    net.reset(&mut registers);
    let error = refusal(net.initialize(&mut registers, &QUEUES, &VECTORS));
    assert_eq!(error, NetError::NotifyOffset(1));
}

#[test]
fn a_reset_leaves_the_driver_knowing_nothing() {
    let mut registers = device();
    let mut net = live(&mut registers);
    net.reset(&mut registers);
    assert_eq!(net.features(), 0);
    assert_eq!(net.mac(), None);
    assert_eq!(net.queue(Side::Receive).size, 0);
    assert_eq!(net.device().phase(), Phase::Reset);
    assert!(net.is_reset(&registers));
}

#[test]
fn the_interrupt_status_is_cleared_by_the_read() {
    let mut registers = device();
    let net = live(&mut registers);
    registers.set_interrupt_status(crate::init::ISR_QUEUE);
    assert_eq!(net.interrupt_status(&registers), crate::init::ISR_QUEUE);
    assert_eq!(net.interrupt_status(&registers), 0);
}

#[test]
fn a_device_that_asks_for_a_reset_is_noticed() {
    let mut registers = device();
    let mut net = live(&mut registers);
    net.poll(&mut registers).expect("a live device");
    registers.write(
        Structure::Common,
        common::DEVICE_STATUS,
        Width::B8,
        u64::from(virtio_queue::STATUS_DEVICE_NEEDS_RESET),
    );
    let error = refusal(net.poll(&mut registers));
    assert_eq!(error, NetError::Device(DeviceError::NeedsReset));
}

#[test]
fn every_refusal_says_something_of_its_own() {
    let mut seen = std::vec::Vec::new();
    for error in [
        NetError::NotReset(3),
        NetError::NoQueue(Side::Receive),
        NetError::QueueSize(6),
        NetError::Slots(16),
        NetError::NoVector(1),
        NetError::NotifyOffset(1),
        NetError::Generation(8),
        NetError::FrameTooLong {
            len: 3,
            capacity: 2,
        },
        NetError::ShortFrame(4),
        NetError::UnknownBuffer(2),
        NetError::NoBuffer,
        NetError::Buffer(1),
        NetError::Device(DeviceError::Legacy),
        NetError::Queue(virtio_queue::QueueError::EmptyChain),
    ] {
        let said = std::format!("{error}");
        assert!(!said.is_empty());
        assert!(!seen.contains(&said), "two refusals say {said}");
        seen.push(said);
    }
}

#[test]
fn the_two_queues_are_numbered_as_the_specification_numbers_them() {
    assert_eq!(Side::Receive.index(), 0);
    assert_eq!(Side::Transmit.index(), 1);
    assert_eq!(Side::ALL, [Side::Receive, Side::Transmit]);
    assert_eq!(std::format!("{}", Side::Receive), "receive");
    assert_eq!(std::format!("{}", Side::Transmit), "transmit");
}

#[test]
fn the_rings_of_a_side_are_the_ones_that_side_was_given() {
    assert_eq!(QUEUES.of(Side::Receive), &QUEUES.receive);
    assert_eq!(QUEUES.of(Side::Transmit), &QUEUES.transmit);
}

#[test]
fn a_device_driven_without_message_interrupts_takes_no_vector() {
    let none = Vectors::none();
    assert_eq!(none.of(Side::Receive), NO_VECTOR);
    assert_eq!(none.of(Side::Transmit), NO_VECTOR);
    assert_eq!(none.config, NO_VECTOR);
}

#[test]
fn the_driver_says_which_queue_it_was_given_how_many_descriptors() {
    let mut registers = device();
    let net = live(&mut registers);
    assert_eq!(net.queue(Side::Receive).size, QUEUE_SIZE);
    assert_eq!(net.queue(Side::Transmit).size, QUEUE_SIZE);
}

#[test]
fn a_queue_that_is_not_live_refuses_to_say_whether_it_has_to_be_told() {
    let net = Net::<SLOTS>::new(MULTIPLIER);
    let memory = virtio_queue::doubles::RamQueue::new(QUEUE_SIZE);
    let mut over = virtio_queue::doubles::RamQueue::new(QUEUE_SIZE);
    let queue = virtio_queue::Queue::<1>::new(&mut over, QUEUE_SIZE).expect("a queue");
    let error = refusal(net.should_notify(&queue, &memory));
    assert_eq!(error, NetError::Queue(virtio_queue::QueueError::NotLive));
}
