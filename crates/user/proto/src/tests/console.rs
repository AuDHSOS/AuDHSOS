// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::console`.

use audhsos_abi::Error;
use audhsos_abi::ipc_buffer::{Buffer, BufferMut, SIZE};

use crate::console::{Chunk, MAX_CHUNK, READ, Reply, Request, WRITE, write_status};
use crate::label::{Label, ProtoError, Protocol};

/// A buffer of zeros to work on.
fn buffer() -> [u8; SIZE] {
    [0; SIZE]
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
fn a_write_comes_back_with_its_bytes() {
    let request = Request::Write {
        bytes: Chunk::new(b"hello from userland\n").unwrap(),
    };
    assert_eq!(round_trip(&request).unwrap(), request);
    assert_eq!(request.label(), Label::new(Protocol::Console, WRITE));
}

#[test]
fn a_read_comes_back_with_its_bound() {
    let request = Request::Read { max: 64 };
    assert_eq!(round_trip(&request).unwrap(), request);
    assert_eq!(request.label(), Label::new(Protocol::Console, READ));
}

#[test]
fn a_chunk_of_no_bytes_and_one_of_the_full_width_both_travel() {
    for bytes in [Chunk::empty(), Chunk::new(&[0x5A; MAX_CHUNK]).unwrap()] {
        let request = Request::Write { bytes };
        assert_eq!(round_trip(&request).unwrap(), request);
        let reply = Reply::Read(Ok(bytes));
        assert_eq!(round_trip_reply(&reply).unwrap(), reply);
    }
}

#[test]
fn every_reply_comes_back_as_it_was_sent() {
    let cases = [
        Reply::Written(Ok(20)),
        Reply::Written(Ok(0)),
        Reply::Written(Err(Error::Busy)),
        Reply::Read(Ok(Chunk::new(b"abc").unwrap())),
        Reply::Read(Ok(Chunk::empty())),
        Reply::Read(Err(Error::WouldBlock)),
    ];
    for reply in cases {
        assert_eq!(round_trip_reply(&reply).unwrap(), reply);
    }
}

#[test]
fn a_failed_write_carries_no_count() {
    let mut bytes = buffer();
    Reply::Written(Err(Error::Busy))
        .encode(&mut BufferMut::new(&mut bytes))
        .unwrap();
    let message = Buffer::new(&bytes).message().unwrap();
    assert_eq!(message.word_count, 1);
}

#[test]
fn the_status_word_of_a_write_is_the_common_one() {
    assert_eq!(write_status(Ok(())), 0);
    assert_eq!(
        write_status(Err(Error::Busy)),
        u64::from(Error::Busy.code())
    );
}

#[test]
fn a_message_of_another_protocol_is_refused() {
    let mut bytes = buffer();
    Request::Read { max: 1 }
        .encode(&mut BufferMut::new(&mut bytes))
        .unwrap();
    BufferMut::new(&mut bytes).set_label(Label::new(Protocol::Name, 1).raw());
    assert_eq!(
        Request::decode(Buffer::new(&bytes)).unwrap_err(),
        ProtoError::WrongProtocol {
            expected: Protocol::Console,
            found: Protocol::Name,
        }
    );
    assert_eq!(
        Reply::decode(Buffer::new(&bytes)).unwrap_err(),
        ProtoError::WrongProtocol {
            expected: Protocol::Console,
            found: Protocol::Name,
        }
    );
}

#[test]
fn a_message_number_this_protocol_does_not_have_is_refused() {
    let mut bytes = buffer();
    Request::Read { max: 1 }
        .encode(&mut BufferMut::new(&mut bytes))
        .unwrap();
    BufferMut::new(&mut bytes).set_label(Label::new(Protocol::Console, 7).raw());
    assert_eq!(
        Request::decode(Buffer::new(&bytes)).unwrap_err(),
        ProtoError::Message(Protocol::Console, 7)
    );

    let mut bytes = buffer();
    Reply::Written(Ok(1))
        .encode(&mut BufferMut::new(&mut bytes))
        .unwrap();
    BufferMut::new(&mut bytes).set_label(Label::new(Protocol::Console, 7).raw());
    assert_eq!(
        Reply::decode(Buffer::new(&bytes)).unwrap_err(),
        ProtoError::Message(Protocol::Console, 7)
    );
}

#[test]
fn a_reply_that_says_it_succeeded_and_carries_nothing_is_refused() {
    let mut bytes = buffer();
    {
        let mut view = BufferMut::new(&mut bytes);
        view.set_word(0, 0);
        view.set_label(Label::new(Protocol::Console, WRITE).raw());
        view.set_counts(1, 0).unwrap();
    }
    assert!(matches!(
        Reply::decode(Buffer::new(&bytes)).unwrap_err(),
        ProtoError::Codec(_)
    ));
}
