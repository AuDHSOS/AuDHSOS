// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of the `Preset` cell, covering the cell item of the catalog
//! 6.6.18.

use crate::{Error, Preset, UncontendedToken};
use std::cell::Cell;
use std::rc::Rc;

const TOKEN: UncontendedToken = UncontendedToken;

#[test]
fn the_value_is_there_without_an_initialization_step() {
    let cell = Preset::new(42u64);
    assert!(!cell.is_borrowed());
    assert_eq!(*cell.borrow(&TOKEN).unwrap(), 42);
}

#[test]
fn first_borrow_succeeds_second_fails_third_after_release_succeeds() {
    let cell = Preset::new(1u32);
    let first = cell.borrow(&TOKEN).unwrap();
    assert!(cell.is_borrowed());
    assert_eq!(cell.borrow(&TOKEN).map(|_| ()), Err(Error::AlreadyBorrowed));
    drop(first);
    assert!(!cell.is_borrowed());
    assert_eq!(*cell.borrow(&TOKEN).unwrap(), 1);
}

#[test]
fn a_write_through_the_borrow_is_what_the_next_borrow_sees() {
    let cell = Preset::new([0u8; 4]);
    {
        let mut borrow = cell.borrow(&TOKEN).unwrap();
        borrow[2] = 9;
    }
    assert_eq!(*cell.borrow(&TOKEN).unwrap(), [0, 0, 9, 0]);
}

#[test]
fn a_cell_the_caller_owns_needs_no_token() {
    let mut cell = Preset::new(5u8);
    *cell.get_mut() = 7;
    assert_eq!(*cell.borrow(&TOKEN).unwrap(), 7);
}

#[test]
fn a_const_cell_is_usable_as_a_static() {
    // This is what the kernel does with its object pools: the value is a
    // constant of zeros, so the `static` is `.bss` and nothing is ever
    // moved into it (D-66).
    static CELL: Preset<[u64; 8]> = Preset::new([0; 8]);
    assert_eq!(CELL.borrow(&TOKEN).unwrap().len(), 8);
    {
        let mut borrow = CELL.borrow(&TOKEN).unwrap();
        borrow[0] = 3;
    }
    assert_eq!(CELL.borrow(&TOKEN).unwrap()[0], 3);
    // Leave the static as the next run of this test would expect it.
    CELL.borrow(&TOKEN).unwrap()[0] = 0;
}

#[test]
fn the_default_cell_holds_the_default_value() {
    let cell: Preset<u16> = Preset::default();
    assert_eq!(*cell.borrow(&TOKEN).unwrap(), 0);
}

#[test]
fn a_value_with_a_destructor_is_dropped_with_the_cell_and_not_before() {
    struct Guard(Rc<Cell<bool>>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }

    let dropped = Rc::new(Cell::new(false));
    let cell = Preset::new(Guard(Rc::clone(&dropped)));
    {
        let borrow = cell.borrow(&TOKEN).unwrap();
        assert!(!borrow.0.get());
    }
    assert!(!dropped.get(), "a released borrow drops no value");
    drop(cell);
    assert!(dropped.get());
}

#[test]
fn debug_output_names_the_cell_and_the_value() {
    let cell = Preset::new(1u8);
    assert!(format!("{cell:?}").starts_with("Preset"));
    assert_eq!(format!("{:?}", cell.borrow(&TOKEN).unwrap()), "1");
}
