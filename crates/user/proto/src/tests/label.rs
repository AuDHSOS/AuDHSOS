// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::label`.

use audhsos_abi::Error;
use audhsos_abi::ipc_buffer::KERNEL_LABEL_BASE;
use user_rt::message::CodecError;

use crate::label::{Label, ProtoError, Protocol, VERSION, status_of, status_word};

#[test]
fn every_protocol_has_a_unique_code_and_a_name() {
    for (index, protocol) in Protocol::ALL.iter().enumerate() {
        assert_ne!(protocol.code(), 0);
        assert_eq!(Protocol::from_code(protocol.code()), Some(*protocol));
        assert!(!protocol.name().is_empty());
        for other in Protocol::ALL.iter().skip(index.wrapping_add(1)) {
            assert_ne!(protocol.code(), other.code());
        }
    }
}

#[test]
fn a_code_no_protocol_has_decodes_to_none() {
    let highest = Protocol::ALL
        .iter()
        .map(|protocol| protocol.code())
        .max()
        .unwrap();
    assert_eq!(Protocol::from_code(0), None);
    assert_eq!(Protocol::from_code(highest.checked_add(1).unwrap()), None);
    assert_eq!(Protocol::from_code(u16::MAX), None);
}

#[test]
fn a_label_reads_back_as_what_it_was_built_from() {
    for protocol in Protocol::ALL {
        for message in [0u16, 1, 2, u16::MAX] {
            let label = Label::new(*protocol, message);
            assert_eq!(label.version, VERSION);
            assert_eq!(Label::parse(label.raw()).unwrap(), label);
        }
    }
}

#[test]
fn every_label_lies_below_the_range_the_kernel_keeps() {
    for protocol in Protocol::ALL {
        let label = Label::new(*protocol, u16::MAX);
        assert!(label.raw() < KERNEL_LABEL_BASE, "{:#x}", label.raw());
    }
}

#[test]
fn a_reserved_bit_is_refused_before_anything_else() {
    let raw = Label::new(Protocol::Name, 1).raw() | (1 << 32);
    assert_eq!(Label::parse(raw).unwrap_err(), ProtoError::Reserved(raw));
}

#[test]
fn a_version_this_crate_does_not_speak_is_refused() {
    let later = u64::from(VERSION).wrapping_add(1) << 48;
    assert_eq!(
        Label::parse(later).unwrap_err(),
        ProtoError::Version(VERSION.wrapping_add(1))
    );
    // Version zero, which is what an all-zero label looks like.
    assert_eq!(Label::parse(0).unwrap_err(), ProtoError::Version(0));
}

#[test]
fn the_version_is_read_before_the_protocol() {
    // A later version over a protocol that does not exist: the version is
    // what is reported, so a client of a newer release is told the truth.
    let raw = (u64::from(VERSION).wrapping_add(1) << 48) | (99 << 16);
    assert_eq!(
        Label::parse(raw).unwrap_err(),
        ProtoError::Version(VERSION.wrapping_add(1))
    );
}

#[test]
fn a_protocol_this_crate_does_not_know_is_refused() {
    let raw = (u64::from(VERSION) << 48) | (99 << 16);
    assert_eq!(Label::parse(raw).unwrap_err(), ProtoError::Protocol(99));
}

#[test]
fn a_status_word_of_zero_is_a_success() {
    assert_eq!(status_of(0).unwrap(), Ok(()));
    assert_eq!(status_word(Ok(())), 0);
}

#[test]
fn a_status_word_that_names_an_error_comes_back_as_that_error() {
    for error in Error::ALL {
        let word = status_word(Err(*error));
        assert_eq!(word, u64::from(error.code()));
        assert_eq!(status_of(word).unwrap(), Err(*error));
    }
}

#[test]
fn a_status_word_that_names_no_error_is_refused() {
    let highest = Error::ALL.iter().map(|error| error.code()).max().unwrap();
    let unknown = u64::from(highest).wrapping_add(1);
    assert_eq!(status_of(unknown).unwrap_err(), ProtoError::Status(unknown));
    assert_eq!(
        status_of(u64::MAX).unwrap_err(),
        ProtoError::Status(u64::MAX)
    );
}

#[test]
fn every_error_renders_a_message_and_an_error_code() {
    let cases = [
        ProtoError::Reserved(1),
        ProtoError::Version(9),
        ProtoError::Protocol(9),
        ProtoError::Message(Protocol::Name, 9),
        ProtoError::WrongProtocol {
            expected: Protocol::Name,
            found: Protocol::Console,
        },
        ProtoError::TooLong {
            len: 9,
            capacity: 4,
        },
        ProtoError::Status(999),
        ProtoError::Codec(CodecError::Truncated),
    ];
    for case in cases {
        assert!(!format!("{case}").is_empty(), "{case:?} has no message");
        assert_ne!(Error::from(case).code(), 0);
    }
    assert_eq!(
        Error::from(ProtoError::Codec(CodecError::Full)),
        Error::BufferTooSmall
    );
}
