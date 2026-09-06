// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Handles: what a generation catches and what it costs.

use crate::error::StackError;
use crate::handle::Slots;

#[test]
fn a_handle_names_the_slot_it_was_made_for() {
    let slots = Slots::<4>::new();
    let handle = slots.handle(2).expect("a slot");
    assert_eq!(handle.index(), 2);
    assert_eq!(slots.resolve(handle), Ok(2));
    assert_eq!(handle.generation(), 1);
}

#[test]
fn a_handle_to_a_slot_that_was_given_up_is_stale() {
    let mut slots = Slots::<4>::new();
    let old = slots.handle(1).expect("a slot");
    slots.retire(1);
    assert_eq!(slots.resolve(old), Err(StackError::Stale));
    let new = slots.handle(1).expect("a slot");
    assert_ne!(new, old);
    assert_eq!(slots.resolve(new), Ok(1));
    // And the other slots are untouched by it.
    let other = slots.handle(0).expect("a slot");
    assert_eq!(slots.resolve(other), Ok(0));
}

#[test]
fn a_handle_to_no_slot_at_all_is_refused() {
    let slots = Slots::<2>::new();
    assert_eq!(slots.handle(2), Err(StackError::Unknown));
    assert_eq!(slots.handle(usize::MAX), Err(StackError::Unknown));
    // A handle made for a larger table names no slot of a smaller one.
    let wide = Slots::<8>::new();
    let handle = wide.handle(5).expect("a slot");
    assert_eq!(slots.resolve(handle), Err(StackError::Unknown));
}

#[test]
fn a_slot_that_has_been_through_every_generation_hands_out_no_more() {
    let mut slots = Slots::<1>::new();
    for _ in 0..u32::from(u16::MAX) {
        slots.retire(0);
    }
    assert_eq!(slots.handle(0), Err(StackError::Full));
    // The last handle it did hand out is still stale, and never valid
    // again.
    let mut fresh = Slots::<1>::new();
    let handle = fresh.handle(0).expect("a slot");
    for _ in 0..u32::from(u16::MAX) {
        fresh.retire(0);
    }
    assert_eq!(fresh.resolve(handle), Err(StackError::Stale));
}

#[test]
fn a_table_of_slots_starts_at_the_first_generation() {
    let slots = Slots::<3>::default();
    for index in 0..3 {
        assert_eq!(slots.handle(index).expect("a slot").generation(), 1);
    }
}
