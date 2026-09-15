// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::rng`.

use crate::rng::Rng;

#[test]
fn the_same_seed_yields_the_same_sequence() {
    let mut left = Rng::from_seed(7);
    let mut right = Rng::from_seed(7);
    let mut other = Rng::from_seed(8);
    let drawn: Vec<u64> = (0..8).map(|_| left.next_u64()).collect();
    assert_eq!(drawn, (0..8).map(|_| right.next_u64()).collect::<Vec<_>>());
    assert_ne!(drawn, (0..8).map(|_| other.next_u64()).collect::<Vec<_>>());
}

#[test]
fn a_bounded_draw_stays_below_its_bound_and_a_zero_bound_is_zero() {
    let mut rng = Rng::from_seed(1);
    assert_eq!(rng.below(0), 0);
    assert_eq!(rng.below(1), 0);
    for _ in 0..200 {
        assert!(rng.below(5) < 5);
    }
}

#[test]
fn a_range_holds_whichever_way_round_its_bounds_are_given() {
    let mut rng = Rng::from_seed(2);
    for _ in 0..200 {
        let value = rng.range(10, 12);
        assert!((10..=12).contains(&value));
        let reversed = rng.range(12, 10);
        assert!((10..=12).contains(&reversed));
    }
    // The whole range, where the span does not fit in a u64.
    assert_eq!(rng.range(7, 7), 7);
    let _ = rng.range(0, u64::MAX);
}

#[test]
fn an_index_and_a_pick_answer_nothing_for_an_empty_slice() {
    let mut rng = Rng::from_seed(3);
    assert_eq!(rng.index(0), None);
    assert_eq!(rng.pick::<u8>(&[]), None);
    let items = [1_u8, 2, 3];
    for _ in 0..50 {
        let picked = rng.pick(&items).copied().unwrap_or(0);
        assert!(items.contains(&picked));
        assert!(rng.index(items.len()).unwrap_or(9) < 3);
    }
}

#[test]
fn a_chance_of_one_in_one_always_holds_and_of_zero_never_does() {
    let mut rng = Rng::from_seed(4);
    for _ in 0..20 {
        assert!(rng.chance(1, 1));
        assert!(!rng.chance(0, 3));
    }
}
