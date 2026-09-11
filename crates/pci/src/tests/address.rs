// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::address`.

#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
#![allow(clippy::as_conversions, clippy::cast_possible_truncation)]

use crate::address::{
    Address, BYTES_PER_BUS, CONFIG_SPACE_LEN, MAX_DEVICE, MAX_FUNCTION, Window, check_offset,
};
use crate::error::PciError;

/// The window of a machine whose firmware published one segment group of
/// every bus.
fn whole_bus_range() -> Window {
    Window::new(0, 0, 255).expect("the range runs forwards")
}

#[test]
fn the_lowest_address_lies_at_the_start_of_the_window() {
    let window = whole_bus_range();
    let address = Address::new(0, 0, 0, 0).unwrap();
    assert_eq!(window.offset_of(address, 0), Ok(0));
}

#[test]
fn the_highest_address_lies_at_the_last_word_of_the_window() {
    let window = whole_bus_range();
    let address = Address::new(0, 255, MAX_DEVICE, MAX_FUNCTION).unwrap();
    let expected = (255 << 20) | (31 << 15) | (7 << 12) | u64::from(CONFIG_SPACE_LEN - 4);
    assert_eq!(
        window.offset_of(address, CONFIG_SPACE_LEN - 4),
        Ok(expected)
    );
}

#[test]
fn each_field_lands_in_the_bits_the_mechanism_gives_it() {
    let window = whole_bus_range();
    let address = Address::new(0, 1, 2, 3).unwrap();
    assert_eq!(
        window.offset_of(address, 0x34),
        Ok((1 << 20) | (2 << 15) | (3 << 12) | 0x34)
    );
}

#[test]
fn the_window_of_a_later_first_bus_starts_that_bus_at_zero() {
    let window = Window::new(0, 16, 31).unwrap();
    let address = Address::new(0, 16, 0, 0).unwrap();
    assert_eq!(window.offset_of(address, 0), Ok(0));
    let next = Address::new(0, 17, 0, 0).unwrap();
    assert_eq!(window.offset_of(next, 0), Ok(BYTES_PER_BUS));
}

#[test]
fn a_device_above_the_thirty_one_a_bus_has_is_refused() {
    assert_eq!(Address::new(0, 0, 32, 0), Err(PciError::Device(32)));
}

#[test]
fn a_function_above_the_seven_a_device_has_is_refused() {
    assert_eq!(Address::new(0, 0, 0, 8), Err(PciError::Function(8)));
}

#[test]
fn an_offset_beyond_the_configuration_space_is_refused() {
    let window = whole_bus_range();
    let address = Address::new(0, 0, 0, 0).unwrap();
    assert_eq!(
        window.offset_of(address, CONFIG_SPACE_LEN),
        Err(PciError::Offset(CONFIG_SPACE_LEN))
    );
    assert_eq!(
        check_offset(CONFIG_SPACE_LEN - 3),
        Err(PciError::Unaligned(CONFIG_SPACE_LEN - 3))
    );
}

#[test]
fn a_word_that_would_leave_the_configuration_space_is_refused() {
    assert_eq!(
        check_offset(CONFIG_SPACE_LEN - 2),
        Err(PciError::Unaligned(CONFIG_SPACE_LEN - 2))
    );
    assert_eq!(check_offset(CONFIG_SPACE_LEN - 4), Ok(()));
}

#[test]
fn an_offset_that_is_no_whole_word_is_refused() {
    let window = whole_bus_range();
    let address = Address::new(0, 0, 0, 0).unwrap();
    assert_eq!(
        window.offset_of(address, 0x35),
        Err(PciError::Unaligned(0x35))
    );
}

#[test]
fn the_window_length_follows_from_the_bus_range() {
    assert_eq!(whole_bus_range().len(), 256 * BYTES_PER_BUS);
    assert_eq!(Window::new(0, 0, 0).unwrap().len(), BYTES_PER_BUS);
    assert_eq!(Window::new(0, 4, 7).unwrap().buses(), 4);
    assert!(!whole_bus_range().is_empty());
}

#[test]
fn a_window_whose_last_bus_is_below_its_first_is_refused() {
    assert_eq!(
        Window::new(0, 16, 15),
        Err(PciError::BusRange {
            first_bus: 16,
            last_bus: 15,
        })
    );
}

#[test]
fn a_function_outside_the_bus_range_is_never_addressed() {
    let window = Window::new(0, 4, 7).unwrap();
    let below = Address::new(0, 3, 0, 0).unwrap();
    let above = Address::new(0, 8, 0, 0).unwrap();
    assert!(!window.holds(below));
    assert!(!window.holds(above));
    assert_eq!(
        window.offset_of(above, 0),
        Err(PciError::BusOutside {
            bus: 8,
            first_bus: 4,
            last_bus: 7,
        })
    );
}

#[test]
fn a_function_of_another_segment_group_is_never_addressed() {
    let window = whole_bus_range();
    let elsewhere = Address::new(1, 0, 0, 0).unwrap();
    assert!(!window.holds(elsewhere));
    assert_eq!(
        window.offset_of(elsewhere, 0),
        Err(PciError::Segment {
            wanted: 1,
            window: 0,
        })
    );
}

#[test]
fn an_address_names_its_own_fields_back() {
    let address = Address::new(2, 3, 4, 5).unwrap();
    assert_eq!(address.segment(), 2);
    assert_eq!(address.bus(), 3);
    assert_eq!(address.device(), 4);
    assert_eq!(address.function(), 5);
    assert_eq!(address.first_function().function(), 0);
    assert_eq!(address.with_function(7).unwrap().function(), 7);
    assert_eq!(address.with_function(8), None);
}

#[test]
fn the_addresses_of_a_window_are_every_function_of_every_device() {
    let window = Window::new(0, 0, 1).unwrap();
    let addresses: Vec<Address> = window.addresses().collect();
    assert_eq!(addresses.len(), 2 * 32 * 8);
    assert_eq!(addresses[0], Address::new(0, 0, 0, 0).unwrap());
    assert_eq!(addresses[8], Address::new(0, 0, 1, 0).unwrap());
    assert_eq!(
        addresses[addresses.len() - 1],
        Address::new(0, 1, 31, 7).unwrap()
    );
}
