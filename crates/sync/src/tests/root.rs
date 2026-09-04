// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate`.

use crate::{Error, Global, UncontendedToken};
use std::cell::Cell;
use std::rc::Rc;

const TOKEN: UncontendedToken = UncontendedToken;

#[test]
fn first_borrow_succeeds_second_fails_third_after_release_succeeds() {
    let cell = Global::new();
    cell.init(7u32).unwrap();
    let first = cell.borrow(&TOKEN).unwrap();
    assert_eq!(*first, 7);
    assert!(cell.is_borrowed());
    assert!(matches!(cell.borrow(&TOKEN), Err(Error::AlreadyBorrowed)));
    drop(first);
    assert!(!cell.is_borrowed());
    let mut third = cell.borrow(&TOKEN).unwrap();
    *third += 1;
    drop(third);
    assert_eq!(*cell.borrow(&TOKEN).unwrap(), 8);
}

#[test]
fn init_succeeds_exactly_once() {
    let cell = Global::new();
    assert_eq!(cell.init(1u8), Ok(()));
    assert_eq!(cell.init(2u8), Err(Error::AlreadyInitialized));
    assert_eq!(*cell.borrow(&TOKEN).unwrap(), 1);
}

#[test]
fn borrow_before_init_fails_and_leaves_the_cell_unborrowed() {
    let cell: Global<u8> = Global::new();
    assert!(matches!(cell.borrow(&TOKEN), Err(Error::Uninitialized)));
    assert!(!cell.is_borrowed());
    assert_eq!(cell.init(3), Ok(()));
}

#[test]
fn init_while_borrowed_fails_without_touching_the_value() {
    let cell = Global::new();
    cell.init(5u8).unwrap();
    let guard = cell.borrow(&TOKEN).unwrap();
    assert_eq!(cell.init(9), Err(Error::AlreadyBorrowed));
    drop(guard);
    assert_eq!(*cell.borrow(&TOKEN).unwrap(), 5);
}

#[test]
fn value_with_a_destructor_is_dropped_with_the_cell_and_not_before() {
    struct Tracked(Rc<Cell<u32>>);
    impl Drop for Tracked {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    let drops = Rc::new(Cell::new(0u32));
    let cell = Global::new();
    cell.init(Tracked(Rc::clone(&drops))).unwrap();
    {
        let guard = cell.borrow(&TOKEN).unwrap();
        assert_eq!(guard.0.get(), 0);
    }
    assert_eq!(drops.get(), 0);
    drop(cell);
    assert_eq!(drops.get(), 1);
}

#[test]
fn cell_is_usable_as_a_static() {
    static CELL: Global<u64> = Global::new();
    CELL.init(42).unwrap();
    assert_eq!(*CELL.borrow(&TOKEN).unwrap(), 42);
}

#[test]
fn errors_have_messages() {
    for error in [
        Error::AlreadyInitialized,
        Error::Uninitialized,
        Error::AlreadyBorrowed,
    ] {
        assert!(!error.to_string().is_empty());
    }
}

#[test]
fn debug_output_names_the_cell_and_the_value() {
    let cell = Global::new();
    cell.init(1u8).unwrap();
    assert!(format!("{cell:?}").starts_with("Global"));
    assert_eq!(format!("{:?}", cell.borrow(&TOKEN).unwrap()), "1");
}
