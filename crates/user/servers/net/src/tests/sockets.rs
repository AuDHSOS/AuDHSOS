// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The socket table: whose a number is, and what a number of a socket that
//! was closed names.

use net_stack::{Handle, Slots};

use crate::sockets::{Kind, MAX_SOCKETS, Sockets};

/// A handle of the stack, for a table that only carries it.
fn handle(index: usize) -> Handle {
    let slots = Slots::<8>::new();
    slots.handle(index).expect("a handle")
}

#[test]
fn a_number_is_answered_for_the_client_it_was_given_to() {
    let mut sockets = Sockets::new();
    let (number, index) = sockets
        .open(7, Kind::Connection, handle(0))
        .expect("a slot was free");
    assert_eq!(sockets.slot(7, number), Some(index));
    assert!(sockets.get(7, number).is_some());
}

#[test]
fn a_number_of_another_client_names_nothing() {
    let mut sockets = Sockets::new();
    let (number, _) = sockets
        .open(7, Kind::Connection, handle(0))
        .expect("a slot was free");
    assert_eq!(sockets.slot(8, number), None);
}

#[test]
fn a_number_of_a_socket_that_was_closed_names_nothing() {
    let mut sockets = Sockets::new();
    let (first, index) = sockets
        .open(7, Kind::Connection, handle(0))
        .expect("a slot was free");
    let _closed = sockets.close(index);
    let (second, again) = sockets
        .open(7, Kind::Datagram, handle(1))
        .expect("the slot is free again");
    assert_eq!(index, again, "the slot was not reused");
    assert_ne!(first, second, "the number did not carry the generation");
    assert_eq!(sockets.slot(7, first), None);
    assert_eq!(sockets.slot(7, second), Some(index));
}

#[test]
fn a_number_that_names_no_slot_is_refused() {
    let sockets = Sockets::new();
    assert_eq!(sockets.slot(7, 0), None);
    assert_eq!(sockets.slot(7, u32::MAX), None);
}

#[test]
fn the_table_holds_as_many_sockets_as_it_says_and_no_more() {
    let mut sockets = Sockets::new();
    for index in 0..MAX_SOCKETS {
        assert!(
            sockets
                .open(
                    u64::try_from(index).unwrap_or(0),
                    Kind::Connection,
                    handle(index)
                )
                .is_some()
        );
    }
    assert_eq!(sockets.len(), MAX_SOCKETS);
    assert!(sockets.open(99, Kind::Connection, handle(0)).is_none());
}

#[test]
fn the_slots_of_one_client_are_the_ones_it_opened() {
    let mut sockets = Sockets::new();
    let _mine = sockets.open(7, Kind::Connection, handle(0));
    let _theirs = sockets.open(8, Kind::Connection, handle(1));
    let _mine_again = sockets.open(7, Kind::Datagram, handle(2));
    let of_seven: Vec<usize> = sockets.of(7).collect();
    assert_eq!(of_seven.len(), 2);
    assert_eq!(sockets.open_slots().count(), 3);
}

#[test]
fn a_table_that_was_never_used_is_empty() {
    assert!(Sockets::new().is_empty());
}

#[test]
fn a_slot_that_has_been_used_sixty_five_thousand_times_still_answers() {
    let mut sockets = Sockets::new();
    // The count a number carries is sixteen bits wide; the count the slot
    // holds has to stay inside them or the two stop agreeing.
    for _round in 0..=0x1_0000u32 {
        let (number, index) = sockets
            .open(7, Kind::Connection, handle(0))
            .expect("a slot was free");
        assert_eq!(sockets.slot(7, number), Some(index), "the number was lost");
        let _closed = sockets.close(index).expect("the slot was open");
    }
}

#[test]
fn a_slot_one_client_let_go_of_is_not_handed_to_another() {
    let mut sockets = Sockets::new();
    let mut held = [0usize; MAX_SOCKETS];
    for slot in &mut held {
        let (_number, index) = sockets
            .open(7, Kind::Connection, handle(0))
            .expect("a slot was free");
        *slot = index;
    }
    for index in held {
        let _closed = sockets.close(index).expect("the slot was open");
    }
    assert!(
        sockets.open(8, Kind::Connection, handle(0)).is_none(),
        "the rings of one client were handed to another"
    );
    assert!(
        sockets.open(7, Kind::Connection, handle(0)).is_some(),
        "the client that held the slot was refused its own"
    );
}

#[test]
fn a_slot_is_another_clients_once_the_one_that_held_it_is_gone() {
    let mut sockets = Sockets::new();
    let (_number, index) = sockets
        .open(7, Kind::Connection, handle(0))
        .expect("a slot was free");
    let _closed = sockets.close(index).expect("the slot was open");
    sockets.release(7);
    let (_taken, again) = sockets
        .open(8, Kind::Connection, handle(0))
        .expect("the slot was let go of");
    assert_eq!(again, index);
}
