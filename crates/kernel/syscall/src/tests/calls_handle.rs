// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of the handle calls and of `debug_log`.

use audhsos_abi::ipc_buffer::{BufferMut, SIZE};
use audhsos_abi::{Error, Handle, Rights, Syscall};
use kernel_objects::object::AnyObjectId;

use crate::tests::double::{Call, Fixture, call, error_of, request, value_of};

#[test]
fn duplicating_a_handle_gives_a_second_name_for_the_same_object() {
    let mut fixture = Fixture::new();
    let own = fixture.own_thread.raw();
    let rights = (Rights::MANAGE | Rights::DUPLICATE).bits();
    let raw = value_of(
        &mut fixture,
        request(Syscall::HandleDuplicate, &[own, u64::from(rights)]),
    );
    let copy = Handle::from_raw(raw).expect("a handle");
    assert_ne!(copy, fixture.own_thread);
    let original = fixture
        .objects
        .entry(fixture.process, fixture.own_thread)
        .unwrap();
    let duplicate = fixture.objects.entry(fixture.process, copy).unwrap();
    assert_eq!(duplicate.object, original.object);
    assert_eq!(duplicate.rights.bits(), rights);
}

#[test]
fn duplicating_with_more_rights_than_the_original_is_refused() {
    let mut fixture = Fixture::new();
    let weak = fixture
        .install(
            AnyObjectId::of(fixture.thread),
            Rights::MANAGE | Rights::DUPLICATE,
        )
        .raw();
    let more = (Rights::MANAGE | Rights::DUPLICATE | Rights::TRANSFER).bits();
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::HandleDuplicate, &[weak, u64::from(more)])
        ),
        Some(Error::AccessDenied)
    );
}

#[test]
fn duplicating_a_handle_that_may_not_be_duplicated_is_refused() {
    let mut fixture = Fixture::new();
    let plain = fixture
        .install(AnyObjectId::of(fixture.thread), Rights::MANAGE)
        .raw();
    assert_eq!(
        error_of(
            &mut fixture,
            request(
                Syscall::HandleDuplicate,
                &[plain, u64::from(Rights::MANAGE.bits())]
            )
        ),
        Some(Error::AccessDenied)
    );
}

#[test]
fn duplicating_with_bits_that_name_no_right_is_refused() {
    let mut fixture = Fixture::new();
    let own = fixture.own_thread.raw();
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::HandleDuplicate, &[own, 0xFFFF_FFFF])
        ),
        Some(Error::InvalidArgument)
    );
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::HandleDuplicate, &[own, u64::from(u32::MAX) + 1])
        ),
        Some(Error::InvalidArgument),
        "a right set that is not even a word"
    );
}

#[test]
fn duplicating_beyond_the_capacity_of_the_process_is_refused() {
    let mut fixture = Fixture::new();
    let own = fixture.own_thread.raw();
    let rights = u64::from((Rights::MANAGE | Rights::DUPLICATE).bits());
    // The fixture grants sixteen handles and holds two.
    for index in 0..14 {
        assert!(
            error_of(
                &mut fixture,
                request(Syscall::HandleDuplicate, &[own, rights])
            )
            .is_none(),
            "handle {index}"
        );
    }
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::HandleDuplicate, &[own, rights])
        ),
        Some(Error::QuotaExceeded)
    );
}

#[test]
fn closing_a_handle_takes_the_name_away_and_closing_it_twice_fails() {
    let mut fixture = Fixture::new();
    let own = fixture.own_thread.raw();
    assert!(error_of(&mut fixture, request(Syscall::HandleClose, &[own])).is_none());
    assert_eq!(
        error_of(&mut fixture, request(Syscall::HandleClose, &[own])),
        Some(Error::InvalidHandle)
    );
    // What it named is still there; a handle is a name, not the object.
    assert!(fixture.objects.threads.get(fixture.thread).is_ok());
}

#[test]
fn closing_a_handle_needs_no_right_at_all() {
    let mut fixture = Fixture::new();
    let bare = fixture
        .install(AnyObjectId::of(fixture.thread), Rights::EMPTY)
        .raw();
    assert!(error_of(&mut fixture, request(Syscall::HandleClose, &[bare])).is_none());
}

#[test]
fn a_handle_call_on_a_handle_of_nobody_is_refused() {
    let mut fixture = Fixture::new();
    let ghost = Handle::new(30, 3).unwrap().raw();
    assert_eq!(
        error_of(&mut fixture, request(Syscall::HandleClose, &[ghost])),
        Some(Error::InvalidHandle)
    );
    assert_eq!(
        error_of(&mut fixture, request(Syscall::HandleDuplicate, &[ghost, 0])),
        Some(Error::InvalidHandle)
    );
}

/// A buffer holding a `debug_log` whose message area carries `text`.
fn logging(text: &str) -> [u8; SIZE] {
    let mut buffer = request(Syscall::DebugLog, &[]);
    let bytes = text.as_bytes();
    let words = bytes.len().div_ceil(8);
    let trailing = bytes.len() % 8;
    let mut writer = BufferMut::new(&mut buffer);
    writer.set_counts(words, 0).unwrap();
    writer.set_label(u64::try_from(trailing).unwrap());
    for (index, chunk) in bytes.chunks(8).enumerate() {
        let mut word = [0_u8; 8];
        word.get_mut(..chunk.len()).unwrap().copy_from_slice(chunk);
        assert!(writer.set_word(index, u64::from_le_bytes(word)));
    }
    buffer
}

#[test]
fn debug_log_writes_the_message_area_and_says_how_much() {
    let mut fixture = Fixture::new();
    let mut buffer = logging("hello, kernel!");
    let (status, values, _) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), None);
    assert_eq!(values[0], 14, "fourteen bytes reached the console");
    assert_eq!(fixture.environment.count(&Call::Log(8)), 1);
    assert_eq!(fixture.environment.count(&Call::Log(6)), 1);
}

#[test]
fn a_message_that_fills_its_last_word_is_written_whole() {
    let mut fixture = Fixture::new();
    let mut buffer = logging("sixteen bytes!!!");
    let (status, values, _) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), None);
    assert_eq!(values[0], 16);
    assert_eq!(fixture.environment.count(&Call::Log(8)), 2);
}

#[test]
fn an_empty_message_writes_nothing() {
    let mut fixture = Fixture::new();
    let mut buffer = logging("");
    let (status, values, _) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), None);
    assert_eq!(values[0], 0);
    assert_eq!(fixture.environment.calls.len(), 0);
}

#[test]
fn a_message_longer_than_the_area_is_refused_before_a_byte_is_written() {
    let mut fixture = Fixture::new();
    let mut buffer = request(Syscall::DebugLog, &[]);
    // Past the accessors: a word count no message can have.
    let count = u64::try_from(audhsos_abi::layout::MAX_MESSAGE_WORDS).unwrap() + 1;
    let offset = audhsos_abi::ipc_buffer::WORD_COUNT;
    buffer
        .get_mut(offset..offset + 8)
        .unwrap()
        .copy_from_slice(&count.to_le_bytes());
    let (status, _, _) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), Some(Error::InvalidArgument));
    assert_eq!(fixture.environment.calls.len(), 0);
}
