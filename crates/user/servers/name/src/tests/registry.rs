// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::registry`.

use audhsos_abi::{Error, Handle};
use user_proto::Name;

use crate::registry::{CAPACITY, Registry};

/// A handle every table could hand out.
fn handle(index: u32) -> Handle {
    Handle::new(index, 1).unwrap()
}

/// The name `text`.
fn name(text: &[u8]) -> Name {
    Name::new(text).unwrap()
}

/// A registry holding the three servers of the boot sequence.
fn filled() -> Registry {
    let mut registry = Registry::new();
    registry.register(1, name(b"console"), handle(10)).unwrap();
    registry.register(2, name(b"memory"), handle(20)).unwrap();
    registry.register(1, name(b"log"), handle(30)).unwrap();
    registry
}

#[test]
fn an_empty_registry_holds_nothing() {
    let registry = Registry::new();
    assert!(registry.is_empty());
    assert_eq!(registry.len(), 0);
    assert_eq!(registry.entries().count(), 0);
    assert_eq!(Registry::default().len(), 0);
}

#[test]
fn a_registered_name_is_found() {
    let registry = filled();
    assert_eq!(registry.lookup(&name(b"console")).unwrap(), handle(10));
    assert_eq!(registry.lookup(&name(b"memory")).unwrap(), handle(20));
    assert_eq!(registry.len(), 3);
}

#[test]
fn a_name_that_was_never_registered_is_not_found() {
    let registry = filled();
    assert_eq!(
        registry.lookup(&name(b"display")).unwrap_err(),
        Error::NotFound
    );
    assert_eq!(
        registry.lookup(&Name::empty()).unwrap_err(),
        Error::NotFound
    );
}

#[test]
fn a_lookup_changes_nothing() {
    let registry = filled();
    let before: Vec<Handle> = registry.entries().map(|entry| entry.endpoint).collect();
    let _found = registry.lookup(&name(b"console"));
    let _missing = registry.lookup(&name(b"nothing"));
    let after: Vec<Handle> = registry.entries().map(|entry| entry.endpoint).collect();
    assert_eq!(before, after);
}

#[test]
fn a_client_may_replace_what_it_registered_itself() {
    let mut registry = filled();
    registry.register(1, name(b"console"), handle(11)).unwrap();
    assert_eq!(registry.lookup(&name(b"console")).unwrap(), handle(11));
    assert_eq!(registry.len(), 3, "a replacement adds no entry");
}

#[test]
fn a_name_another_client_holds_is_refused() {
    let mut registry = filled();
    assert_eq!(
        registry
            .register(9, name(b"console"), handle(99))
            .unwrap_err(),
        Error::AlreadyExists
    );
    assert_eq!(
        registry.lookup(&name(b"console")).unwrap(),
        handle(10),
        "the refused registration changed nothing"
    );
}

#[test]
fn a_message_without_a_badge_registers_nothing() {
    let mut registry = Registry::new();
    assert_eq!(
        registry
            .register(0, name(b"console"), handle(1))
            .unwrap_err(),
        Error::InvalidArgument
    );
    assert!(registry.is_empty());
}

#[test]
fn a_full_registry_takes_no_further_name() {
    let mut registry = Registry::new();
    for index in 0..CAPACITY {
        let raw = u32::try_from(index).unwrap().wrapping_add(1);
        let text = [b'n', raw.to_le_bytes()[0], raw.to_le_bytes()[1]];
        registry.register(1, name(&text), handle(raw)).unwrap();
    }
    assert_eq!(registry.len(), CAPACITY);
    assert_eq!(
        registry
            .register(1, name(b"one more"), handle(1))
            .unwrap_err(),
        Error::PoolExhausted
    );
    // A replacement still works: it needs no slot.
    registry
        .register(1, name(b"n\x01\x00"), handle(77))
        .unwrap();
    assert_eq!(registry.lookup(&name(b"n\x01\x00")).unwrap(), handle(77));
}

#[test]
fn a_client_may_take_back_what_it_registered() {
    let mut registry = filled();
    assert_eq!(registry.forget(1, &name(b"console")).unwrap(), handle(10));
    assert_eq!(
        registry.lookup(&name(b"console")).unwrap_err(),
        Error::NotFound
    );
    assert_eq!(registry.len(), 2);
}

#[test]
fn a_client_may_not_take_back_what_another_registered() {
    let mut registry = filled();
    assert_eq!(
        registry.forget(9, &name(b"console")).unwrap_err(),
        Error::AccessDenied
    );
    assert_eq!(registry.len(), 3);
}

#[test]
fn taking_back_a_name_that_is_not_there_is_refused() {
    let mut registry = filled();
    assert_eq!(
        registry.forget(1, &name(b"display")).unwrap_err(),
        Error::NotFound
    );
}

#[test]
fn everything_a_dead_client_registered_goes_with_it() {
    let mut registry = filled();
    assert_eq!(registry.forget_client(1), 2);
    assert_eq!(registry.len(), 1);
    assert_eq!(registry.lookup(&name(b"memory")).unwrap(), handle(20));
    assert_eq!(
        registry.lookup(&name(b"console")).unwrap_err(),
        Error::NotFound
    );
    assert_eq!(registry.lookup(&name(b"log")).unwrap_err(), Error::NotFound);
}

#[test]
fn a_client_that_registered_nothing_leaves_nothing_behind() {
    let mut registry = filled();
    assert_eq!(registry.forget_client(9), 0);
    assert_eq!(registry.len(), 3);
}

#[test]
fn the_entries_come_out_in_the_order_they_went_in() {
    let registry = filled();
    let names: Vec<Vec<u8>> = registry
        .entries()
        .map(|entry| entry.name.as_bytes().to_vec())
        .collect();
    assert_eq!(
        names,
        vec![b"console".to_vec(), b"memory".to_vec(), b"log".to_vec()]
    );
    let owners: Vec<u64> = registry.entries().map(|entry| entry.owner).collect();
    assert_eq!(owners, vec![1, 2, 1]);
}
