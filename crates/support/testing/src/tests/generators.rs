// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::generators`.

use crate::generators::{Generator, bool, bytes, just, one_of, option, pair, range, vec};
use crate::rng::Rng;

fn values<G: Generator>(generator: &G, seed: u64, count: usize) -> Vec<G::Value> {
    let mut rng = Rng::from_seed(seed);
    (0..count)
        .map(|_| generator.generate(&mut rng).into_value())
        .collect()
}

#[test]
fn range_respects_inclusive_bounds() {
    for value in values(&range(10u8..=20), 1, 500) {
        assert!((10..=20).contains(&value));
    }
    for value in values(&range(-5i32..=5), 2, 500) {
        assert!((-5..=5).contains(&value));
    }
}

#[test]
fn single_value_range_always_yields_that_value() {
    assert!(values(&range(7u64..=7), 3, 50).iter().all(|&v| v == 7));
    assert!(values(&range(-3i8..=-3), 4, 50).iter().all(|&v| v == -3));
}

#[test]
fn full_u64_range_produces_distinct_values() {
    let drawn = values(&range(0u64..=u64::MAX), 5, 20);
    assert!(drawn.iter().any(|&v| v != drawn[0]));
}

#[test]
fn range_shrinks_toward_the_value_nearest_zero() {
    let mut rng = Rng::from_seed(6);
    let tree = range(50u32..=100).generate(&mut rng);
    let firsts: Vec<u32> = tree.shrinks().iter().map(|t| *t.value()).collect();
    if *tree.value() > 50 {
        assert_eq!(firsts.first(), Some(&50));
        assert!(firsts.iter().all(|&c| c < *tree.value() && c >= 50));
    }
    let negative = range(-100i32..=-10).generate(&mut rng);
    for child in negative.shrinks() {
        assert!(*child.value() > *negative.value());
        assert!(*child.value() <= -10);
    }
    let spanning = range(-10i32..=10).generate(&mut rng);
    for child in spanning.shrinks() {
        assert!(child.value().abs() < spanning.value().abs());
    }
}

#[test]
fn vec_respects_length_bounds_including_zero() {
    assert!(
        values(&vec(range(0u8..=9), 0..=0), 7, 20)
            .iter()
            .all(Vec::is_empty)
    );
    for drawn in values(&vec(range(0u8..=9), 2..=5), 8, 200) {
        assert!((2..=5).contains(&drawn.len()));
        assert!(drawn.iter().all(|&b| b <= 9));
    }
}

#[test]
fn vec_shrink_candidates_keep_the_minimum_length() {
    let mut rng = Rng::from_seed(9);
    let tree = vec(range(0u8..=9), 3..=6).generate(&mut rng);
    for child in tree.shrinks() {
        assert!(child.value().len() >= 3);
    }
}

#[test]
fn bytes_are_full_range() {
    let drawn: Vec<u8> = values(&bytes(64..=64), 10, 8).concat();
    assert!(drawn.iter().any(|&b| b > 200));
    assert!(drawn.iter().any(|&b| b < 50));
}

#[test]
fn map_and_filter_compose() {
    let even_doubled = range(0u32..=100).filter(|v| v % 2 == 0).map(|v| v * 2);
    for value in values(&even_doubled, 11, 200) {
        assert_eq!(value % 4, 0);
    }
}

#[test]
#[should_panic(expected = "filter rejected")]
fn filter_that_rejects_everything_panics() {
    let mut rng = Rng::from_seed(12);
    let _ = range(0u8..=1).filter(|_| false).generate(&mut rng);
}

#[test]
fn just_one_of_bool_pair_option_behave() {
    assert!(values(&just(3u8), 13, 10).iter().all(|&v| v == 3));
    let choices = values(&one_of(vec!['a', 'b', 'c']), 14, 100);
    assert!(choices.iter().all(|c| ['a', 'b', 'c'].contains(c)));
    assert!(choices.contains(&'c'));
    let bools = values(&bool(), 15, 100);
    assert!(bools.contains(&true) && bools.contains(&false));
    for (a, b) in values(&pair(range(0u8..=1), range(10u8..=11)), 16, 50) {
        assert!(a <= 1 && (10..=11).contains(&b));
    }
    let options = values(&option(range(1u8..=1)), 17, 100);
    assert!(options.contains(&None) && options.contains(&Some(1)));
}

#[test]
fn one_of_shrinks_toward_the_first_entry_and_true_toward_false() {
    let mut rng = Rng::from_seed(18);
    let tree = one_of(vec![10u8, 20, 30]).generate(&mut rng);
    for child in tree.shrinks() {
        assert!(*child.value() < *tree.value());
    }
    let boolean = bool().generate(&mut rng);
    if *boolean.value() {
        assert_eq!(boolean.shrinks().first().map(|t| *t.value()), Some(false));
    }
}

#[test]
fn option_shrinks_to_none_first() {
    let mut rng = Rng::from_seed(19);
    let tree = option(range(1u8..=9)).generate(&mut rng);
    if tree.value().is_some() {
        assert_eq!(tree.shrinks().first().map(|t| *t.value()), Some(None));
    }
}

#[test]
#[should_panic(expected = "range is empty")]
fn empty_range_panics() {
    let (low, high) = (5u8, 4u8);
    let _ = range(low..=high);
}

#[test]
#[should_panic(expected = "one_of needs")]
fn empty_one_of_panics() {
    let _ = one_of(Vec::<u8>::new());
}

#[test]
#[should_panic(expected = "length range is empty")]
fn empty_length_range_panics() {
    let (min, max) = (3usize, 2usize);
    let _ = vec(range(0u8..=1), min..=max);
}

#[test]
fn boxed_generator_is_clonable_and_debuggable() {
    let boxed = range(0u8..=3).boxed();
    let copy = boxed.clone();
    assert!(values(&copy, 20, 10).iter().all(|&v| v <= 3));
    assert_eq!(format!("{boxed:?}"), "BoxGen");
}
