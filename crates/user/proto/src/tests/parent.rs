// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::parent`.

use audhsos_abi::ipc_buffer::{Buffer, BufferMut, SIZE};

use crate::label::{Label, ProtoError, Protocol};
use crate::parent::{FINISHED, Request, SUCCESS};

/// Encodes `request` and reads it back.
fn round_trip(request: &Request) -> Result<Request, ProtoError> {
    let mut bytes = [0u8; SIZE];
    request.encode(&mut BufferMut::new(&mut bytes))?;
    Request::decode(Buffer::new(&bytes))
}

#[test]
fn a_report_comes_back_with_the_status_it_carried() {
    for status in [SUCCESS, 1, u64::MAX] {
        let request = Request::Finished { status };
        assert_eq!(round_trip(&request).unwrap(), request);
    }
}

#[test]
fn a_report_is_sent_under_the_label_of_this_protocol() {
    let request = Request::Finished { status: SUCCESS };
    assert_eq!(request.label(), Label::new(Protocol::Parent, FINISHED));
}

#[test]
fn a_message_of_another_protocol_is_not_a_report() {
    let mut bytes = [0u8; SIZE];
    crate::name::Request::Lookup {
        name: crate::name::Name::new(b"console").unwrap(),
    }
    .encode(&mut BufferMut::new(&mut bytes))
    .unwrap();
    assert_eq!(
        Request::decode(Buffer::new(&bytes)).unwrap_err(),
        ProtoError::WrongProtocol {
            expected: Protocol::Parent,
            found: Protocol::Name,
        }
    );
}

#[test]
fn a_message_number_this_protocol_does_not_have_is_refused() {
    let mut bytes = [0u8; SIZE];
    {
        let mut buffer = BufferMut::new(&mut bytes);
        let mut writer = user_rt::message::Writer::new();
        writer.word(&mut buffer, 0).unwrap();
        writer
            .finish(&mut buffer, Label::new(Protocol::Parent, 7).raw())
            .unwrap();
    }
    assert_eq!(
        Request::decode(Buffer::new(&bytes)).unwrap_err(),
        ProtoError::Message(Protocol::Parent, 7)
    );
}

#[test]
fn a_report_without_its_status_word_is_refused() {
    let mut bytes = [0u8; SIZE];
    {
        let mut buffer = BufferMut::new(&mut bytes);
        let writer = user_rt::message::Writer::new();
        writer
            .finish(&mut buffer, Label::new(Protocol::Parent, FINISHED).raw())
            .unwrap();
    }
    assert!(matches!(
        Request::decode(Buffer::new(&bytes)).unwrap_err(),
        ProtoError::Codec(_)
    ));
}
