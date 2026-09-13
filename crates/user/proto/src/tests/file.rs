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
    for request in requests() {
        assert_eq!(round_trip(&request).unwrap(), request, "{request:?}");
    }
}

/// One request of every kind, answered one at a time: a value of this
/// protocol carries [`MAX_DATA`] bytes, so an array of them is kibibytes
/// of stack.
fn requests() -> impl Iterator<Item = Request> {
    let cases = [
        Case::Open,
        Case::CreateDir,
        Case::CreateFile,
        Case::Read,
        Case::Write,
        Case::ReadDirStart,
        Case::ReadDirEnd,
        Case::Stat,
        Case::Remove,
        Case::Close,
        Case::Flush,
    ];
    cases.into_iter().map(build)
}

/// What one case of [`requests`] stands for.
#[derive(Clone, Copy, Debug)]
enum Case {
    Open,
    CreateDir,
    CreateFile,
    Read,
    Write,
    ReadDirStart,
    ReadDirEnd,
    Stat,
    Remove,
    Close,
    Flush,
}

/// The request `case` stands for.
fn build(case: Case) -> Request {
    match case {
        Case::Open => Request::Open {
            parent: ROOT,
            name: name(),
        },
        Case::CreateDir => Request::Create {
            parent: ROOT,
            name: name(),
            directory: true,
        },
        Case::CreateFile => Request::Create {
            parent: 7,
            name: name(),
            directory: false,
        },
        Case::Read => Request::Read {
            file: 3,
            offset: 512,
            len: 64,
        },
        Case::Write => Request::Write {
            file: 3,
            offset: 0,
            data: Data::new(b"one line\n").unwrap(),
        },
        Case::ReadDirStart => Request::ReadDir {
            dir: ROOT,
            cursor: START,
        },
        Case::ReadDirEnd => Request::ReadDir {
            dir: ROOT,
            cursor: u64::MAX,
        },
        Case::Stat => Request::Stat { file: 3 },
        Case::Remove => Request::Remove {
            parent: ROOT,
            name: name(),
        },
        Case::Close => Request::Close { file: 3 },
        Case::Flush => Request::Flush,
    }
}

#[test]
fn every_request_carries_the_message_number_of_its_kind() {
    let numbers = [
        (Case::Open, OPEN),
        (Case::CreateDir, CREATE),
        (Case::Read, READ),
        (Case::Write, WRITE),
        (Case::ReadDirStart, READ_DIR),
        (Case::Stat, STAT),
        (Case::Remove, REMOVE),
        (Case::Close, CLOSE),
        (Case::Flush, FLUSH),
    ];
    for (case, message) in numbers {
        assert_eq!(build(case).label(), Label::new(Protocol::File, message));
    }
}

#[test]
fn every_reply_comes_back_as_it_went_out() {
    for reply in replies() {
        assert_eq!(round_trip_reply(&reply).unwrap(), reply, "{reply:?}");
    }
}

/// One reply of every kind, answered one at a time and for the reason
/// [`requests`] is written that way.
fn replies() -> impl Iterator<Item = Reply> {
    (0u8..11).map(|index| match index {
        0 => Reply::Opened(Ok(Opened {
            file: 4,
            size: 1234,
            directory: false,
        })),
        1 => Reply::Opened(Ok(Opened {
            file: ROOT,
            size: 0,
            directory: true,
        })),
        2 => Reply::Created(Ok(5)),
        3 => Reply::Read(Ok(Data::new(b"bytes").unwrap())),
        4 => Reply::Written(Ok(9)),
        5 => Reply::Entry(Ok(Some(DirEntry {
            name: name(),
            size: 20,
            directory: false,
            cursor: 3,
        }))),
        6 => Reply::Entry(Ok(None)),
        7 => Reply::Stat(Ok(Stat {
            size: 20,
            attributes: 0x20,
            modified: 1_700_000_000,
        })),
        8 => Reply::Removed(Ok(())),
        9 => Reply::Closed(Ok(())),
        _ => Reply::Flushed(Ok(())),
    })
}

#[test]
fn every_reply_carries_a_refusal_back_under_its_own_message() {
    let refusals = (0u8..9).map(|index| match index {
        0 => Reply::Opened(Err(Error::NotFound)),
        1 => Reply::Created(Err(Error::AlreadyExists)),
        2 => Reply::Read(Err(Error::InvalidHandle)),
        3 => Reply::Written(Err(Error::AccessDenied)),
        4 => Reply::Entry(Err(Error::InvalidArgument)),
        5 => Reply::Stat(Err(Error::NotFound)),
        6 => Reply::Removed(Err(Error::NotFound)),
        7 => Reply::Closed(Err(Error::InvalidHandle)),
        _ => Reply::Flushed(Err(Error::Unsupported)),
    });
    for reply in refusals {
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
