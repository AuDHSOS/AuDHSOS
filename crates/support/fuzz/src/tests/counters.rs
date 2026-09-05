// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::counters`.
//!
//! They register ranges with the one registry the process has, so they
//! hold the lock of `super::GLOBALS` while they run. Miri skips them: what
//! they check is the handling of ranges the linker placed, which Miri has
//! no way to produce and no reason to model.

use crate::counters::{blocks, clear, for_each_nonzero, register, total};

use super::{GLOBALS, region_at, register_counters};

#[test]
#[cfg_attr(miri, ignore)]
fn a_registered_range_is_read_back_counter_by_counter() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let before = total();
    // A length that is not a whole number of blocks, so that both the
    // block loop and the tail after it are walked.
    let address = register_counters(100);
    assert_eq!(total(), before + 100);

    // SAFETY: the lock is held and nothing else holds this range.
    let region = unsafe { region_at(address, 100) };
    region[0] = 1;
    region[9] = 200;
    region[23] = 3;
    region[64] = 4;
    region[97] = 5;

    let mut seen = Vec::new();
    for_each_nonzero(|index, count| seen.push((index, count)));
    let mine: Vec<(usize, u8)> = seen
        .into_iter()
        .filter(|(index, _)| *index >= before)
        .map(|(index, count)| (index - before, count))
        .collect();
    assert_eq!(mine, vec![(0, 1), (9, 200), (23, 3), (64, 4), (97, 5)]);

    clear();
    let mut after = 0usize;
    for_each_nonzero(|_, _| after += 1);
    assert_eq!(after, 0);
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_range_that_is_empty_or_turned_around_is_ignored() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let before = total();
    let mut byte = 0u8;
    let pointer = &raw mut byte;
    // SAFETY: an empty range, which `register` drops before it reads it.
    unsafe { register(pointer, pointer) };
    // SAFETY: a range whose end is before its start, dropped as well.
    unsafe { register(pointer.wrapping_add(8), pointer) };
    assert_eq!(total(), before);
    drop(guard);
}

#[test]
fn the_program_table_is_counted_in_blocks() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let before = blocks();
    crate::counters::add_blocks(5);
    assert_eq!(blocks(), before + 5);
    drop(guard);
}
