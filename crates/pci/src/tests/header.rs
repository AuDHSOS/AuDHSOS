// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::header`.

#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
#![allow(clippy::as_conversions, clippy::cast_possible_truncation)]

use crate::address::Address;
use crate::doubles::RecordedConfigSpace;
use crate::header::{
    COMMAND_MEMORY, Kind, STATUS_CAPABILITIES, Subsystem, read, read_command, write_command,
};
use crate::tests::build::{Builder, VIRTIO_NET, one};

/// The function every test of this file reads.
fn address(space: &RecordedConfigSpace) -> Address {
    space.address(0, 0).expect("the device and function exist")
}

#[test]
fn a_function_reports_what_its_header_holds() {
    let mut builder = Builder::new(VIRTIO_NET.0, VIRTIO_NET.1);
    builder.class(0x02, 0x00, 0x00);
    builder.byte(0x08, 0x01);
    builder.word(0x2C, 0x1100_1AF4);
    let space = one(&builder);
    let header = read(&space, address(&space))
        .expect("the space answers")
        .expect("the function is there");
    assert_eq!(header.vendor, VIRTIO_NET.0);
    assert_eq!(header.device, VIRTIO_NET.1);
    assert_eq!(header.class, 0x02);
    assert_eq!(header.subclass, 0x00);
    assert_eq!(header.revision, 0x01);
    assert_eq!(header.kind, Kind::Endpoint);
    assert_eq!(
        header.subsystem,
        Some(Subsystem {
            vendor: 0x1AF4,
            device: 0x1100,
        })
    );
    assert!(header.decodes_memory());
    assert!(!header.is_multi_function());
    assert!(!header.has_capabilities());
}

#[test]
fn an_absent_function_reads_as_all_ones_and_is_not_an_error() {
    let space = RecordedConfigSpace::blank();
    let address = Address::new(0, 0, 0, 0).unwrap();
    assert_eq!(read(&space, address), Ok(None));
}

#[test]
fn a_header_type_this_crate_does_not_read_is_reported_and_skipped() {
    let mut builder = Builder::new(0x8086, 0x1234);
    builder.header_type(0x02);
    builder.word(0x2C, 0xDEAD_BEEF);
    let space = one(&builder);
    let header = read(&space, address(&space)).unwrap().unwrap();
    assert_eq!(header.kind, Kind::Other(2));
    assert_eq!(header.subsystem, None, "no type-0 field is read out of it");
}

#[test]
fn a_bridge_is_read_and_carries_no_subsystem_of_a_type_zero_header() {
    let mut builder = Builder::new(0x8086, 0x2918);
    builder.header_type(0x81);
    let space = one(&builder);
    let header = read(&space, address(&space)).unwrap().unwrap();
    assert_eq!(header.kind, Kind::Bridge);
    assert_eq!(header.subsystem, None);
    assert!(header.is_multi_function());
}

#[test]
fn the_capability_pointer_is_read_only_when_the_status_bit_says_there_is_one() {
    let mut builder = Builder::new(0x8086, 0x1234);
    builder.byte(0x34, 0x40);
    let space = one(&builder);
    let header = read(&space, address(&space)).unwrap().unwrap();
    assert_eq!(header.capabilities, 0, "the status bit is clear");
    assert!(!header.has_capabilities());

    let mut with = Builder::new(0x8086, 0x1234);
    with.capabilities(0x40);
    let space = one(&with);
    let header = read(&space, address(&space)).unwrap().unwrap();
    assert_eq!(header.capabilities, 0x40);
    assert!(header.status & STATUS_CAPABILITIES != 0);
    assert!(header.has_capabilities());
}

#[test]
fn the_command_register_is_read_back_as_it_was_written() {
    let mut builder = Builder::new(0x8086, 0x1234);
    builder.word(0x04, 0x0000_0003);
    let mut space = one(&builder);
    let address = address(&space);
    assert_eq!(read_command(&space, address), Ok(0x0003));
    write_command(&mut space, address, 0x0003 & !COMMAND_MEMORY).unwrap();
    assert_eq!(read_command(&space, address), Ok(0x0001));
    assert_eq!(
        space.peek(0, 0, 0x04).unwrap() >> 16,
        0,
        "the status half of the word is untouched"
    );
}

#[test]
fn the_kind_of_a_header_type_ignores_the_multi_function_bit() {
    assert_eq!(Kind::of(0x80), Kind::Endpoint);
    assert_eq!(Kind::of(0x81), Kind::Bridge);
    assert_eq!(Kind::of(0x7F), Kind::Other(0x7F));
}
