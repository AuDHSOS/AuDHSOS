// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::lapic`.

use crate::lapic::{
    DIVIDE_BY_1, DIVIDE_BY_16, EOI, ID, LVT_ERROR, LVT_LINT0, LVT_LINT1, LVT_MASKED, LVT_PERIODIC,
    LVT_TIMER, SVR, SVR_ENABLE, TIMER_CURRENT, TIMER_DIVIDE, TIMER_INITIAL, TPR, VERSION,
    count_for_rate, lvt, lvt_masked, lvt_periodic, lvt_vector, spurious,
};

#[test]
fn every_register_sits_where_the_specification_says() {
    assert_eq!(
        [
            ID,
            VERSION,
            TPR,
            EOI,
            SVR,
            LVT_TIMER,
            LVT_LINT0,
            LVT_LINT1,
            LVT_ERROR,
            TIMER_INITIAL,
            TIMER_CURRENT,
            TIMER_DIVIDE,
        ],
        [
            0x20, 0x30, 0x80, 0xB0, 0xF0, 0x320, 0x350, 0x360, 0x370, 0x380, 0x390, 0x3E0,
        ]
    );
}

#[test]
fn the_spurious_register_carries_the_enable_bit_and_the_vector() {
    let value = spurious(0xFF);
    assert_eq!(value & 0xFF, 0xFF);
    assert_eq!(value & SVR_ENABLE, SVR_ENABLE);
    assert_eq!(value, 0x1FF);
}

#[test]
fn a_masked_periodic_entry_carries_both_bits_and_its_vector() {
    let entry = lvt(0x30, true, true);
    assert_eq!(lvt_vector(entry), 0x30);
    assert!(lvt_masked(entry));
    assert!(lvt_periodic(entry));
    assert_eq!(entry, 0x30 | LVT_MASKED | LVT_PERIODIC);
}

#[test]
fn an_unmasked_one_shot_entry_carries_neither_bit() {
    let entry = lvt(0x40, false, false);
    assert_eq!(entry, 0x40);
    assert!(!lvt_masked(entry));
    assert!(!lvt_periodic(entry));
    assert_eq!(lvt_vector(entry), 0x40);
}

#[test]
fn the_two_divisors_the_kernel_uses_have_the_values_of_the_split_field() {
    assert_eq!(DIVIDE_BY_16, 0b0011);
    assert_eq!(DIVIDE_BY_1, 0b1011);
}

#[test]
fn the_count_for_a_rate_is_the_bus_clock_divided_by_it() {
    assert_eq!(count_for_rate(1000, 1000), Some(1000));
    assert_eq!(count_for_rate(1000, 100), Some(10_000));
    assert_eq!(count_for_rate(1000, 1), Some(1_000_000));
}

#[test]
fn a_rate_the_timer_cannot_produce_has_no_count() {
    assert_eq!(count_for_rate(1000, 0), None, "no ticks at all");
    assert_eq!(
        count_for_rate(1000, 2_000_000),
        None,
        "faster than the timer counts"
    );
    assert_eq!(count_for_rate(0, 1000), None, "a bus clock that stands");
    assert_eq!(
        count_for_rate(u32::MAX, 1),
        None,
        "a count beyond the register"
    );
}
