// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::handles`.

use audhsos_abi::ipc_buffer::SIZE;
use audhsos_abi::layout::MAX_MESSAGE_HANDLES;
use audhsos_abi::{Buffer, BufferMut, Handle};

use crate::handles::Carried;

/// The handles a client may attach to one message.
fn handles() -> Vec<Handle> {
    (1..=u32::try_from(MAX_MESSAGE_HANDLES).unwrap())
        .map(|index| Handle::new(index, 1).unwrap())
        .collect()
}

/// A buffer holding a message with `count` handles attached.
fn message(count: usize) -> [u8; SIZE] {
    let mut bytes = [0; SIZE];
    let mut writer = BufferMut::new(&mut bytes);
    writer.set_counts(0, count).unwrap();
    for (index, handle) in handles().iter().enumerate().take(count) {
        writer.set_handle(index, *handle);
    }
    bytes
}

#[test]
fn every_handle_of_the_message_is_read_before_the_buffer_is_overwritten() {
    for count in 0..=MAX_MESSAGE_HANDLES {
        let mut bytes = message(count);
        let carried = Carried::read(Buffer::new(&bytes));
        bytes.fill(0);
        assert_eq!(carried.count(), count);
        assert_eq!(carried.handles().collect::<Vec<_>>(), handles()[..count]);
    }
}

#[test]
fn a_request_that_keeps_nothing_gives_every_handle_up() {
    for count in 0..=MAX_MESSAGE_HANDLES {
        let bytes = message(count);
        let mut closed = Vec::new();
        Carried::read(Buffer::new(&bytes)).give_up(&[], |handle| closed.push(handle));
        assert_eq!(closed, handles()[..count]);
    }
}

#[test]
fn a_request_that_keeps_one_handle_gives_the_rest_up() {
    let bytes = message(MAX_MESSAGE_HANDLES);
    let kept = handles()[1];
    let mut closed = Vec::new();
    Carried::read(Buffer::new(&bytes)).give_up(&[kept], |handle| closed.push(handle));
    assert!(!closed.contains(&kept));
    assert_eq!(closed.len(), MAX_MESSAGE_HANDLES - 1);
}

#[test]
fn a_buffer_that_holds_no_message_carries_no_handles() {
    let bytes = [0xff; SIZE];
    let carried = Carried::read(Buffer::new(&bytes));
    assert_eq!(carried.count(), 0);
}
