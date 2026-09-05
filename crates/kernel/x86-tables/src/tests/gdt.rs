// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::gdt`, covering the descriptor items of the catalog
//! 6.6.16.

#![allow(clippy::arithmetic_side_effects)]

use crate::gdt::{
    GDT_ENTRIES, KERNEL_CODE, KERNEL_CODE_SELECTOR, KERNEL_DATA, KERNEL_DATA_SELECTOR, NULL,
    Selector, TSS_INDEX, TSS_LIMIT, TSS_SELECTOR, USER_CODE, USER_CODE_SELECTOR, USER_DATA,
    USER_DATA_SELECTOR, build_gdt, tss_descriptor, tss_descriptor_base, tss_descriptor_limit,
};
use crate::tss::TSS_LEN;
use test_support::generators::range;
use test_support::property::check;

/// Bit 47 of a segment descriptor.
const PRESENT: u64 = 1 << 47;

/// Bit 44 of a segment descriptor: set for code and data.
const NON_SYSTEM: u64 = 1 << 44;

/// Bit 43 of a segment descriptor: set for code.
const EXECUTABLE: u64 = 1 << 43;

/// Bit 41 of a segment descriptor: read for code, write for data.
const READ_WRITE: u64 = 1 << 41;

/// Bit 53 of a segment descriptor: 64-bit code.
const LONG_MODE: u64 = 1 << 53;

fn privilege(descriptor: u64) -> u64 {
    (descriptor >> 45) & 0x3
}

#[test]
fn the_null_descriptor_is_zero() {
    assert_eq!(NULL, 0);
    assert_eq!(build_gdt(0).first().copied(), Some(0));
}

#[test]
fn the_code_and_data_descriptors_carry_the_documented_bits() {
    for descriptor in [KERNEL_CODE, KERNEL_DATA, USER_CODE, USER_DATA] {
        assert_ne!(descriptor & PRESENT, 0, "{descriptor:#x} is present");
        assert_ne!(
            descriptor & NON_SYSTEM,
            0,
            "{descriptor:#x} is no system segment"
        );
        assert_ne!(descriptor & READ_WRITE, 0, "{descriptor:#x} is readable");
    }
    assert_ne!(KERNEL_CODE & EXECUTABLE, 0);
    assert_ne!(USER_CODE & EXECUTABLE, 0);
    assert_eq!(KERNEL_DATA & EXECUTABLE, 0);
    assert_eq!(USER_DATA & EXECUTABLE, 0);
    assert_ne!(KERNEL_CODE & LONG_MODE, 0);
    assert_ne!(USER_CODE & LONG_MODE, 0);
    assert_eq!(privilege(KERNEL_CODE), 0);
    assert_eq!(privilege(KERNEL_DATA), 0);
    assert_eq!(privilege(USER_CODE), 3);
    assert_eq!(privilege(USER_DATA), 3);
}

#[test]
fn the_selectors_carry_the_index_and_the_privilege_level() {
    let cases = [
        (KERNEL_CODE_SELECTOR, 0x08u16, 1u16, 0u16),
        (KERNEL_DATA_SELECTOR, 0x10, 2, 0),
        (USER_DATA_SELECTOR, 0x1B, 3, 3),
        (USER_CODE_SELECTOR, 0x23, 4, 3),
        (TSS_SELECTOR, 0x28, TSS_INDEX, 0),
    ];
    for (selector, raw, index, rpl) in cases {
        assert_eq!(selector.as_u16(), raw);
        assert_eq!(selector.index(), index);
        assert_eq!(selector.rpl(), rpl);
    }
    assert!(KERNEL_CODE_SELECTOR < KERNEL_DATA_SELECTOR);
}

#[test]
fn the_task_state_segment_descriptor_splits_the_base_across_its_fields() {
    let base = 0x0000_1234_5678_9AB0;
    let descriptor = tss_descriptor(base, TSS_LIMIT);
    assert_eq!(tss_descriptor_base(descriptor), base);
    assert_eq!(tss_descriptor_limit(descriptor), TSS_LIMIT);
    let [low, high] = descriptor;
    assert_eq!(
        (low >> 40) & 0xF,
        0x9,
        "an available 64-bit task state segment"
    );
    assert_eq!(low & NON_SYSTEM, 0, "a system descriptor");
    assert_ne!(low & PRESENT, 0);
    assert_eq!(privilege(low), 0);
    assert_eq!(high, base >> 32);
    assert_eq!(TSS_LIMIT, u32::try_from(TSS_LEN).unwrap() - 1);
}

#[test]
fn a_base_at_the_limits_round_trips() {
    for base in [0u64, 0xFFFF_FFFF_FFFF_FFFF, 0x00FF_FFFF, 0x0100_0000] {
        let descriptor = tss_descriptor(base, TSS_LIMIT);
        assert_eq!(tss_descriptor_base(descriptor), base, "base {base:#x}");
    }
    let large = tss_descriptor(0, 0xF_FFFF);
    assert_eq!(tss_descriptor_limit(large), 0xF_FFFF);
    let truncated = tss_descriptor(0, 0x10_0000);
    assert_eq!(
        tss_descriptor_limit(truncated),
        0,
        "a limit needs at most twenty bits"
    );
}

#[test]
fn the_table_places_the_task_state_segment_at_its_index() {
    let base = 0x1234_5678_9ABC_DEF0;
    let table = build_gdt(base);
    assert_eq!(table.len(), GDT_ENTRIES);
    assert_eq!(table.get(1).copied(), Some(KERNEL_CODE));
    assert_eq!(table.get(2).copied(), Some(KERNEL_DATA));
    assert_eq!(table.get(3).copied(), Some(USER_DATA));
    assert_eq!(table.get(4).copied(), Some(USER_CODE));
    let descriptor = [
        table.get(usize::from(TSS_INDEX)).copied().unwrap(),
        table.get(usize::from(TSS_INDEX) + 1).copied().unwrap(),
    ];
    assert_eq!(tss_descriptor_base(descriptor), base);
    assert_eq!(tss_descriptor_limit(descriptor), TSS_LIMIT);
}

#[test]
fn property_every_base_round_trips_through_the_descriptor() {
    check("tss_base_round_trip", &range(0u64..=u64::MAX), |base| {
        let descriptor = tss_descriptor(*base, TSS_LIMIT);
        if tss_descriptor_base(descriptor) == *base {
            Ok(())
        } else {
            Err(format!("{base:#x} did not round-trip"))
        }
    });
    check("selector_round_trip", &range(0u16..=0x1FFF), |index| {
        for rpl in 0..=3u16 {
            let selector = Selector::new(*index, rpl);
            if selector.index() != *index || selector.rpl() != rpl {
                return Err(format!("{selector:?} did not round-trip"));
            }
        }
        Ok(())
    });
}
