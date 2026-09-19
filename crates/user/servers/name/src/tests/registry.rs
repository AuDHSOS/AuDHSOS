// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::registry`.

use audhsos_abi::{Error, Handle, Rights};
use user_proto::Name;

use crate::registry::{CAPACITY, HANDED_OUT, Handles, Registry};

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
    for (owner, text, index) in [
        (1, &b"console"[..], 10),
        (2, b"memory", 20),
        (1, b"log", 30),
    ] {
        let replaced = registry.register(owner, name(text), handle(index)).unwrap();
        assert_eq!(replaced, None);
    }
    registry
}

/// The first handle the fake table hands out, kept away from the handles
/// the tests send so that a stored handle names which of the two it is.
const FIRST_NARROWED: u32 = 100;

/// A handle table that records what the registry asked of it.
#[derive(Default)]
struct Table {
    /// Every duplication, with the rights it was asked for.
    duplicated: Vec<(Handle, Rights)>,
    /// Every handle that was given up, in order.
    closed: Vec<Handle>,
    /// How many handles the table handed out.
    handed_out: u32,
    /// Answers every duplication with [`Error::AccessDenied`].
    refuses: bool,
}

impl Handles for Table {
    fn duplicate(&mut self, from: Handle, rights: Rights) -> Result<Handle, Error> {
        self.duplicated.push((from, rights));
        if self.refuses {
            return Err(Error::AccessDenied);
        }
        self.handed_out = self.handed_out.wrapping_add(1);
        Ok(handle(FIRST_NARROWED.wrapping_add(self.handed_out)))
    }

    fn close(&mut self, given_up: Handle) {
        self.closed.push(given_up);
    }
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
    let replaced = registry.register(1, name(b"console"), handle(11)).unwrap();
    assert_eq!(replaced, Some(handle(10)), "the handle the entry held");
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
        let _fresh = registry.register(1, name(&text), handle(raw)).unwrap();
    }
    assert_eq!(registry.len(), CAPACITY);
    assert_eq!(
        registry
            .register(1, name(b"one more"), handle(1))
            .unwrap_err(),
        Error::PoolExhausted
    );
    // A replacement still works: it needs no slot.
    let replaced = registry
        .register(1, name(b"n\x01\x00"), handle(77))
        .unwrap();
    assert_eq!(replaced, Some(handle(1)));
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
    let mut table = Table::default();
    assert_eq!(registry.forget_client(&mut table, 1), 2);
    assert_eq!(
        table.closed,
        vec![handle(30), handle(10)],
        "the handle of every entry that went, from the back"
    );
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
    let mut table = Table::default();
    assert_eq!(registry.forget_client(&mut table, 9), 0);
    assert_eq!(registry.len(), 3);
    assert!(table.closed.is_empty());
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

#[test]
fn what_a_lookup_hands_out_carries_no_recv_and_no_badge() {
    assert_eq!(HANDED_OUT, Rights::SEND | Rights::TRANSFER);
    assert!(
        !HANDED_OUT.contains(Rights::RECV),
        "a client could take the requests other clients sent to the server"
    );
    assert!(
        !HANDED_OUT.contains(Rights::BADGE),
        "a client could mint the badge the server trusts"
    );
    assert!(!HANDED_OUT.contains(Rights::DUPLICATE));
}

#[test]
fn a_registration_stores_a_handle_narrowed_to_what_a_lookup_may_hand_out() {
    let mut registry = Registry::new();
    let mut table = Table::default();
    registry
        .accept(&mut table, 1, name(b"console"), handle(10))
        .unwrap();
    assert_eq!(
        table.duplicated,
        vec![(handle(10), HANDED_OUT)],
        "the handle the client sent is narrowed once"
    );
    let stored = registry.lookup(&name(b"console")).unwrap();
    assert_eq!(stored, handle(FIRST_NARROWED.wrapping_add(1)));
    assert_ne!(
        stored,
        handle(10),
        "the handle the client sent carries the RECV and BADGE of the server's endpoint"
    );
    assert_eq!(
        table.closed,
        vec![handle(10)],
        "the sent handle is given up"
    );
}

#[test]
fn a_replaced_registration_gives_the_handle_it_held_up() {
    let mut registry = Registry::new();
    let mut table = Table::default();
    registry
        .accept(&mut table, 1, name(b"console"), handle(10))
        .unwrap();
    registry
        .accept(&mut table, 1, name(b"console"), handle(11))
        .unwrap();
    assert_eq!(
        registry.lookup(&name(b"console")).unwrap(),
        handle(FIRST_NARROWED.wrapping_add(2))
    );
    assert_eq!(
        table.closed,
        vec![
            handle(10),
            handle(11),
            handle(FIRST_NARROWED.wrapping_add(1))
        ],
        "both sent handles and the one the entry held"
    );
}

#[test]
fn a_refused_registration_gives_every_handle_up() {
    let mut registry = Registry::new();
    let mut table = Table::default();
    registry
        .accept(&mut table, 1, name(b"console"), handle(10))
        .unwrap();
    table.closed.clear();
    assert_eq!(
        registry
            .accept(&mut table, 9, name(b"console"), handle(20))
            .unwrap_err(),
        Error::AlreadyExists
    );
    assert_eq!(
        registry.lookup(&name(b"console")).unwrap(),
        handle(FIRST_NARROWED.wrapping_add(1)),
        "the refused registration changed nothing"
    );
    assert_eq!(
        table.closed,
        vec![handle(20), handle(FIRST_NARROWED.wrapping_add(2))],
        "the sent handle and the narrowed one that was never stored"
    );
}

#[test]
fn a_registration_whose_narrowing_is_refused_gives_the_sent_handle_up() {
    let mut registry = Registry::new();
    let mut table = Table {
        refuses: true,
        ..Table::default()
    };
    assert_eq!(
        registry
            .accept(&mut table, 1, name(b"console"), handle(10))
            .unwrap_err(),
        Error::AccessDenied
    );
    assert!(registry.is_empty());
    assert_eq!(table.closed, vec![handle(10)]);
}

#[test]
fn a_registration_without_a_badge_stores_nothing_and_gives_every_handle_up() {
    let mut registry = Registry::new();
    let mut table = Table::default();
    assert_eq!(
        registry
            .accept(&mut table, 0, name(b"console"), handle(10))
            .unwrap_err(),
        Error::InvalidArgument
    );
    assert!(registry.is_empty());
    assert_eq!(
        table.closed,
        vec![handle(10), handle(FIRST_NARROWED.wrapping_add(1))]
    );
}
