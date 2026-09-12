// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Regression tests for received handle ownership on every reply path.

use crate::request::ReceivedHandles;
use audhsos_abi::{Buffer, BufferMut, Error, Handle};
use user_proto::input::{Reply, Request};

#[test]
fn rejected_and_extra_handles_are_closed_even_if_nested_ipc_overwrites_the_buffer() {
    let handles: Vec<_> = (1..=4)
        .map(|index| Handle::new(index, 1).unwrap())
        .collect();
    for request in [
        Request::Unsubscribe,
        Request::Subscribe {
            notification: handles[0],
            process: handles[1],
        },
    ] {
        for count in 0..=4 {
            for failed in [false, true] {
                let mut bytes = [0; audhsos_abi::ipc_buffer::SIZE];
                let mut writer = BufferMut::new(&mut bytes);
                request.encode(&mut writer).unwrap();
                writer.set_counts(0, count).unwrap();
                for (index, handle) in handles.iter().enumerate().take(count) {
                    writer.set_handle(index, *handle);
                }
                let snapshot = ReceivedHandles::read(Buffer::new(&bytes));
                let decoded = Request::decode(Buffer::new(&bytes)).ok();
                let reply = match (decoded, failed) {
                    (Some(Request::Subscribe { .. }), false) => Reply::Subscribed(Ok(handles[0])),
                    _ => Reply::Unsubscribed(Err(Error::InvalidArgument)),
                };
                bytes.fill(0);
                let mut closed = Vec::new();
                snapshot.finish(decoded, reply, |handle| closed.push(handle));
                let kept = matches!(reply, Reply::Subscribed(Ok(_)));
                assert_eq!(
                    closed,
                    if kept {
                        vec![]
                    } else {
                        handles[..count].to_vec()
                    }
                );
            }
        }
    }
}

#[test]
fn wrong_protocol_handles_are_closed_not_leaked() {
    let mut bytes = [0; audhsos_abi::ipc_buffer::SIZE];
    let handle = Handle::new(1, 1).unwrap();
    let mut writer = BufferMut::new(&mut bytes);
    writer.set_counts(0, 1).unwrap();
    writer.set_handle(0, handle);
    let received = ReceivedHandles::read(Buffer::new(&bytes));
    assert!(Request::decode(Buffer::new(&bytes)).is_err());
    let mut closed = Vec::new();
    received.finish(
        None,
        Reply::Unsubscribed(Err(Error::InvalidArgument)),
        |handle| closed.push(handle),
    );
    assert_eq!(closed, [handle]);
}
