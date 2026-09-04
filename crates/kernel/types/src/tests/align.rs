// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::align`.

#![allow(clippy::arithmetic_side_effects)]

use crate::Error;
use crate::align::Alignment;
use crate::strategies::any_alignment;
use test_support::generators::{pair, range};
use test_support::property::check;

#[test]
fn powers_of_two_are_accepted_everything_else_rejected() {
    for shift in 0..64 {
        assert_eq!(
            Alignment::new(1 << shift).map(Alignment::bytes),
            Ok(1 << shift)
        );
    }
    for bad in [0, 3, 6, 4095, 4097, u64::MAX] {
        assert_eq!(Alignment::new(bad), Err(Error::InvalidAlignment(bad)));
    }
}

#[test]
fn page_alignment_constants() {
    assert_eq!(Alignment::PAGE.bytes(), 4096);
    assert_eq!(Alignment::PAGE.log2(), 12);
    assert_eq!(Alignment::PAGE.mask(), 0xFFF);
    assert_eq!(Alignment::BYTE.mask(), 0);
}

#[test]
fn boundaries_around_the_page_alignment() {
    let page = Alignment::PAGE;
    assert!(page.is_aligned(0));
    assert!(page.is_aligned(4096));
    assert!(!page.is_aligned(4095));
    assert!(!page.is_aligned(4097));
    assert_eq!(page.align_down(4095), 0);
    assert_eq!(page.align_down(4097), 4096);
    assert_eq!(page.align_up(4095), Some(4096));
    assert_eq!(page.align_up(4097), Some(8192));
    assert_eq!(page.align_up(4096), Some(4096));
}

#[test]
fn align_up_at_the_top_overflows() {
    assert_eq!(Alignment::PAGE.align_up(u64::MAX), None);
    assert_eq!(
        Alignment::PAGE.align_up(u64::MAX - 4095),
        Some(u64::MAX - 4095)
    );
    assert_eq!(Alignment::PAGE.align_up(u64::MAX - 4094), None);
    assert_eq!(Alignment::BYTE.align_up(u64::MAX), Some(u64::MAX));
}

#[test]
fn large_alignments_two_mib_and_one_gib() {
    let two_mib = Alignment::new(2 << 20).unwrap();
    let one_gib = Alignment::new(1 << 30).unwrap();
    assert_eq!(two_mib.align_up((2 << 20) - 1), Some(2 << 20));
    assert_eq!(one_gib.align_down((1 << 30) + 1), 1 << 30);
    assert!(!one_gib.is_aligned((1 << 30) - 1));
    assert!(one_gib.is_aligned(1 << 31));
}

#[test]
fn property_align_down_and_up_bracket_the_value() {
    let generator = pair(any_alignment(), range(0u64..=u64::MAX));
    check("alignment_brackets", &generator, |&(align, value)| {
        let down = align.align_down(value);
        if down > value || !align.is_aligned(down) || value - down >= align.bytes() {
            return Err(format!("align_down({value:#x}) = {down:#x} is wrong"));
        }
        match align.align_up(value) {
            Some(up) if up < value || !align.is_aligned(up) || up - value >= align.bytes() => {
                Err(format!("align_up({value:#x}) = {up:#x} is wrong"))
            }
            None if value <= u64::MAX - align.mask() => Err("align_up overflowed early".into()),
            _ => Ok(()),
        }
    });
}
