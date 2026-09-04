// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::rights`.

#![allow(clippy::arithmetic_side_effects)]

use crate::Error;
use crate::rights::Rights;
use std::collections::HashSet;
use test_support::generators::Generator;
use test_support::property::check;

#[test]
fn every_named_right_has_a_unique_single_bit() {
    let mut seen = HashSet::new();
    for &(right, _) in Rights::NAMED {
        assert_eq!(right.bits().count_ones(), 1);
        assert!(seen.insert(right.bits()));
    }
    assert_eq!(seen.len(), Rights::NAMED.len());
}

#[test]
fn all_is_the_union_of_every_named_right() {
    let union = Rights::NAMED
        .iter()
        .fold(Rights::EMPTY, |acc, &(r, _)| acc | r);
    assert_eq!(union, Rights::ALL);
}

#[test]
fn empty_set_is_subset_of_everything_and_contains_nothing() {
    assert!(Rights::EMPTY.is_empty());
    assert!(Rights::EMPTY.is_subset_of(Rights::EMPTY));
    assert!(Rights::EMPTY.is_subset_of(Rights::ALL));
    for &(right, _) in Rights::NAMED {
        assert!(!Rights::EMPTY.contains(right));
        assert!(Rights::ALL.contains(right));
    }
}

#[test]
fn full_set_is_subset_only_of_itself() {
    assert!(Rights::ALL.is_subset_of(Rights::ALL));
    for &(right, _) in Rights::NAMED {
        assert!(!Rights::ALL.is_subset_of(Rights::ALL.difference(right)));
    }
}

#[test]
fn subset_and_superset_for_every_pair_of_single_rights() {
    for &(a, _) in Rights::NAMED {
        for &(b, _) in Rights::NAMED {
            assert_eq!(a.is_subset_of(b), a == b);
            assert!(a.is_subset_of(a | b));
            assert!(!(a | b).is_subset_of(a) || a == b);
        }
    }
}

#[test]
fn union_and_intersection_follow_set_semantics() {
    let a = Rights::READ | Rights::WRITE;
    let b = Rights::WRITE | Rights::MAP;
    assert_eq!(a | b, Rights::READ | Rights::WRITE | Rights::MAP);
    assert_eq!(a & b, Rights::WRITE);
    assert_eq!(a.difference(b), Rights::READ);
    assert_eq!(a & Rights::EMPTY, Rights::EMPTY);
}

#[test]
fn unknown_bits_are_rejected_on_decode() {
    assert_eq!(Rights::from_bits(Rights::ALL.bits()), Ok(Rights::ALL));
    assert_eq!(Rights::from_bits(0), Ok(Rights::EMPTY));
    let first_unknown = Rights::ALL.bits() + 1;
    assert_eq!(
        Rights::from_bits(first_unknown),
        Err(Error::InvalidArgument)
    );
    assert_eq!(Rights::from_bits(u32::MAX), Err(Error::InvalidArgument));
}

#[test]
fn debug_lists_names_or_empty() {
    assert_eq!(format!("{:?}", Rights::EMPTY), "Rights(EMPTY)");
    assert_eq!(
        format!("{:?}", Rights::READ | Rights::TRANSFER),
        "Rights(READ | TRANSFER)"
    );
}

#[test]
fn property_bits_round_trip_and_subset_is_a_partial_order() {
    let any_rights = test_support::generators::range(0u32..=Rights::ALL.bits())
        .filter(|bits| Rights::from_bits(*bits).is_ok())
        .map(|bits| Rights::from_bits(bits).unwrap());
    let pair = test_support::generators::pair(any_rights.clone(), any_rights);
    check("rights_partial_order", &pair, |&(a, b)| {
        if Rights::from_bits(a.bits()) != Ok(a) {
            return Err("bits do not round-trip".into());
        }
        if !(a & b).is_subset_of(a) || !a.is_subset_of(a | b) {
            return Err("intersection or union violates subset order".into());
        }
        if a.is_subset_of(b) && b.is_subset_of(a) && a != b {
            return Err("antisymmetry violated".into());
        }
        Ok(())
    });
}
