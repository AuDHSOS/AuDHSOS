// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The region the stack writes into, and the pieces handed out of it.

use crate::memory::{CONNECTIONS, DATAGRAMS, Pool, REGION_BYTES, SOCKETS, WINDOW};

#[test]
fn a_region_shorter_than_the_pieces_is_refused() {
    let mut bytes = vec![0u8; REGION_BYTES - 1];
    assert!(Pool::split(&mut bytes).is_none());
}

#[test]
fn the_pieces_are_the_lengths_the_constants_name() {
    let mut bytes = vec![0u8; REGION_BYTES];
    let (outgoing, mut pool) = Pool::split(&mut bytes).expect("the region is long enough");
    assert!(outgoing.len() >= net_stack::FRAME_LEN);
    let (send, receive) = pool.take_window().expect("a window");
    assert_eq!(send.len(), WINDOW);
    assert_eq!(receive.len(), WINDOW);
    let buffer = pool.take_datagram().expect("a buffer");
    assert_eq!(buffer.len(), DATAGRAMS);
}

#[test]
fn the_pool_holds_as_many_of_each_as_it_says_and_no_more() {
    let mut bytes = vec![0u8; REGION_BYTES];
    let (_outgoing, mut pool) = Pool::split(&mut bytes).expect("the region is long enough");
    let mut windows = Vec::new();
    for _ in 0..CONNECTIONS {
        windows.push(pool.take_window().expect("a window"));
    }
    assert!(pool.take_window().is_none());
    let mut buffers = Vec::new();
    for _ in 0..SOCKETS {
        buffers.push(pool.take_datagram().expect("a buffer"));
    }
    assert!(pool.take_datagram().is_none());
    // What went out comes back, and the pool hands it out again.
    for window in windows {
        assert!(pool.put_window(window));
    }
    for buffer in buffers {
        assert!(pool.put_datagram(buffer));
    }
    assert!(pool.take_window().is_some());
    assert!(pool.take_datagram().is_some());
}

#[test]
fn a_piece_given_back_to_a_full_pool_is_let_go_rather_than_doubling_a_slot() {
    let mut bytes = vec![0u8; REGION_BYTES];
    let (_outgoing, mut pool) = Pool::split(&mut bytes).expect("the region is long enough");
    // Nothing was taken, so the pool is full and what is offered to it
    // belongs to nobody: it is let go.
    let send: &'static mut [u8] = Box::leak(vec![0u8; WINDOW].into_boxed_slice());
    let receive: &'static mut [u8] = Box::leak(vec![0u8; WINDOW].into_boxed_slice());
    assert!(!pool.put_window((send, receive)));
    let buffer: &'static mut [u8] = Box::leak(vec![0u8; DATAGRAMS].into_boxed_slice());
    assert!(!pool.put_datagram(buffer));
    // One that was taken goes back into the slot it came out of.
    let window = pool.take_window().expect("a window");
    assert!(pool.put_window(window));
    let taken = pool.take_datagram().expect("a buffer");
    assert!(pool.put_datagram(taken));
}
