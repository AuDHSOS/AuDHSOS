// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::features`.

use virtio_queue::F_VERSION_1;

use crate::features::{
    F_BARRIER, F_BLK_SIZE, F_CONFIG_WCE, F_DISCARD, F_FLUSH, F_GEOMETRY, F_LIFETIME, F_MQ, F_RO,
    F_SCSI, F_SECURE_ERASE, F_SEG_MAX, F_SIZE_MAX, F_TOPOLOGY, F_WRITE_ZEROES, F_ZONED, WANTED,
    name, refused,
};

/// Every bit of the block device's own range, as virtio 5.2.3 has them.
const EVERY: [(u64, u32); 16] = [
    (F_BARRIER, 0),
    (F_SIZE_MAX, 1),
    (F_SEG_MAX, 2),
    (F_GEOMETRY, 4),
    (F_RO, 5),
    (F_BLK_SIZE, 6),
    (F_SCSI, 7),
    (F_FLUSH, 9),
    (F_TOPOLOGY, 10),
    (F_CONFIG_WCE, 11),
    (F_MQ, 12),
    (F_DISCARD, 13),
    (F_WRITE_ZEROES, 14),
    (F_LIFETIME, 15),
    (F_SECURE_ERASE, 16),
    (F_ZONED, 17),
];

#[test]
fn every_feature_is_the_bit_the_specification_numbers_it() {
    for (feature, bit) in EVERY {
        assert_eq!(feature, 1u64 << bit, "bit {bit}");
    }
}

#[test]
fn the_driver_asks_for_the_version_the_flush_and_the_read_only_flag() {
    assert_eq!(WANTED, F_VERSION_1 | F_FLUSH | F_RO);
}

#[test]
fn every_bit_of_the_range_has_a_name_and_no_two_share_one() {
    let mut names: Vec<&str> = EVERY.iter().map(|(bit, _)| name(*bit).unwrap()).collect();
    assert!(names.iter().all(|text| text.starts_with("VIRTIO_BLK_F_")));
    names.sort_unstable();
    let count = names.len();
    names.dedup();
    assert_eq!(names.len(), count, "two features read the same");
}

#[test]
fn a_bit_of_the_transport_range_has_no_name_here() {
    assert_eq!(name(F_VERSION_1), None);
    assert_eq!(name(1 << 63), None);
    assert_eq!(name(0), None);
}

#[test]
fn what_the_driver_does_not_take_is_what_is_refused() {
    assert_eq!(refused(WANTED), 0);
    assert_eq!(refused(F_VERSION_1 | F_MQ), F_MQ);
    assert_eq!(refused(F_VERSION_1 | F_FLUSH | F_ZONED), F_ZONED);
    assert_eq!(refused(0), 0);
}
