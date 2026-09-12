// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::virtio`.

#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
#![allow(clippy::as_conversions, clippy::cast_possible_truncation)]

use crate::capability::{ID_VENDOR, walk};
use crate::doubles::RecordedConfigSpace;
use crate::error::PciError;
use crate::header::read as read_header;
use crate::tests::build::{Builder, VIRTIO_NET, one};
use crate::virtio::{
    BLOCK_DEVICE, DEVICE_BASE, Kind, MAX_STRUCTURES, NETWORK_DEVICE, Structure, first, is_modern,
    structures,
};

/// The structures the one function of `space` publishes.
fn found(space: &RecordedConfigSpace) -> Result<[Option<Structure>; MAX_STRUCTURES], PciError> {
    let address = space.address(0, 0).expect("the function exists");
    let header = read_header(space, address).unwrap().unwrap();
    let capabilities = walk(space, address, &header)?;
    structures(space, address, &capabilities)
}

/// A device with the four structures a driver needs.
fn four_structures() -> RecordedConfigSpace {
    let mut builder = Builder::new(VIRTIO_NET.0, VIRTIO_NET.1);
    builder.capabilities(0x40);
    builder.virtio((0x40, 0x50), 1, 4, 0x0000, 0x1000, None);
    builder.virtio((0x50, 0x60), 3, 4, 0x1000, 0x1000, None);
    builder.virtio((0x60, 0x70), 4, 4, 0x2000, 0x1000, None);
    builder.virtio((0x70, 0x00), 2, 4, 0x3000, 0x1000, Some(4));
    one(&builder)
}

#[test]
fn the_four_structures_a_driver_needs_are_found_by_their_type() {
    let space = four_structures();
    let structures = found(&space).expect("every capability is well formed");
    assert_eq!(structures.iter().flatten().count(), 4);
    assert_eq!(
        first(&structures, Kind::Common),
        Some(Structure {
            kind: Kind::Common,
            bar: 4,
            id: 0,
            offset: 0x0000,
            len: 0x1000,
            multiplier: None,
        })
    );
    assert_eq!(first(&structures, Kind::Isr).unwrap().offset, 0x1000);
    assert_eq!(first(&structures, Kind::Device).unwrap().offset, 0x2000);
    assert_eq!(first(&structures, Kind::SharedMemory), None);
}

#[test]
fn the_notify_capability_carries_a_multiplier_and_the_others_do_not() {
    let space = four_structures();
    let structures = found(&space).unwrap();
    let notify = first(&structures, Kind::Notify).unwrap();
    assert_eq!(notify.multiplier, Some(4));
    assert_eq!(first(&structures, Kind::Common).unwrap().multiplier, None);
    assert_eq!(first(&structures, Kind::Isr).unwrap().multiplier, None);
    assert_eq!(first(&structures, Kind::Device).unwrap().multiplier, None);
}

#[test]
fn the_three_types_this_driver_does_not_use_are_recognized_and_passed_over() {
    let mut builder = Builder::new(VIRTIO_NET.0, VIRTIO_NET.1);
    builder.capabilities(0x40);
    builder.virtio((0x40, 0x50), 5, 0, 0x0000, 0x0000, None);
    builder.virtio((0x50, 0x60), 8, 2, 0x4000, 0x1000, None);
    builder.virtio((0x60, 0x00), 9, 2, 0x5000, 0x0010, None);
    let space = one(&builder);
    let structures = found(&space).unwrap();
    let kinds: Vec<Kind> = structures
        .iter()
        .flatten()
        .map(|found| found.kind)
        .collect();
    assert_eq!(
        kinds,
        vec![Kind::PciConfig, Kind::SharedMemory, Kind::Vendor]
    );
}

#[test]
fn a_type_the_specification_reserves_is_skipped_and_does_not_end_the_walk() {
    let mut builder = Builder::new(VIRTIO_NET.0, VIRTIO_NET.1);
    builder.capabilities(0x40);
    builder.virtio((0x40, 0x50), 6, 4, 0x0000, 0x1000, None);
    builder.virtio((0x50, 0x00), 1, 4, 0x1000, 0x1000, None);
    let space = one(&builder);
    let structures = found(&space).unwrap();
    assert_eq!(structures.iter().flatten().count(), 1);
    assert_eq!(first(&structures, Kind::Common).unwrap().offset, 0x1000);
}

#[test]
fn two_structures_of_a_type_both_come_back_in_the_order_the_list_had_them() {
    let mut builder = Builder::new(VIRTIO_NET.0, VIRTIO_NET.1);
    builder.capabilities(0x40);
    builder.virtio((0x40, 0x60), 2, 0, 0x0000, 0x0004, Some(2));
    builder.virtio((0x60, 0x00), 2, 4, 0x3000, 0x1000, Some(4));
    let space = one(&builder);
    let structures = found(&space).unwrap();
    let notifications: Vec<Structure> = structures
        .iter()
        .flatten()
        .filter(|found| found.kind == Kind::Notify)
        .copied()
        .collect();
    assert_eq!(notifications.len(), 2);
    assert_eq!(
        notifications[0].bar, 0,
        "the device's own order of preference"
    );
    assert_eq!(notifications[1].bar, 4);
    assert_eq!(
        first(&structures, Kind::Notify),
        Some(notifications[0]),
        "the crate chooses neither and answers the first the device named"
    );
}

#[test]
fn a_capability_shorter_than_its_type_needs_is_refused() {
    let mut builder = Builder::new(VIRTIO_NET.0, VIRTIO_NET.1);
    builder.capabilities(0x40);
    builder.virtio((0x40, 0x00), 2, 4, 0x3000, 0x1000, Some(4));
    builder.byte(0x42, 16);
    let space = one(&builder);
    assert_eq!(
        found(&space),
        Err(PciError::CapabilityLength {
            id: ID_VENDOR,
            length: 16,
        })
    );
}

#[test]
fn a_capability_naming_a_register_outside_zero_to_five_is_refused() {
    let mut builder = Builder::new(VIRTIO_NET.0, VIRTIO_NET.1);
    builder.capabilities(0x40);
    builder.virtio((0x40, 0x00), 1, 6, 0x0000, 0x1000, None);
    let space = one(&builder);
    assert_eq!(found(&space), Err(PciError::CapabilityBar(6)));
}

#[test]
fn a_capability_that_would_leave_the_list_is_refused() {
    let mut builder = Builder::new(VIRTIO_NET.0, VIRTIO_NET.1);
    builder.capabilities(0xF4);
    builder.capability(0xF4, ID_VENDOR, 0x00, &[16, 1, 4, 0]);
    let space = one(&builder);
    assert_eq!(found(&space), Err(PciError::Offset(0xF4)));
}

#[test]
fn a_capability_that_is_not_vendor_specific_is_no_structure() {
    let mut builder = Builder::new(VIRTIO_NET.0, VIRTIO_NET.1);
    builder.capabilities(0x40);
    builder.msix(0x40, 0x50, 4, (1, 0), (1, 0x800));
    builder.virtio((0x50, 0x00), 1, 4, 0x0000, 0x1000, None);
    let space = one(&builder);
    let structures = found(&space).unwrap();
    assert_eq!(structures.iter().flatten().count(), 1);
}

#[test]
fn the_network_device_of_virtio_one_is_not_the_transitional_one() {
    assert_eq!(NETWORK_DEVICE, DEVICE_BASE + 1);
    assert_eq!(BLOCK_DEVICE, DEVICE_BASE + 2);
    assert!(is_modern(0x1AF4, BLOCK_DEVICE, 2));
    assert!(is_modern(0x1AF4, NETWORK_DEVICE, 1));
    assert!(!is_modern(0x1AF4, 0x1000, 1), "the transitional device");
    assert!(!is_modern(0x8086, NETWORK_DEVICE, 1));
}

#[test]
fn the_length_a_capability_has_to_announce_follows_from_its_type() {
    assert_eq!(Kind::Notify.capability_len(), 20);
    assert_eq!(Kind::Common.capability_len(), 16);
    assert_eq!(Kind::of(0), None);
    assert_eq!(Kind::of(7), None);
    assert_eq!(Kind::of(8), Some(Kind::SharedMemory));
}
