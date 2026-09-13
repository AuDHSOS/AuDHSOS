// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::file`.

use audhsos_abi::Error;
use audhsos_abi::ipc_buffer::{Buffer, BufferMut, SIZE};

use crate::file::{
    CLOSE, CREATE, Data, DirEntry, FLUSH, MAX_DATA, MAX_NAME, Name, OPEN, Opened, READ, READ_DIR,
    REMOVE, ROOT, Reply, Request, START, STAT, Stat, WRITE,
};
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

/// The name a test opens.
fn name() -> Name {
    Name::new(b"NOTES.TXT").unwrap()
}

#[test]
fn every_request_comes_back_as_it_went_out() {
    let requests = [
        Request::Open {
            parent: ROOT,
            name: name(),
        },
        Request::Create {
            parent: ROOT,
            name: name(),
            directory: true,
        },
        Request::Create {
            parent: 7,
            name: name(),
            directory: false,
        },
        Request::Read {
            file: 3,
            offset: 512,
            len: 64,
        },
        Request::Write {
            file: 3,
            offset: 0,
            data: Data::new(b"one line\n").unwrap(),
        },
        Request::ReadDir {
            dir: ROOT,
            cursor: START,
        },
        Request::ReadDir {
            dir: ROOT,
            cursor: u64::MAX,
        },
        Request::Stat { file: 3 },
        Request::Remove {
            parent: ROOT,
            name: name(),
        },
        Request::Close { file: 3 },
        Request::Flush,
    ];
    for request in requests {
        assert_eq!(round_trip(&request).unwrap(), request, "{request:?}");
    }
}

#[test]
fn every_request_carries_the_message_number_of_its_kind() {
    let cases = [
        (
            Request::Open {
                parent: ROOT,
                name: name(),
            },
            OPEN,
        ),
        (
            Request::Create {
                parent: ROOT,
                name: name(),
                directory: false,
            },
            CREATE,
        ),
        (
            Request::Read {
                file: 1,
                offset: 0,
                len: 1,
            },
            READ,
        ),
        (
            Request::Write {
                file: 1,
                offset: 0,
                data: Data::empty(),
            },
            WRITE,
        ),
        (
            Request::ReadDir {
                dir: ROOT,
                cursor: START,
            },
            READ_DIR,
        ),
        (Request::Stat { file: 1 }, STAT),
        (
            Request::Remove {
                parent: ROOT,
                name: name(),
            },
            REMOVE,
        ),
        (Request::Close { file: 1 }, CLOSE),
        (Request::Flush, FLUSH),
    ];
    for (request, message) in cases {
        assert_eq!(request.label(), Label::new(Protocol::File, message));
    }
}

#[test]
fn every_reply_comes_back_as_it_went_out() {
    let replies = [
        Reply::Opened(Ok(Opened {
            file: 4,
            size: 1234,
            directory: false,
        })),
        Reply::Opened(Ok(Opened {
            file: ROOT,
            size: 0,
            directory: true,
        })),
        Reply::Created(Ok(5)),
        Reply::Read(Ok(Data::new(b"bytes").unwrap())),
        Reply::Written(Ok(9)),
        Reply::Entry(Ok(Some(DirEntry {
            name: name(),
            size: 20,
            directory: false,
            cursor: 3,
        }))),
        Reply::Entry(Ok(None)),
        Reply::Stat(Ok(Stat {
            size: 20,
            attributes: 0x20,
            modified: 1_700_000_000,
        })),
        Reply::Removed(Ok(())),
        Reply::Closed(Ok(())),
        Reply::Flushed(Ok(())),
    ];
    for reply in replies {
        assert_eq!(round_trip_reply(&reply).unwrap(), reply, "{reply:?}");
    }
}

#[test]
fn every_reply_carries_a_refusal_back_under_its_own_message() {
    let replies = [
        Reply::Opened(Err(Error::NotFound)),
        Reply::Created(Err(Error::AlreadyExists)),
        Reply::Read(Err(Error::InvalidHandle)),
        Reply::Written(Err(Error::AccessDenied)),
        Reply::Entry(Err(Error::InvalidArgument)),
        Reply::Stat(Err(Error::NotFound)),
        Reply::Removed(Err(Error::NotFound)),
        Reply::Closed(Err(Error::InvalidHandle)),
        Reply::Flushed(Err(Error::Unsupported)),
    ];
    for reply in replies {
        assert_eq!(round_trip_reply(&reply).unwrap(), reply, "{reply:?}");
    }
}

#[test]
fn a_name_of_no_bytes_and_one_of_the_full_width_both_travel() {
    for name in [Name::empty(), Name::new(&[b'A'; MAX_NAME]).unwrap()] {
        let request = Request::Open { parent: ROOT, name };
        assert_eq!(round_trip(&request).unwrap(), request);
    }
}

#[test]
fn a_whole_sector_travels_in_one_message() {
    let data = Data::new(&[0x5A; MAX_DATA]).unwrap();
    let request = Request::Write {
        file: 1,
        offset: 0,
        data,
    };
    assert_eq!(round_trip(&request).unwrap(), request);
    let reply = Reply::Read(Ok(data));
    assert_eq!(round_trip_reply(&reply).unwrap(), reply);
}

#[test]
fn more_bytes_than_one_message_carries_are_refused_before_they_are_sent() {
    assert_eq!(
        Data::new(&[0; MAX_DATA + 1]).unwrap_err(),
        ProtoError::TooLong {
            len: MAX_DATA + 1,
            capacity: MAX_DATA,
        }
    );
    assert_eq!(
        Name::new(&[b'A'; MAX_NAME + 1]).unwrap_err(),
        ProtoError::TooLong {
            len: MAX_NAME + 1,
            capacity: MAX_NAME,
        }
    );
}

#[test]
fn a_message_number_this_protocol_has_not_is_refused() {
    let mut bytes = buffer();
    Request::Flush
        .encode(&mut BufferMut::new(&mut bytes))
        .unwrap();
    let unknown = Label {
        version: crate::label::VERSION,
        protocol: Protocol::File,
        message: 99,
    };
    BufferMut::new(&mut bytes).set_label(unknown.raw());
    assert_eq!(
        Request::decode(Buffer::new(&bytes)).unwrap_err(),
        ProtoError::Message(Protocol::File, 99)
    );
    // A reply reads its status word before its message number, so the
    // buffer it is read out of has to carry one.
    let mut bytes = buffer();
    Reply::Flushed(Ok(()))
        .encode(&mut BufferMut::new(&mut bytes))
        .unwrap();
    BufferMut::new(&mut bytes).set_label(unknown.raw());
    assert_eq!(
        Reply::decode(Buffer::new(&bytes)).unwrap_err(),
        ProtoError::Message(Protocol::File, 99)
    );
}

#[test]
fn a_message_of_another_protocol_is_refused() {
    let mut bytes = buffer();
    crate::name::Reply::Registered(Ok(()))
        .encode(&mut BufferMut::new(&mut bytes))
        .unwrap();
    assert_eq!(
        Request::decode(Buffer::new(&bytes)).unwrap_err(),
        ProtoError::WrongProtocol {
            expected: Protocol::File,
            found: Protocol::Name,
        }
    );
    assert_eq!(
        Reply::decode(Buffer::new(&bytes)).unwrap_err(),
        ProtoError::WrongProtocol {
            expected: Protocol::File,
            found: Protocol::Name,
        }
    );
}
