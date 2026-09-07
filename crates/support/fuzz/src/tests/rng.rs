// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::rng`.

use crate::rng::Rng;

#[test]
fn a_seed_of_zero_still_generates() {
    let mut zero = Rng::new(0);
    let first = zero.next_u64();
    assert_ne!(first, 0);
    assert_ne!(zero.next_u64(), first);
}

#[test]
fn the_same_seed_yields_the_same_run() {
    let mut one = Rng::new(42);
    let mut two = Rng::new(42);
    for _ in 0..64 {
        assert_eq!(one.next_u64(), two.next_u64());
    }
    assert_ne!(Rng::new(43).next_u64(), Rng::new(42).next_u64());
}

#[test]
fn a_bound_of_zero_yields_zero() {
    let mut rng = Rng::new(7);
    for _ in 0..32 {
        assert_eq!(rng.below(0), 0);
    }
}

#[test]
fn a_draw_stays_below_its_bound_and_reaches_both_ends() {
    let mut rng = Rng::new(9);
    let mut seen = [false; 4];
    for _ in 0..10_000 {
        let drawn = rng.below(4);
        assert!(drawn < 4);
        if let Some(slot) = seen.get_mut(drawn) {
            *slot = true;
        }
    }
    assert!(seen.iter().all(|hit| *hit));
}

#[test]
fn a_range_holds_its_ends_and_an_empty_range_is_its_low_end() {
    let mut rng = Rng::new(11);
    for _ in 0..1000 {
        let drawn = rng.between(3, 6);
        assert!((3..=6).contains(&drawn));
    }
    assert_eq!(rng.between(5, 5), 5);
    assert_eq!(rng.between(9, 2), 9);
}

#[test]
fn a_coin_falls_both_ways() {
    let mut rng = Rng::new(13);
    let heads = (0..1000).filter(|_| rng.coin()).count();
    assert!((300..700).contains(&heads), "{heads} heads of 1000");
}

#[test]
fn a_byte_favours_the_ends_of_its_range_without_leaving_out_the_middle() {
    let mut rng = Rng::new(17);
    let mut counts = [0usize; 256];
    for _ in 0..100_000 {
        if let Some(slot) = counts.get_mut(usize::from(rng.byte())) {
            *slot = slot.saturating_add(1);
        }
    }
    let zero = counts.first().copied().unwrap_or(0);
    let full = counts.last().copied().unwrap_or(0);
    let middle = counts.get(0x42).copied().unwrap_or(0);
    assert!(zero > middle * 4, "{zero} zeroes against {middle}");
    assert!(full > middle * 4, "{full} full bytes against {middle}");
    assert!(middle > 0);
}

#[test]
fn a_shuffle_keeps_the_elements_and_moves_them() {
    let mut rng = Rng::new(19);
    let mut moved = false;
    for _ in 0..32 {
        let mut items = [0u8, 1, 2, 3, 4, 5, 6, 7];
        rng.shuffle(&mut items);
        let mut sorted = items;
        sorted.sort_unstable();
        assert_eq!(sorted, [0, 1, 2, 3, 4, 5, 6, 7]);
        moved |= items != [0, 1, 2, 3, 4, 5, 6, 7];
    }
    assert!(moved);
    let mut one = [9u8];
    rng.shuffle(&mut one);
    assert_eq!(one, [9]);
    let mut none: [u8; 0] = [];
    rng.shuffle(&mut none);
}

#[test]
fn a_slot_is_drawn_from_a_slice_and_an_empty_one_has_none() {
    let mut rng = Rng::new(23);
    let mut items = [0u8, 1, 2, 3];
    let mut seen = [false; 4];
    for _ in 0..10_000 {
        let slot = rng.choose_mut(&mut items).expect("the slice is not empty");
        let at = usize::from(*slot);
        seen[at] = true;
    }
    assert_eq!(seen, [true; 4]);
    let mut none: [u8; 0] = [];
    assert!(rng.choose_mut(&mut none).is_none());
    assert!(rng.choose(&items).is_some());
    assert!(rng.choose::<u8>(&[]).is_none());
}

#[test]
fn a_stretch_lies_inside_the_slice_and_one_too_long_is_not_there() {
    let mut rng = Rng::new(29);
    let mut items = [0u8, 1, 2, 3, 4, 5, 6, 7];
    let mut starts = [false; 6];
    for _ in 0..10_000 {
        let stretch = rng.stretch(&items, 3).expect("three of eight fit");
        assert_eq!(stretch.len(), 3);
        assert!(
            stretch
                .windows(2)
                .all(|pair| pair.last().copied() == pair.first().map(|b| b.saturating_add(1))),
            "the stretch is not one run of the slice: {stretch:?}"
        );
        let start = usize::from(*stretch.first().expect("the stretch is not empty"));
        starts[start] = true;
    }
    assert_eq!(starts, [true; 6]);
    assert_eq!(rng.stretch(&items, 8).map(<[u8]>::len), Some(8));
    assert!(rng.stretch(&items, 9).is_none());
    assert!(rng.stretch::<u8>(&[], 1).is_none());
    assert_eq!(rng.stretch_mut(&mut items, 2).map(|s| s.len()), Some(2));
    assert!(rng.stretch_mut(&mut items, 9).is_none());
}
