// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::mmio`.

use crate::mmio::checked_offset;

/// A page-aligned base address.
const PAGE: usize = 0x10_0000;

#[test]
fn an_aligned_access_inside_the_window_answers_its_offset() {
    assert_eq!(checked_offset(PAGE, 4096, 0, 8), Some(0));
    assert_eq!(checked_offset(PAGE, 4096, 4088, 8), Some(4088));
    assert_eq!(checked_offset(PAGE, 4096, 4095, 1), Some(4095));
    assert_eq!(checked_offset(PAGE, 4096, 2, 2), Some(2));
    assert_eq!(checked_offset(PAGE, 4096, 12, 4), Some(12));
}

#[test]
fn an_access_past_the_end_answers_none() {
    assert_eq!(checked_offset(PAGE, 4096, 4089, 8), None);
    assert_eq!(checked_offset(PAGE, 4096, 4096, 1), None);
    assert_eq!(checked_offset(PAGE, 0, 0, 1), None);
    assert_eq!(checked_offset(PAGE, 4096, usize::MAX, 8), None);
}

#[test]
fn a_misaligned_offset_answers_none() {
    assert_eq!(checked_offset(PAGE, 4096, 4, 8), None);
    assert_eq!(checked_offset(PAGE, 4096, 1, 2), None);
    assert_eq!(checked_offset(PAGE, 4096, 2, 4), None);
}

/// Regression test for issue #99: `Mmio::of(&mut page[1..]).read_u64(0)`
/// read a `u64` at an address congruent to 1 modulo 8, and a window at
/// offset 4 of a page read one congruent to 4.
#[test]
fn a_misaligned_base_answers_none_for_an_aligned_offset() {
    assert_eq!(checked_offset(PAGE + 1, 4095, 0, 8), None);
    assert_eq!(checked_offset(PAGE + 4, 8, 0, 8), None);
    assert_eq!(checked_offset(PAGE + 1, 4095, 0, 2), None);
    assert_eq!(checked_offset(PAGE + 1, 4095, 7, 8), Some(7));
    assert_eq!(checked_offset(PAGE + 1, 4095, 0, 1), Some(0));
}

#[test]
fn an_address_that_overflows_answers_none() {
    assert_eq!(checked_offset(usize::MAX, 16, 8, 8), None);
}

#[test]
fn a_zero_width_answers_none() {
    assert_eq!(checked_offset(PAGE, 4096, 0, 0), None);
}
