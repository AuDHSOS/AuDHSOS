// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::transfer`.

use audhsos_abi::ipc_buffer::{Buffer, BufferMut, SIZE};
use audhsos_abi::layout::{MAX_MESSAGE_HANDLES, MAX_MESSAGE_WORDS};
use audhsos_abi::{Error, Handle, ObjectType, Rights};
use kernel_objects::object::AnyObjectId;

use super::fixture::{Fixture, message};
use crate::transfer::{Transferred, transfer};

/// The rights a handle needs to travel.
const TRAVELS: Rights = Rights::READ.union(Rights::TRANSFER);

#[test]
fn a_message_of_no_words_carries_its_label_and_nothing_else() {
    let mut fixture = Fixture::new();
    let sender = fixture.process(8);
    let receiver = fixture.process(8);
    let from = message(7, 0, &[]);
    let mut to = [0; SIZE];
    let moved = transfer(&from, &mut to, &mut fixture.objects, sender, receiver).unwrap();
    assert_eq!(
        moved,
        Transferred {
            words: 0,
            handles: 0,
            truncated: false
        }
    );
    let header = Buffer::new(&to).message().unwrap();
    assert_eq!(header.label, 7);
    assert_eq!(header.word_count, 0);
    assert_eq!(header.handle_count, 0);
}

#[test]
fn a_message_of_the_widest_payload_arrives_word_for_word() {
    let mut fixture = Fixture::new();
    let sender = fixture.process(8);
    let receiver = fixture.process(8);
    let from = message(1, MAX_MESSAGE_WORDS, &[]);
    let mut to = [0; SIZE];
    let moved = transfer(&from, &mut to, &mut fixture.objects, sender, receiver).unwrap();
    assert_eq!(moved.words, u16::try_from(MAX_MESSAGE_WORDS).unwrap());
    let view = Buffer::new(&to);
    assert_eq!(view.message().unwrap().word_count, MAX_MESSAGE_WORDS);
    for index in 0..MAX_MESSAGE_WORDS {
        assert_eq!(
            view.word(index),
            Some(u64::try_from(index).unwrap().saturating_add(1)),
            "word {index}"
        );
    }
}

#[test]
fn a_word_count_above_the_area_is_refused_before_anything_is_copied() {
    let mut fixture = Fixture::new();
    let sender = fixture.process(8);
    let receiver = fixture.process(8);
    let mut from = [0; SIZE];
    {
        let mut writer = BufferMut::new(&mut from);
        writer.set_label(9);
        // The setter refuses the count, so the raw word is written by hand:
        // this is what a thread that wrote its own header looks like.
        writer.set_argument(0, 0);
    }
    let raw = u64::try_from(MAX_MESSAGE_WORDS).unwrap().saturating_add(1);
    write_raw(&mut from, audhsos_abi::ipc_buffer::WORD_COUNT, raw);
    let mut to = [0; SIZE];
    assert_eq!(
        transfer(&from, &mut to, &mut fixture.objects, sender, receiver),
        Err(Error::InvalidArgument)
    );
    assert_eq!(to, [0; SIZE], "not a byte of the receiver's buffer changed");
}

#[test]
fn a_handle_count_above_the_area_is_refused() {
    let mut fixture = Fixture::new();
    let sender = fixture.process(8);
    let receiver = fixture.process(8);
    let mut from = message(1, 2, &[]);
    let raw = u64::try_from(MAX_MESSAGE_HANDLES)
        .unwrap()
        .saturating_add(1);
    write_raw(&mut from, audhsos_abi::ipc_buffer::HANDLE_COUNT, raw);
    let mut to = [0; SIZE];
    assert_eq!(
        transfer(&from, &mut to, &mut fixture.objects, sender, receiver),
        Err(Error::InvalidArgument)
    );
    assert_eq!(to, [0; SIZE]);
}

#[test]
fn four_handles_travel_with_their_rights_their_badge_and_a_reference() {
    let mut fixture = Fixture::new();
    let sender = fixture.process(8);
    let receiver = fixture.process(8);
    let mut sent = Vec::new();
    let mut objects = Vec::new();
    for _ in 0..MAX_MESSAGE_HANDLES {
        let memory = fixture.memory();
        objects.push(memory);
        sent.push(fixture.install(sender, AnyObjectId::of(memory), TRAVELS));
    }
    let from = message(3, 1, &sent);
    let mut to = [0; SIZE];
    let moved = transfer(&from, &mut to, &mut fixture.objects, sender, receiver).unwrap();
    assert_eq!(moved.handles, 4);
    assert!(!moved.truncated);
    let view = Buffer::new(&to);
    assert_eq!(view.message().unwrap().handle_count, 4);
    for (index, object) in objects.iter().enumerate() {
        let handle = view.handle(index).unwrap();
        let entry = fixture.objects.handles.lookup(receiver, handle).unwrap();
        assert_eq!(entry.object, AnyObjectId::of(*object));
        assert_eq!(entry.rights, TRAVELS, "the same rights, never more");
        assert_eq!(
            fixture.objects.memory.references(*object),
            Ok(2),
            "the sender keeps its handle and the receiver holds a reference"
        );
    }
}

#[test]
fn a_badged_handle_keeps_its_badge_across_the_transfer() {
    let mut fixture = Fixture::new();
    let sender = fixture.process(8);
    let receiver = fixture.process(8);
    let endpoint = fixture.endpoint();
    let handle = fixture.install(
        sender,
        AnyObjectId::of(endpoint),
        Rights::SEND | Rights::TRANSFER,
    );
    fixture
        .objects
        .handles
        .lookup_mut(sender, handle)
        .unwrap()
        .badge = 0x5EED;
    let from = message(1, 0, &[handle]);
    let mut to = [0; SIZE];
    transfer(&from, &mut to, &mut fixture.objects, sender, receiver).unwrap();
    let arrived = Buffer::new(&to).handle(0).unwrap();
    let entry = fixture.objects.handles.lookup(receiver, arrived).unwrap();
    assert_eq!(entry.badge, 0x5EED);
    assert_eq!(entry.object_type(), ObjectType::Endpoint);
}

#[test]
fn a_handle_without_transfer_refuses_the_message_before_any_is_installed() {
    let mut fixture = Fixture::new();
    let sender = fixture.process(8);
    let receiver = fixture.process(8);
    let first = fixture.memory();
    let second = fixture.memory();
    let travels = fixture.install(sender, AnyObjectId::of(first), TRAVELS);
    let stays = fixture.install(sender, AnyObjectId::of(second), Rights::READ);
    let from = message(1, 1, &[travels, stays]);
    let mut to = [0; SIZE];
    assert_eq!(
        transfer(&from, &mut to, &mut fixture.objects, sender, receiver),
        Err(Error::AccessDenied)
    );
    assert_eq!(fixture.handle_count(receiver), 0, "not one arrived");
    assert_eq!(to, [0; SIZE], "and not a word was copied");
    assert_eq!(fixture.objects.memory.references(first), Ok(1));
}

#[test]
fn a_handle_the_sender_does_not_hold_refuses_the_message() {
    let mut fixture = Fixture::new();
    let sender = fixture.process(8);
    let receiver = fixture.process(8);
    let nothing = Handle::new(0x00FF_FFFF, 1).unwrap();
    let from = message(1, 1, &[nothing]);
    let mut to = [0; SIZE];
    assert_eq!(
        transfer(&from, &mut to, &mut fixture.objects, sender, receiver),
        Err(Error::InvalidHandle)
    );
    assert_eq!(to, [0; SIZE]);
}

#[test]
fn a_handle_word_that_names_no_handle_refuses_the_message() {
    let mut fixture = Fixture::new();
    let sender = fixture.process(8);
    let receiver = fixture.process(8);
    let mut from = message(1, 1, &[]);
    write_raw(&mut from, audhsos_abi::ipc_buffer::HANDLE_COUNT, 1);
    let mut to = [0; SIZE];
    assert_eq!(
        transfer(&from, &mut to, &mut fixture.objects, sender, receiver),
        Err(Error::InvalidHandle),
        "a zero word is no handle"
    );
}

#[test]
fn a_receiver_with_room_for_fewer_handles_gets_what_fits_and_the_flag() {
    let mut fixture = Fixture::new();
    let sender = fixture.process(8);
    let receiver = fixture.process(3);
    let filler = fixture.memory();
    // Two slots of three are taken, so one handle of two fits.
    fixture.install(receiver, AnyObjectId::of(filler), Rights::EMPTY);
    fixture.install(receiver, AnyObjectId::of(filler), Rights::EMPTY);
    let first = fixture.memory();
    let second = fixture.memory();
    let sent = [
        fixture.install(sender, AnyObjectId::of(first), TRAVELS),
        fixture.install(sender, AnyObjectId::of(second), TRAVELS),
    ];
    let from = message(1, 2, &sent);
    let mut to = [0; SIZE];
    let moved = transfer(&from, &mut to, &mut fixture.objects, sender, receiver).unwrap();
    assert_eq!(moved.handles, 1);
    assert!(moved.truncated, "the message is delivered all the same");
    let view = Buffer::new(&to);
    assert_eq!(view.message().unwrap().handle_count, 1);
    assert_eq!(view.word(1), Some(2), "the words arrived in full");
    assert_eq!(fixture.objects.memory.references(second), Ok(1));
}

#[test]
fn a_message_between_two_threads_of_one_process_moves_all_the_same() {
    let mut fixture = Fixture::new();
    let both = fixture.process(8);
    let memory = fixture.memory();
    let handle = fixture.install(both, AnyObjectId::of(memory), TRAVELS);
    let from = message(4, 3, &[handle]);
    let mut to = [0; SIZE];
    let moved = transfer(&from, &mut to, &mut fixture.objects, both, both).unwrap();
    assert_eq!(moved.handles, 1);
    assert_eq!(
        fixture.handle_count(both),
        2,
        "the second handle is in the same list"
    );
    let arrived = Buffer::new(&to).handle(0).unwrap();
    assert_ne!(arrived, handle);
    assert_eq!(fixture.objects.memory.references(memory), Ok(2));
}

/// Writes a raw word at `offset`, for the headers a setter refuses.
fn write_raw(bytes: &mut [u8; SIZE], offset: usize, value: u64) {
    let end = offset.saturating_add(8);
    bytes
        .get_mut(offset..end)
        .unwrap()
        .copy_from_slice(&value.to_le_bytes());
}
