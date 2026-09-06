// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::name`.

use audhsos_abi::ipc_buffer::{Buffer, BufferMut, SIZE};
use audhsos_abi::{Error, Handle};

use crate::label::{Label, ProtoError, Protocol};
use crate::name::{LOOKUP, MAX_NAME, Name, REGISTER, Reply, Request};

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
fn a_registration_comes_back_with_its_name_and_its_endpoint() {
    let request = Request::Register {
        name: Name::new(b"console").unwrap(),
        endpoint: handle(4),
    };
    assert_eq!(round_trip(&request).unwrap(), request);
    assert_eq!(request.label(), Label::new(Protocol::Name, REGISTER));
}

#[test]
fn a_lookup_comes_back_with_its_name() {
    let request = Request::Lookup {
        name: Name::new(b"memory").unwrap(),
    };
    assert_eq!(round_trip(&request).unwrap(), request);
    assert_eq!(request.label(), Label::new(Protocol::Name, LOOKUP));
}

#[test]
fn a_name_of_no_bytes_and_a_name_of_the_full_width_both_travel() {
    for name in [Name::empty(), Name::new(&[0x41; MAX_NAME]).unwrap()] {
        let request = Request::Lookup { name };
        assert_eq!(round_trip(&request).unwrap(), request);
    }
}

#[test]
fn every_reply_comes_back_as_it_was_sent() {
    let cases = [
        Reply::Registered(Ok(())),
        Reply::Registered(Err(Error::AlreadyExists)),
        Reply::Found(Ok(handle(7))),
        Reply::Found(Err(Error::NotFound)),
    ];
    for reply in cases {
        assert_eq!(round_trip_reply(&reply).unwrap(), reply);
    }
}

#[test]
fn a_failed_lookup_carries_no_handle() {
    let mut bytes = buffer();
    Reply::Found(Err(Error::NotFound))
        .encode(&mut BufferMut::new(&mut bytes))
        .unwrap();
    let message = Buffer::new(&bytes).message().unwrap();
    assert_eq!(message.handle_count, 0);
}

#[test]
fn a_message_of_another_protocol_is_refused() {
    let mut bytes = buffer();
    Request::Lookup {
        name: Name::empty(),
    }
    .encode(&mut BufferMut::new(&mut bytes))
    .unwrap();
    BufferMut::new(&mut bytes).set_label(Label::new(Protocol::Memory, 1).raw());
    assert_eq!(
        Request::decode(Buffer::new(&bytes)).unwrap_err(),
        ProtoError::WrongProtocol {
            expected: Protocol::Name,
            found: Protocol::Memory,
        }
    );
    assert_eq!(
        Reply::decode(Buffer::new(&bytes)).unwrap_err(),
        ProtoError::WrongProtocol {
            expected: Protocol::Name,
            found: Protocol::Memory,
        }
    );
}

#[test]
fn a_message_number_this_protocol_does_not_have_is_refused() {
    let mut bytes = buffer();
    Request::Lookup {
        name: Name::empty(),
    }
    .encode(&mut BufferMut::new(&mut bytes))
    .unwrap();
    BufferMut::new(&mut bytes).set_label(Label::new(Protocol::Name, 9).raw());
    assert_eq!(
        Request::decode(Buffer::new(&bytes)).unwrap_err(),
        ProtoError::Message(Protocol::Name, 9)
    );

    let mut bytes = buffer();
    Reply::Registered(Ok(()))
        .encode(&mut BufferMut::new(&mut bytes))
        .unwrap();
    BufferMut::new(&mut bytes).set_label(Label::new(Protocol::Name, 9).raw());
    assert_eq!(
        Reply::decode(Buffer::new(&bytes)).unwrap_err(),
        ProtoError::Message(Protocol::Name, 9)
    );
}

#[test]
fn a_registration_without_its_handle_is_refused() {
    let mut bytes = buffer();
    Request::Register {
        name: Name::new(b"x").unwrap(),
        endpoint: handle(1),
    }
    .encode(&mut BufferMut::new(&mut bytes))
    .unwrap();
    BufferMut::new(&mut bytes).set_counts(2, 0).unwrap();
    assert!(matches!(
        Request::decode(Buffer::new(&bytes)).unwrap_err(),
        ProtoError::Codec(_)
    ));
}

#[test]
fn a_name_longer_than_the_field_is_refused_where_it_is_read() {
    // Written by hand, because the encoder cannot build such a message.
    let mut bytes = buffer();
    {
        let mut view = BufferMut::new(&mut bytes);
        let len = u64::try_from(MAX_NAME).unwrap().wrapping_add(1);
        view.set_word(0, len);
        for index in 1..=5 {
            view.set_word(index, 0x4141_4141_4141_4141);
        }
        view.set_label(Label::new(Protocol::Name, LOOKUP).raw());
        view.set_counts(6, 0).unwrap();
    }
    assert!(matches!(
        Request::decode(Buffer::new(&bytes)).unwrap_err(),
        ProtoError::Codec(_)
    ));
}
