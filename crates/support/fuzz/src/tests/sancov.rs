// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::sancov`.
//!
//! The callbacks are what the compiler emits calls to, so a test calls
//! them the way the compiled target would: inside a run, with the values a
//! comparison had. They touch the one trace the process has, so they hold
//! the lock of `super::GLOBALS`.

use crate::dictionary::{Compare, TABLE_SIZE};
use crate::sancov::{
    __sanitizer_cov_pcs_init, __sanitizer_cov_trace_cmp1, __sanitizer_cov_trace_cmp2,
    __sanitizer_cov_trace_cmp4, __sanitizer_cov_trace_cmp8, __sanitizer_cov_trace_const_cmp1,
    __sanitizer_cov_trace_const_cmp2, __sanitizer_cov_trace_const_cmp4,
    __sanitizer_cov_trace_const_cmp8, __sanitizer_cov_trace_pc_indir, __sanitizer_cov_trace_switch,
    record, with_trace,
};

use super::GLOBALS;

/// The slot of the table that a comparison of `left` and `right` lands in.
fn slot(left: u64, right: u64) -> usize {
    usize::try_from(left ^ right).unwrap_or(0) % TABLE_SIZE
}

/// Empties both tables so that one test does not read another's leavings.
fn clear_tables() {
    with_trace(|trace| {
        for index in 0..TABLE_SIZE {
            let _ = index;
        }
        trace.compares4 = crate::dictionary::Compares::new(4);
        trace.compares8 = crate::dictionary::Compares::new(8);
        trace.values.clear();
        trace.value_profile = false;
    });
}

#[test]
fn a_comparison_outside_a_run_is_dropped() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    clear_tables();
    __sanitizer_cov_trace_cmp4(0x1111_1111, 0x2222_2222);
    __sanitizer_cov_trace_cmp8(1, 2);
    __sanitizer_cov_trace_const_cmp4(7, 9);
    with_trace(|trace| {
        assert_eq!(
            trace.compares4.get(slot(0x1111_1111, 0x2222_2222)),
            Compare::default()
        );
        assert_eq!(trace.compares8.get(slot(1, 2)), Compare::default());
    });
    drop(guard);
}

#[test]
fn a_four_byte_comparison_of_a_run_lands_in_the_four_byte_table() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    clear_tables();
    record(|| {
        __sanitizer_cov_trace_cmp4(0x3132_3334, 0x3132_3335);
        __sanitizer_cov_trace_const_cmp4(0xdead_beef, 0);
    });
    with_trace(|trace| {
        assert_eq!(
            trace.compares4.get(slot(0x3132_3334, 0x3132_3335)),
            Compare {
                left: 0x3132_3334,
                right: 0x3132_3335
            }
        );
        assert_eq!(
            trace.compares4.get(slot(0xdead_beef, 0)),
            Compare {
                left: 0xdead_beef,
                right: 0
            }
        );
    });
    drop(guard);
}

#[test]
fn an_eight_byte_comparison_lands_in_the_eight_byte_table() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    clear_tables();
    record(|| {
        __sanitizer_cov_trace_cmp8(0x0102_0304_0506_0708, 9);
        __sanitizer_cov_trace_const_cmp8(0x1000, 0x2000);
    });
    with_trace(|trace| {
        assert_eq!(
            trace.compares8.get(slot(0x0102_0304_0506_0708, 9)).left,
            0x0102_0304_0506_0708
        );
        assert_eq!(trace.compares8.get(slot(0x1000, 0x2000)).right, 0x2000);
    });
    drop(guard);
}

#[test]
fn a_narrow_comparison_is_seen_but_kept_out_of_both_tables() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    clear_tables();
    with_trace(|trace| trace.value_profile = true);
    record(|| {
        __sanitizer_cov_trace_cmp1(0x30, 0x31);
        __sanitizer_cov_trace_cmp2(0x3031, 0x3131);
        __sanitizer_cov_trace_const_cmp1(0x0d, 0x0a);
        __sanitizer_cov_trace_const_cmp2(0x0102, 0x0102);
    });
    with_trace(|trace| {
        assert_eq!(trace.compares4.get(slot(0x30, 0x31)), Compare::default());
        assert_eq!(trace.compares8.get(slot(0x30, 0x31)), Compare::default());
        let mut bits = 0usize;
        trace.values.for_each(|_| bits += 1);
        assert!(bits > 0, "the value profile kept nothing");
        trace.value_profile = false;
    });
    drop(guard);
}

#[test]
fn the_value_profile_is_empty_unless_it_was_asked_for() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    clear_tables();
    record(|| __sanitizer_cov_trace_const_cmp8(0x1234, 0x5678));
    with_trace(|trace| {
        let mut bits = 0usize;
        trace.values.for_each(|_| bits += 1);
        assert_eq!(bits, 0);
    });
    drop(guard);
}

#[test]
fn a_switch_over_wide_values_leaves_the_two_arms_the_value_falls_between() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    clear_tables();
    // The two arms differ in their low bits, so that the pair the value
    // falls between takes two slots of the table and not one.
    let table: [u64; 6] = [4, 64, 0x100, 0x200, 0x40f, 0x800];
    record(|| {
        // SAFETY: the table is laid out the way the compiler lays one out:
        // the number of arms, the width in bits, then the arms in order.
        unsafe { __sanitizer_cov_trace_switch(0x300, table.as_ptr()) };
    });
    with_trace(|trace| {
        assert_eq!(trace.compares8.get(slot(0x200, 0x300)).left, 0x200);
        assert_eq!(trace.compares8.get(slot(0x40f, 0x300)).left, 0x40f);
    });
    drop(guard);
}

#[test]
fn a_switch_that_carries_no_signal_is_dropped() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    clear_tables();
    let small: [u64; 5] = [3, 32, 1, 2, 3];
    let wide: [u64; 4] = [2, 32, 0x1000, 0x2000];
    let none: [u64; 2] = [0, 32];
    record(|| {
        // SAFETY: a table whose arms are all small, which carries nothing.
        unsafe { __sanitizer_cov_trace_switch(2, small.as_ptr()) };
        // SAFETY: a well-formed table against a value that is small.
        unsafe { __sanitizer_cov_trace_switch(7, wide.as_ptr()) };
        // SAFETY: a table of no arms at all.
        unsafe { __sanitizer_cov_trace_switch(0x1500, none.as_ptr()) };
        // SAFETY: no table, which the callback checks for before it reads.
        unsafe { __sanitizer_cov_trace_switch(0x1500, core::ptr::null()) };
    });
    with_trace(|trace| {
        for index in 0..TABLE_SIZE {
            assert_eq!(trace.compares8.get(index), Compare::default());
            assert_eq!(trace.compares4.get(index), Compare::default());
        }
    });
    drop(guard);
}

#[test]
fn a_switch_below_every_arm_falls_between_nothing_and_the_first() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    clear_tables();
    let table: [u64; 4] = [2, 64, 0x1007, 0x2000];
    record(|| {
        // SAFETY: a well-formed table, as above.
        unsafe { __sanitizer_cov_trace_switch(0x800, table.as_ptr()) };
    });
    with_trace(|trace| {
        assert_eq!(trace.compares8.get(slot(0x1007, 0x800)).left, 0x1007);
        assert_eq!(trace.compares8.get(slot(0, 0x800)).right, 0x800);
    });
    drop(guard);
}

#[test]
fn a_switch_over_four_byte_values_lands_in_the_four_byte_table() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    clear_tables();
    let table: [u64; 4] = [2, 32, 0x1007, 0x2000];
    record(|| {
        // SAFETY: a well-formed table whose values are four bytes wide.
        unsafe { __sanitizer_cov_trace_switch(0x800, table.as_ptr()) };
    });
    with_trace(|trace| {
        assert_eq!(trace.compares4.get(slot(0x1007, 0x800)).left, 0x1007);
        assert_eq!(trace.compares8.get(slot(0x1007, 0x800)), Compare::default());
    });
    drop(guard);
}

#[test]
fn the_program_table_says_how_many_blocks_the_compiler_instrumented() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let before = crate::counters::blocks();
    let table = [0usize; 8];
    let start = table.as_ptr();
    // SAFETY: eight words, which is four pairs, which is four blocks.
    unsafe { __sanitizer_cov_pcs_init(start, start.wrapping_add(8)) };
    assert_eq!(crate::counters::blocks(), before + 4);
    // SAFETY: an empty table, which describes nothing.
    unsafe { __sanitizer_cov_pcs_init(start, start) };
    assert_eq!(crate::counters::blocks(), before + 4);
    drop(guard);
}

#[test]
fn an_indirect_call_is_taken_and_kept_from_nowhere() {
    __sanitizer_cov_trace_pc_indir(0x1234);
}
