// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::rect`.

use crate::rect::{DAMAGE_CAPACITY, Damage, Rect};

#[test]
fn a_rectangle_of_zero_width_or_height_is_empty() {
    assert!(Rect::new(1, 1, 0, 5).is_empty());
    assert!(Rect::new(1, 1, 5, 0).is_empty());
    assert!(!Rect::new(1, 1, 5, 5).is_empty());
    assert!(Rect::EMPTY.is_empty());
}

#[test]
fn the_edges_of_a_rectangle_saturate_instead_of_wrapping() {
    let rect = Rect::new(u32::MAX, u32::MAX, 4, 4);
    assert_eq!(rect.right(), u32::MAX);
    assert_eq!(rect.bottom(), u32::MAX);
}

#[test]
fn a_rectangle_contains_its_first_pixel_and_not_the_one_past_its_last() {
    let rect = Rect::new(2, 3, 4, 5);
    assert!(rect.contains(2, 3));
    assert!(rect.contains(5, 7));
    assert!(!rect.contains(6, 7));
    assert!(!rect.contains(5, 8));
    assert!(!rect.contains(1, 3));
    assert!(!rect.contains(2, 2));
}

#[test]
fn two_overlapping_rectangles_intersect_in_what_both_hold() {
    let left = Rect::new(0, 0, 10, 10);
    let right = Rect::new(5, 5, 10, 10);
    assert_eq!(left.intersect(right), Rect::new(5, 5, 5, 5));
    assert!(left.overlaps(right));
}

#[test]
fn rectangles_that_share_no_pixel_intersect_in_nothing() {
    let left = Rect::new(0, 0, 5, 5);
    assert_eq!(left.intersect(Rect::new(5, 0, 5, 5)), Rect::EMPTY);
    assert_eq!(left.intersect(Rect::new(0, 5, 5, 5)), Rect::EMPTY);
    assert!(!left.overlaps(Rect::new(5, 0, 5, 5)));
}

#[test]
fn an_empty_rectangle_intersects_nothing() {
    assert_eq!(Rect::EMPTY.intersect(Rect::new(0, 0, 4, 4)), Rect::EMPTY);
    assert_eq!(Rect::new(0, 0, 4, 4).intersect(Rect::EMPTY), Rect::EMPTY);
}

#[test]
fn the_union_of_two_rectangles_encloses_both() {
    let left = Rect::new(0, 0, 4, 4);
    let right = Rect::new(6, 8, 2, 2);
    assert_eq!(left.union(right), Rect::new(0, 0, 8, 10));
    assert_eq!(right.union(left), Rect::new(0, 0, 8, 10));
}

#[test]
fn an_empty_rectangle_leaves_the_other_one_as_it_is() {
    let rect = Rect::new(3, 4, 5, 6);
    assert_eq!(rect.union(Rect::EMPTY), rect);
    assert_eq!(Rect::EMPTY.union(rect), rect);
}

#[test]
fn clipping_keeps_the_part_inside_the_surface() {
    let rect = Rect::new(6, 6, 10, 10);
    assert_eq!(rect.clip_to(8, 12), Rect::new(6, 6, 2, 6));
    assert_eq!(rect.clip_to(4, 4), Rect::EMPTY);
}

#[test]
fn a_damage_set_starts_empty() {
    let damage = Damage::new();
    assert!(damage.is_empty());
    assert_eq!(damage.len(), 0);
    assert!(!damage.is_full());
    assert_eq!(damage.bounds(), Rect::EMPTY);
    assert_eq!(damage.iter().count(), 0);
    assert_eq!(damage, Damage::default());
}

#[test]
fn an_empty_rectangle_damages_nothing() {
    let mut damage = Damage::new();
    damage.push(Rect::new(1, 1, 0, 4));
    damage.push(Rect::EMPTY);
    assert!(damage.is_empty());
}

#[test]
fn two_rectangles_that_do_not_meet_are_kept_apart() {
    let mut damage = Damage::new();
    damage.push(Rect::new(0, 0, 2, 2));
    damage.push(Rect::new(10, 10, 2, 2));
    assert_eq!(damage.len(), 2);
    assert_eq!(damage.bounds(), Rect::new(0, 0, 12, 12));
}

#[test]
fn a_rectangle_that_overlaps_one_of_the_set_is_merged_into_it() {
    let mut damage = Damage::new();
    damage.push(Rect::new(0, 0, 4, 4));
    damage.push(Rect::new(2, 2, 4, 4));
    assert_eq!(damage.len(), 1);
    assert_eq!(damage.iter().next(), Some(Rect::new(0, 0, 6, 6)));
}

#[test]
fn a_full_damage_set_collapses_to_the_rectangle_that_encloses_it() {
    let mut damage = Damage::new();
    for index in 0..DAMAGE_CAPACITY {
        let step = u32::try_from(index).unwrap_or(0).saturating_mul(4);
        damage.push(Rect::new(step, step, 2, 2));
    }
    assert_eq!(damage.len(), DAMAGE_CAPACITY);
    assert!(damage.is_full());
    let last = u32::try_from(DAMAGE_CAPACITY)
        .unwrap_or(0)
        .saturating_mul(4);
    damage.push(Rect::new(last, last, 2, 2));
    assert_eq!(damage.len(), 1);
    assert_eq!(
        damage.iter().next(),
        Some(Rect::new(
            0,
            0,
            last.saturating_add(2),
            last.saturating_add(2)
        ))
    );
}

#[test]
fn clearing_a_damage_set_forgets_every_rectangle() {
    let mut damage = Damage::new();
    damage.push(Rect::new(1, 1, 2, 2));
    damage.clear();
    assert!(damage.is_empty());
    assert_eq!(damage.iter().count(), 0);
}
