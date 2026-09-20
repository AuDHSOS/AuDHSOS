// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Regression, issue #42: `Scalar` answers equality as a [`Choice`] that
//! folds every limb, rather than as a derived `==` that stops at the first
//! differing limb.

use crypto_ct::Choice;

use crate::scalar::{BYTES, Scalar};

/// The scalar a single limb set to `value` encodes, for the differences
/// that must be found in each of the four limbs.
fn with_limb(limb: usize, value: u64) -> Scalar {
    let mut bytes = [0u8; BYTES];
    for (offset, byte) in value.to_le_bytes().into_iter().enumerate() {
        if let Some(slot) = bytes.get_mut(limb.wrapping_mul(8).wrapping_add(offset)) {
            *slot = byte;
        }
    }
    Scalar::from_bytes_reduced(&bytes)
}

#[test]
fn ct_eq_holds_for_equal_scalars_alone() {
    let one = with_limb(0, 1);
    assert!(one.ct_eq(one).is_true());
    assert!(Scalar::ZERO.ct_eq(Scalar::ZERO).is_true());
    assert!(!one.ct_eq(Scalar::ZERO).is_true());
}

#[test]
fn ct_eq_finds_a_difference_in_every_limb() {
    // A derived `==` returns on the first differing limb; the fold has to
    // reach the top one as well.
    for limb in 0..4 {
        let value = with_limb(limb, 1);
        assert!(!value.ct_eq(Scalar::ZERO).is_true(), "limb {limb}");
        assert!(!Scalar::ZERO.ct_eq(value).is_true(), "limb {limb}");
        assert!(value.ct_eq(value).is_true(), "limb {limb}");
    }
}

#[test]
fn is_zero_holds_for_the_zero_scalar_alone() {
    assert!(Scalar::ZERO.is_zero().is_true());
    assert!(!with_limb(0, 1).is_zero().is_true());
    for limb in 0..4 {
        assert!(!with_limb(limb, 1).is_zero().is_true(), "limb {limb}");
    }
}

#[test]
fn ct_eq_answers_a_choice_carrying_zero_or_one() {
    let value = with_limb(2, 0x0102_0304_0506_0708);
    for (left, right) in [(value, value), (value, Scalar::ZERO)] {
        let choice: Choice = left.ct_eq(right);
        assert!(choice.value() <= 1);
    }
}
