// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::feature`.

use crate::feature::{BUCKETS, ValueMap, bucket, feature_of};

#[test]
fn a_counter_lands_in_the_bucket_of_its_size() {
    assert_eq!(bucket(0), 0);
    assert_eq!(bucket(1), 0);
    assert_eq!(bucket(2), 1);
    assert_eq!(bucket(3), 2);
    for counter in 4..=7 {
        assert_eq!(bucket(counter), 3);
    }
    for counter in 8..=15 {
        assert_eq!(bucket(counter), 4);
    }
    for counter in 16..=31 {
        assert_eq!(bucket(counter), 5);
    }
    for counter in 32..=127 {
        assert_eq!(bucket(counter), 6);
    }
    for counter in 128..=255u8 {
        assert_eq!(bucket(counter), 7);
    }
}

#[test]
fn two_counters_never_share_a_feature() {
    assert_eq!(feature_of(0, 1), 0);
    assert_eq!(feature_of(0, 2), 1);
    assert_eq!(feature_of(1, 1), BUCKETS);
    assert_eq!(feature_of(1, 128), BUCKETS + 7);
    assert_ne!(feature_of(5, 1), feature_of(4, 255));
}

#[test]
fn a_counter_number_that_will_not_fit_saturates_instead_of_wrapping() {
    assert_eq!(feature_of(usize::MAX, 1), u32::MAX);
}

#[test]
fn a_value_map_reports_a_bit_as_new_once() {
    let mut map = ValueMap::new();
    assert!(map.add(1234));
    assert!(!map.add(1234));
    assert!(map.add(1235));
}

#[test]
fn a_value_map_wraps_around_its_size() {
    let mut map = ValueMap::new();
    assert!(map.add(3));
    assert!(!map.add(3 + (1 << 16)));
}

#[test]
fn a_value_map_lists_what_it_holds_and_forgets_it_when_cleared() {
    let mut map = ValueMap::default();
    for value in [0u64, 1, 63, 64, 65, 4095, 65535] {
        assert!(map.add(value));
    }
    let mut seen = Vec::new();
    map.for_each(|bit| seen.push(bit));
    assert_eq!(seen, vec![0, 1, 63, 64, 65, 4095, 65535]);
    map.clear();
    let mut after = 0usize;
    map.for_each(|_| after += 1);
    assert_eq!(after, 0);
}
