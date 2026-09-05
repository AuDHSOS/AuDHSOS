// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use crate::Choice;

#[test]
fn the_two_constants_carry_one_and_zero() {
    assert_eq!(Choice::YES.value(), 1);
    assert_eq!(Choice::NO.value(), 0);
    assert!(Choice::YES.is_true());
    assert!(!Choice::NO.is_true());
}

#[test]
fn from_lsb_keeps_the_lowest_bit_only() {
    for value in 0u8..=255 {
        let choice = Choice::from_lsb(value);
        assert_eq!(choice.value(), value & 1);
        assert!(choice.value() <= 1);
    }
}

#[test]
fn is_zero_holds_for_zero_alone() {
    assert!(Choice::is_zero_u8(0).is_true());
    for value in 1u8..=255 {
        assert!(!Choice::is_zero_u8(value).is_true(), "value {value}");
    }
}

#[test]
fn masks_are_all_ones_or_all_zeros() {
    assert_eq!(Choice::YES.mask_u8(), 0xFF);
    assert_eq!(Choice::NO.mask_u8(), 0x00);
    assert_eq!(Choice::YES.mask_u32(), u32::MAX);
    assert_eq!(Choice::NO.mask_u32(), 0);
    assert_eq!(Choice::YES.mask_u64(), u64::MAX);
    assert_eq!(Choice::NO.mask_u64(), 0);
}

#[test]
fn a_bool_converts_to_the_matching_choice() {
    assert_eq!(Choice::from(true), Choice::YES);
    assert_eq!(Choice::from(false), Choice::NO);
}

#[test]
fn the_operators_form_the_expected_truth_tables() {
    assert_eq!(!Choice::YES, Choice::NO);
    assert_eq!(!Choice::NO, Choice::YES);
    assert_eq!(Choice::YES & Choice::YES, Choice::YES);
    assert_eq!(Choice::YES & Choice::NO, Choice::NO);
    assert_eq!(Choice::NO | Choice::NO, Choice::NO);
    assert_eq!(Choice::NO | Choice::YES, Choice::YES);
    assert_eq!(Choice::YES ^ Choice::YES, Choice::NO);
    assert_eq!(Choice::NO ^ Choice::YES, Choice::YES);
}

#[test]
fn debug_renders_the_choice() {
    assert!(!format!("{:?}", Choice::YES).is_empty());
}
