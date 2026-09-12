// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::config`.

use crate::config::{
    ATTEMPTS, BLK_SIZE, CAPACITY, GEOMETRY, SECTOR_LEN, SEG_MAX, SIZE_MAX, capacity,
};
use crate::doubles::{RamDevice, Refusal};
use crate::error::BlkError;

use super::support::{CAPACITY as SECTORS, device};

#[test]
fn the_capacity_is_read_out_of_the_device_configuration() {
    let device = device();
    assert_eq!(capacity(&device), Ok(SECTORS));
}

#[test]
fn a_configuration_that_will_not_stand_still_is_refused() {
    let device = RamDevice::new(0, 8, SECTORS).refusing(Refusal::Generation);
    assert_eq!(capacity(&device), Err(BlkError::Generation(ATTEMPTS)));
}

#[test]
fn the_fields_lie_where_the_configuration_layout_puts_them() {
    assert_eq!(CAPACITY, 0);
    assert_eq!(SIZE_MAX, 8);
    assert_eq!(SEG_MAX, 12);
    assert_eq!(GEOMETRY, 16);
    assert_eq!(BLK_SIZE, 20);
    assert_eq!(SECTOR_LEN, 512);
}
