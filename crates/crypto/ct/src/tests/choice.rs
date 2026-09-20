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
fn is_zero_of_a_word_holds_for_zero_alone() {
    assert!(Choice::is_zero_u64(0).is_true());
    for shift in 0u32..64 {
        let value = 1u64.wrapping_shl(shift);
        assert!(!Choice::is_zero_u64(value).is_true(), "bit {shift}");
        assert!(
            !Choice::is_zero_u64(value.wrapping_neg()).is_true(),
            "negated bit {shift}"
        );
    }
    assert!(!Choice::is_zero_u64(u64::MAX).is_true());
    assert!(!Choice::is_zero_u64(1u64.wrapping_shl(63)).is_true());
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
    assert_eq!(Choice::from(true).value(), 1);
    assert_eq!(Choice::from(false).value(), 0);
}

#[test]
fn the_operators_form_the_expected_truth_tables() {
    assert_eq!((!Choice::YES).value(), 0);
    assert_eq!((!Choice::NO).value(), 1);
    assert_eq!((Choice::YES & Choice::YES).value(), 1);
    assert_eq!((Choice::YES & Choice::NO).value(), 0);
    assert_eq!((Choice::NO | Choice::NO).value(), 0);
    assert_eq!((Choice::NO | Choice::YES).value(), 1);
    assert_eq!((Choice::YES ^ Choice::YES).value(), 0);
    assert_eq!((Choice::NO ^ Choice::YES).value(), 1);
}

/// Regression, issue #43: each constructor that derives its byte from a
/// value keeps its `black_box` barrier.
///
/// A barrier changes no answer, so no call can observe it; the guard
/// reads the source, the way the SPDX and asm-option checks of the xtask
/// do.
#[test]
fn every_derived_constructor_keeps_its_barrier() {
    let source = include_str!("../choice.rs");
    for constructed in [
        "Choice(black_box(value & 1))",
        "Choice(black_box(folded.wrapping_shr(7) ^ 1))",
        "Choice(black_box((folded.wrapping_shr(63) as u8) ^ 1))",
        "Choice(black_box(u8::from(value)))",
    ] {
        assert!(source.contains(constructed), "no barrier in {constructed}");
    }
}

/// The barrier changes no answer: the invariant the masks rest on holds
/// for every input.
#[test]
fn the_zero_or_one_invariant_holds_for_every_input() {
    for value in 0u8..=255 {
        for choice in [Choice::from_lsb(value), Choice::is_zero_u8(value)] {
            assert!(choice.value() <= 1, "value {value}");
            assert_eq!(choice.mask_u8(), if choice.is_true() { 0xFF } else { 0x00 });
            assert_eq!(
                choice.mask_u64(),
                if choice.is_true() { u64::MAX } else { 0 }
            );
        }
    }
    for shift in 0u32..64 {
        let choice = Choice::is_zero_u64(1u64.wrapping_shl(shift));
        assert_eq!(choice.value(), 0, "bit {shift}");
    }
    for flag in [true, false] {
        assert_eq!(Choice::from(flag).value(), u8::from(flag));
    }
}

#[test]
fn debug_renders_the_choice() {
    assert!(!format!("{:?}", Choice::YES).is_empty());
}
