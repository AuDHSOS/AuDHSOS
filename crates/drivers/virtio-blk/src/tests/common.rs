// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::common`.

use virtio_queue::DeviceRegisters;

use crate::common::{
    self, CONFIG_GENERATION, CONFIG_MSIX_VECTOR, Common, DEVICE_FEATURE, DEVICE_FEATURE_SELECT,
    DEVICE_STATUS, DRIVER_FEATURE, DRIVER_FEATURE_SELECT, LEN, NUM_QUEUES, QUEUE_DESC,
    QUEUE_DEVICE, QUEUE_DRIVER, QUEUE_ENABLE, QUEUE_MSIX_VECTOR, QUEUE_NOTIFY_OFF, QUEUE_SELECT,
    QUEUE_SIZE,
};
use crate::registers::Width;

use super::support::{OFFERED, device};

/// The structure of virtio 4.1.4.3, field by field: the offset and how
/// many bytes it takes.
const LAYOUT: [(u16, u16); 15] = [
    (DEVICE_FEATURE_SELECT, 4),
    (DEVICE_FEATURE, 4),
    (DRIVER_FEATURE_SELECT, 4),
    (DRIVER_FEATURE, 4),
    (CONFIG_MSIX_VECTOR, 2),
    (NUM_QUEUES, 2),
    (DEVICE_STATUS, 1),
    (CONFIG_GENERATION, 1),
    (QUEUE_SELECT, 2),
    (QUEUE_SIZE, 2),
    (QUEUE_MSIX_VECTOR, 2),
    (QUEUE_ENABLE, 2),
    (QUEUE_NOTIFY_OFF, 2),
    (QUEUE_DESC, 8),
    (QUEUE_DRIVER, 8),
];

#[test]
fn every_field_follows_the_one_before_it_with_no_gap() {
    let mut at = 0u16;
    for (offset, len) in LAYOUT {
        assert_eq!(offset, at, "the field at {at:#x}");
        at = at.saturating_add(len);
    }
    assert_eq!(QUEUE_DEVICE, at);
    assert_eq!(LEN, at.saturating_add(8), "the structure this driver reads");
}

#[test]
fn the_offered_features_come_out_of_both_windows() {
    let mut device = device();
    let mut common = Common::new(&mut device);
    assert_eq!(common.device_features(), 0, "nothing is read until it is");
    assert_eq!(common.read_offered(), OFFERED);
    assert_eq!(common.device_features(), OFFERED);
    let high = 1u64 << 40;
    let mut wide = crate::doubles::RamDevice::new(OFFERED | high, 8, 16);
    let mut common = Common::new(&mut wide);
    assert_eq!(common.read_offered(), OFFERED | high);
}

#[test]
fn the_accepted_features_go_into_both_windows() {
    let mut device = device();
    let accepted = OFFERED | (1u64 << 34);
    let mut common = Common::new(&mut device);
    common.set_driver_features(accepted);
    assert_eq!(device.accepted(), accepted);
}

#[test]
fn the_status_is_read_and_written_as_one_byte() {
    let mut device = device();
    let mut common = Common::new(&mut device);
    assert_eq!(common.status(), 0);
    common.set_status(0x0F);
    assert_eq!(common.status(), 0x0F);
    assert_eq!(device.common_field(DEVICE_STATUS, Width::B8), 0x0F);
}

#[test]
fn the_fields_of_a_queue_are_reached_by_width() {
    let mut device = device();
    let mut common = Common::new(&mut device);
    common.set_u16(QUEUE_SELECT, 0);
    assert_eq!(common.u16_at(QUEUE_SIZE), super::support::QUEUE_SIZE);
    common.set_field(QUEUE_DESC, Width::B64, 0xDEAD_BEEF_0000);
    assert_eq!(common.field(QUEUE_DESC, Width::B64), 0xDEAD_BEEF_0000);
    assert_eq!(common.u8_at(DEVICE_STATUS), 0);
    assert_eq!(common.offered(), 0, "not read yet");
    let _ = common.registers();
}

#[test]
fn the_constants_of_the_transport_are_the_ones_the_specification_fixes() {
    assert_eq!(common::NO_VECTOR, 0xFFFF);
    assert_eq!(common::ENABLED, 1);
    assert_eq!(common::REQUEST_QUEUE, 0);
}
