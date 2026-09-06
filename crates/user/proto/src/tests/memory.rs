// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::memory`.

use audhsos_abi::ipc_buffer::{Buffer, BufferMut, SIZE};
use audhsos_abi::{Error, Handle};

use crate::label::{Label, ProtoError, Protocol};
use crate::memory::{ALLOCATE, RELEASE, Reply, Request};

/// A buffer of zeros to work on.
fn buffer() -> [u8; SIZE] {
    [0; SIZE]
}

/// A handle every table could hand out.
fn handle(index: u32) -> Handle {
    Handle::new(index, 1).unwrap()
}

/// Encodes `request` and reads it back.
fn round_trip(request: &Request) -> Result<Request, ProtoError> {
    let mut bytes = buffer();
    request.encode(&mut BufferMut::new(&mut bytes))?;
    Request::decode(Buffer::new(&bytes))
}

/// Encodes `reply` and reads it back.
fn round_trip_reply(reply: &Reply) -> Result<Reply, ProtoError> {
    let mut bytes = buffer();
    reply.encode(&mut BufferMut::new(&mut bytes))?;
    Reply::decode(Buffer::new(&bytes))
}

#[test]
fn an_allocation_comes_back_with_its_length_and_its_alignment() {
    let request = Request::Allocate {
        len: 8192,
        align: 4096,
    };
    assert_eq!(round_trip(&request).unwrap(), request);
    assert_eq!(request.label(), Label::new(Protocol::Memory, ALLOCATE));
}

#[test]
fn a_release_comes_back_with_its_object() {
    let request = Request::Release { memory: handle(3) };
    assert_eq!(round_trip(&request).unwrap(), request);
    assert_eq!(request.label(), Label::new(Protocol::Memory, RELEASE));
}

#[test]
fn every_reply_comes_back_as_it_was_sent() {
    let cases = [
        Reply::Allocated(Ok(handle(5))),
        Reply::Allocated(Err(Error::OutOfKernelMemory)),
        Reply::Released(Ok(())),
        Reply::Released(Err(Error::InvalidHandle)),
    ];
    for reply in cases {
        assert_eq!(round_trip_reply(&reply).unwrap(), reply);
    }
}

#[test]
fn a_failed_allocation_carries_no_object() {
    let mut bytes = buffer();
    Reply::Allocated(Err(Error::OutOfKernelMemory))
        .encode(&mut BufferMut::new(&mut bytes))
        .unwrap();
    let message = Buffer::new(&bytes).message().unwrap();
    assert_eq!(message.handle_count, 0);
}

#[test]
fn a_message_of_another_protocol_is_refused() {
    let mut bytes = buffer();
    Request::Allocate { len: 1, align: 1 }
        .encode(&mut BufferMut::new(&mut bytes))
        .unwrap();
    BufferMut::new(&mut bytes).set_label(Label::new(Protocol::Console, 1).raw());
    assert_eq!(
        Request::decode(Buffer::new(&bytes)).unwrap_err(),
        ProtoError::WrongProtocol {
            expected: Protocol::Memory,
            found: Protocol::Console,
        }
    );
    assert_eq!(
        Reply::decode(Buffer::new(&bytes)).unwrap_err(),
        ProtoError::WrongProtocol {
            expected: Protocol::Memory,
            found: Protocol::Console,
        }
    );
}

#[test]
fn a_message_number_this_protocol_does_not_have_is_refused() {
    let mut bytes = buffer();
    Request::Allocate { len: 1, align: 1 }
        .encode(&mut BufferMut::new(&mut bytes))
        .unwrap();
    BufferMut::new(&mut bytes).set_label(Label::new(Protocol::Memory, 5).raw());
    assert_eq!(
        Request::decode(Buffer::new(&bytes)).unwrap_err(),
        ProtoError::Message(Protocol::Memory, 5)
    );

    let mut bytes = buffer();
    Reply::Released(Ok(()))
        .encode(&mut BufferMut::new(&mut bytes))
        .unwrap();
    BufferMut::new(&mut bytes).set_label(Label::new(Protocol::Memory, 5).raw());
    assert_eq!(
        Reply::decode(Buffer::new(&bytes)).unwrap_err(),
        ProtoError::Message(Protocol::Memory, 5)
    );
}

#[test]
fn an_allocation_missing_its_alignment_is_refused() {
    let mut bytes = buffer();
    Request::Allocate { len: 1, align: 1 }
        .encode(&mut BufferMut::new(&mut bytes))
        .unwrap();
    BufferMut::new(&mut bytes).set_counts(1, 0).unwrap();
    assert!(matches!(
        Request::decode(Buffer::new(&bytes)).unwrap_err(),
        ProtoError::Codec(_)
    ));
}

#[test]
fn a_release_without_its_object_is_refused() {
    let mut bytes = buffer();
    Request::Release { memory: handle(1) }
        .encode(&mut BufferMut::new(&mut bytes))
        .unwrap();
    BufferMut::new(&mut bytes).set_counts(0, 0).unwrap();
    assert!(matches!(
        Request::decode(Buffer::new(&bytes)).unwrap_err(),
        ProtoError::Codec(_)
    ));
}
