// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::enumerate`.

#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
#![allow(clippy::as_conversions, clippy::cast_possible_truncation)]

use crate::address::{Address, Window};
use crate::doubles::RecordedConfigSpace;
use crate::enumerate::{find, walk};
use crate::header::Kind;
use crate::tests::build::{Builder, VIRTIO_NET, one};

/// The window of one bus, which is what every test of this file walks.
fn one_bus() -> Window {
    Window::new(0, 0, 0).expect("the range runs forwards")
}

/// Every address the walk reported.
fn addresses(space: &RecordedConfigSpace) -> Vec<Address> {
    let mut seen = Vec::new();
    let count =
        walk(space, one_bus(), |function| seen.push(function.address)).expect("the space answers");
    assert_eq!(count, seen.len());
    seen
}

#[test]
fn a_bus_with_one_function_reports_that_one() {
    let builder = Builder::new(0x8086, 0x29C0);
    let space = one(&builder);
    assert_eq!(addresses(&space), vec![Address::new(0, 0, 0, 0).unwrap()]);
}

#[test]
fn an_empty_bus_reports_nothing() {
    let space = RecordedConfigSpace::blank();
    assert_eq!(addresses(&space), Vec::new());
}

#[test]
fn only_function_zero_is_read_of_a_device_that_is_not_multi_function() {
    let mut space = RecordedConfigSpace::blank();
    Builder::new(0x8086, 0x2918).install(&mut space, 31, 0);
    Builder::new(0x8086, 0x2922).install(&mut space, 31, 2);
    assert_eq!(
        addresses(&space),
        vec![Address::new(0, 0, 31, 0).unwrap()],
        "function two is there and is not looked at"
    );
}

#[test]
fn bit_seven_of_the_header_type_makes_the_other_functions_be_read() {
    let mut space = RecordedConfigSpace::blank();
    let mut first = Builder::new(0x8086, 0x2918);
    first.header_type(0x80);
    first.install(&mut space, 31, 0);
    Builder::new(0x8086, 0x2922).install(&mut space, 31, 2);
    Builder::new(0x8086, 0x2930).install(&mut space, 31, 3);
    assert_eq!(
        addresses(&space),
        vec![
            Address::new(0, 0, 31, 0).unwrap(),
            Address::new(0, 0, 31, 2).unwrap(),
            Address::new(0, 0, 31, 3).unwrap(),
        ]
    );
}

#[test]
fn a_device_that_is_not_there_ends_the_walk_of_that_device_and_not_of_the_bus() {
    let mut space = RecordedConfigSpace::blank();
    Builder::new(0x8086, 0x29C0).install(&mut space, 0, 0);
    Builder::new(0x1AF4, 0x1041).install(&mut space, 5, 0);
    assert_eq!(
        addresses(&space),
        vec![
            Address::new(0, 0, 0, 0).unwrap(),
            Address::new(0, 0, 5, 0).unwrap(),
        ],
        "the devices between the two are absent and stop nothing"
    );
}

#[test]
fn a_bridge_is_reported_and_not_descended_into() {
    let mut builder = Builder::new(0x8086, 0x2918);
    builder.header_type(0x01);
    builder.byte(0x19, 1);
    let space = one(&builder);
    let mut kinds = Vec::new();
    walk(&space, one_bus(), |function| {
        kinds.push(function.header.kind);
    })
    .unwrap();
    assert_eq!(kinds, vec![Kind::Bridge]);
    assert_eq!(
        addresses(&space).len(),
        1,
        "the bus behind the bridge is not walked"
    );
}

#[test]
fn a_walk_over_several_buses_reports_every_bus_it_covers() {
    let space = RecordedConfigSpace::blank();
    let window = Window::new(0, 0, 3).unwrap();
    let count = walk(&space, window, |_| ()).unwrap();
    assert_eq!(count, 0, "no bus of this space holds a function");
}

#[test]
fn the_device_a_walk_is_looking_for_is_found_by_its_identifiers() {
    let mut space = RecordedConfigSpace::blank();
    Builder::new(0x8086, 0x29C0).install(&mut space, 0, 0);
    Builder::new(VIRTIO_NET.0, VIRTIO_NET.1).install(&mut space, 1, 0);
    let found = find(&space, one_bus(), VIRTIO_NET.0, VIRTIO_NET.1)
        .unwrap()
        .expect("the device is on the bus");
    assert_eq!(found.address, Address::new(0, 0, 1, 0).unwrap());
    assert_eq!(found.header.device, VIRTIO_NET.1);
    assert_eq!(find(&space, one_bus(), 0x1AF4, 0x1000).unwrap(), None);
}
