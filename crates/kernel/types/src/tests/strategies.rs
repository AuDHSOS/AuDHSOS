// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::strategies`.

use crate::strategies::{any_alignment, any_page_range, any_phys_frame_range, any_virt_addr};
use test_support::property::check;

#[test]
fn generated_ranges_are_valid_and_alignments_are_powers_of_two() {
    check("phys_ranges_valid", &any_phys_frame_range(), |range| {
        if range.end_number() > crate::phys::MAX_FRAME_NUMBER.wrapping_add(1) {
            return Err("frame range beyond the limit".into());
        }
        Ok(())
    });
    check(
        "page_ranges_valid",
        &any_page_range(),
        |range| match range.last() {
            Some(last) if last.is_user() != range.is_user() => Err("spans both halves".into()),
            _ => Ok(()),
        },
    );
    check("alignments_valid", &any_alignment(), |align| {
        if align.bytes().is_power_of_two() {
            Ok(())
        } else {
            Err("not a power of two".into())
        }
    });
    check("virt_both_halves", &any_virt_addr(), |_| Ok(()));
}
