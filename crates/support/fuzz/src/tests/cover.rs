// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::cover`.

use crate::cover::{COVER_BYTES, Cover, FEATURE_SLOTS, slot_of};

#[test]
fn a_feature_nothing_reached_is_claimed_and_then_covered() {
    let mut cover = Cover::new();
    assert!(!cover.covers(7, 100));
    assert!(cover.claim(7, 10, true));
    assert!(cover.covers(7, 10));
    assert!(cover.covers(7, 11));
    assert!(!cover.covers(7, 9));
}

#[test]
fn a_larger_input_never_takes_a_feature_over() {
    let mut cover = Cover::new();
    assert!(cover.claim(7, 10, true));
    assert!(!cover.claim(7, 11, true));
    assert!(!cover.claim(7, 10, true));
}

#[test]
fn a_smaller_input_takes_a_feature_over_only_where_the_run_shrinks() {
    let mut cover = Cover::new();
    assert!(cover.claim(7, 10, true));
    assert!(!cover.claim(7, 9, false));
    assert!(cover.claim(7, 9, true));
    assert!(cover.covers(7, 9));
}

#[test]
fn a_feature_past_the_table_shares_a_slot_with_the_one_it_wraps_onto() {
    let mut cover = Cover::new();
    let wrapped = u32::try_from(FEATURE_SLOTS).unwrap_or(0).saturating_add(3);
    assert_eq!(slot_of(wrapped), slot_of(3));
    assert!(cover.claim(3, 10, true));
    assert!(!cover.claim(wrapped, 10, true));
}

#[test]
fn a_table_survives_the_wire() {
    let mut cover = Cover::new();
    assert!(cover.claim(1, 5, true));
    assert!(cover.claim(2, 9, true));
    let bytes = cover.bytes();
    assert_eq!(bytes.len(), COVER_BYTES);
    let mut other = Cover::default();
    other.set_bytes(&bytes);
    assert!(other.covers(1, 5));
    assert!(other.covers(2, 9));
    assert!(!other.covers(3, 4096));
}

#[test]
fn a_table_cut_short_leaves_the_slots_past_it_where_they_were() {
    let mut cover = Cover::new();
    assert!(cover.claim(0, 4, true));
    assert!(cover.claim(1, 4, true));
    let mut other = Cover::new();
    assert!(other.claim(1, 8, true));
    other.set_bytes(cover.bytes().get(..4).unwrap_or_default());
    assert!(other.covers(0, 4));
    assert!(other.covers(1, 8));
    assert!(!other.covers(1, 7));
}
