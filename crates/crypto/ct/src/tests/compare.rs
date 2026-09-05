// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use test_support::generators::{bytes, vec};
use test_support::property::check;

use crate::{Choice, ct_copy, ct_eq, ct_select_u8, ct_select_u32, ct_select_u64, ct_swap};

#[test]
fn equal_slices_compare_equal() {
    assert!(ct_eq(b"", b"").is_true());
    assert!(ct_eq(b"a", b"a").is_true());
    assert!(ct_eq(&[0u8; 32], &[0u8; 32]).is_true());
}

#[test]
fn a_difference_in_any_position_is_found() {
    let reference = [7u8; 16];
    for position in 0..reference.len() {
        let mut other = reference;
        if let Some(byte) = other.get_mut(position) {
            *byte ^= 0x80;
        }
        assert!(!ct_eq(&reference, &other).is_true(), "position {position}");
    }
}

#[test]
fn slices_of_different_length_are_unequal() {
    assert!(!ct_eq(b"abc", b"ab").is_true());
    assert!(!ct_eq(b"", b"a").is_true());
}

#[test]
fn selection_returns_the_first_value_for_yes_and_the_second_for_no() {
    assert_eq!(ct_select_u8(Choice::YES, 0xAA, 0x55), 0xAA);
    assert_eq!(ct_select_u8(Choice::NO, 0xAA, 0x55), 0x55);
    assert_eq!(ct_select_u32(Choice::YES, u32::MAX, 0), u32::MAX);
    assert_eq!(ct_select_u32(Choice::NO, u32::MAX, 0), 0);
    assert_eq!(ct_select_u64(Choice::YES, 1, u64::MAX), 1);
    assert_eq!(ct_select_u64(Choice::NO, 1, u64::MAX), u64::MAX);
}

#[test]
fn swap_exchanges_only_on_yes() {
    let mut a = [1u8, 2, 3, 4];
    let mut b = [9u8, 8, 7, 6];
    ct_swap(Choice::NO, &mut a, &mut b);
    assert_eq!(a, [1, 2, 3, 4]);
    assert_eq!(b, [9, 8, 7, 6]);
    ct_swap(Choice::YES, &mut a, &mut b);
    assert_eq!(a, [9, 8, 7, 6]);
    assert_eq!(b, [1, 2, 3, 4]);
}

#[test]
fn swap_and_copy_work_at_every_length() {
    let mut a = [1u8; 1];
    let mut b = [2u8; 1];
    ct_swap(Choice::YES, &mut a, &mut b);
    assert_eq!((a, b), ([2u8; 1], [1u8; 1]));

    let mut wide = [0u8; 64];
    ct_copy(Choice::YES, &mut wide, &[7u8; 64]);
    assert_eq!(wide, [7u8; 64]);

    let mut empty: [u8; 0] = [];
    ct_swap(Choice::YES, &mut empty, &mut []);
    ct_copy(Choice::YES, &mut empty, &[]);
}

#[test]
fn copy_writes_only_on_yes() {
    let mut destination = [0u8; 4];
    ct_copy(Choice::NO, &mut destination, &[1, 2, 3, 4]);
    assert_eq!(destination, [0, 0, 0, 0]);
    ct_copy(Choice::YES, &mut destination, &[1, 2, 3, 4]);
    assert_eq!(destination, [1, 2, 3, 4]);
}

#[test]
fn property_comparison_agrees_with_the_ordinary_one() {
    check("ct_eq_agrees", &vec(bytes(0..=24), 2..=2), |pair| {
        let (Some(left), Some(right)) = (pair.first(), pair.get(1)) else {
            return Err("the generator produced fewer than two values".to_owned());
        };
        if ct_eq(left, right).is_true() != (left == right) {
            return Err(format!("disagreement on {left:?} and {right:?}"));
        }
        if !ct_eq(left, left).is_true() {
            return Err("a slice differs from itself".to_owned());
        }
        Ok(())
    });
}

#[test]
fn property_selection_picks_one_of_its_arguments() {
    check("ct_select_picks", &vec(bytes(2..=2), 1..=1), |values| {
        let Some(pair) = values.first() else {
            return Err("no value".to_owned());
        };
        let (Some(&a), Some(&b)) = (pair.first(), pair.get(1)) else {
            return Err("the generator produced fewer than two bytes".to_owned());
        };
        if ct_select_u8(Choice::YES, a, b) != a || ct_select_u8(Choice::NO, a, b) != b {
            return Err(format!("selection failed for {a} and {b}"));
        }
        Ok(())
    });
}
