// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::rng`.

use crate::rng::Rng;

#[test]
fn same_seed_same_sequence() {
    let mut a = Rng::from_seed(7);
    let mut b = Rng::from_seed(7);
    for _ in 0..100 {
        assert_eq!(a.next_u64(), b.next_u64());
    }
}

#[test]
fn different_seeds_differ() {
    assert_ne!(Rng::from_seed(1).next_u64(), Rng::from_seed(2).next_u64());
}

#[test]
fn below_respects_the_bound_and_reaches_every_value() {
    let mut rng = Rng::from_seed(3);
    let mut seen = [false; 5];
    for _ in 0..1000 {
        let value = rng.below(5);
        assert!(value < 5);
        seen[usize::try_from(value).unwrap()] = true;
    }
    assert!(seen.iter().all(|&s| s));
    assert_eq!(rng.below(0), 0);
    assert_eq!(rng.below(1), 0);
}

#[test]
fn range_is_inclusive_and_order_insensitive() {
    let mut rng = Rng::from_seed(9);
    for _ in 0..200 {
        let value = rng.range(10, 12);
        assert!((10..=12).contains(&value));
        let swapped = rng.range(12, 10);
        assert!((10..=12).contains(&swapped));
    }
    assert_eq!(rng.range(5, 5), 5);
}

#[test]
fn full_range_draws_do_not_loop_forever() {
    let mut rng = Rng::from_seed(11);
    let mut differs = false;
    let first = rng.range(0, u64::MAX);
    for _ in 0..10 {
        differs |= rng.range(0, u64::MAX) != first;
    }
    assert!(differs);
}

#[test]
fn bounds_above_half_the_range_reject_biased_draws_and_stay_in_range() {
    let bound = (1u64 << 63) + 1;
    let mut rng = Rng::from_seed(21);
    for _ in 0..64 {
        assert!(rng.below(bound) < bound);
    }
}

#[test]
fn chance_extremes_are_deterministic() {
    let mut rng = Rng::from_seed(13);
    for _ in 0..50 {
        assert!(rng.chance(1, 1));
        assert!(!rng.chance(0, 1));
    }
    let heads = (0..1000).filter(|_| rng.bool()).count();
    assert!((400..=600).contains(&heads));
}
