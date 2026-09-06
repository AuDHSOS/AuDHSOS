// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::handle`.

use audhsos_abi::{Handle, ObjectType};

use crate::handle::{
    EndpointHandle, InterruptHandle, IoPortHandle, MemoryHandle, NotificationHandle, ProcessHandle,
    ReplyHandle, SystemControlHandle, ThreadHandle, Typed,
};

/// A handle every table could hand out.
fn handle() -> Handle {
    Handle::new(7, 3).unwrap()
}

/// Builds a typed handle, takes it apart again, and says what came back.
fn round_trip<T: Typed>() -> (Handle, u64, ObjectType) {
    let typed = T::from_handle(handle());
    (typed.handle(), typed.raw(), T::TYPE)
}

#[test]
fn every_typed_handle_carries_the_handle_it_was_built_from() {
    let cases = [
        round_trip::<ProcessHandle>(),
        round_trip::<ThreadHandle>(),
        round_trip::<MemoryHandle>(),
        round_trip::<EndpointHandle>(),
        round_trip::<ReplyHandle>(),
        round_trip::<NotificationHandle>(),
        round_trip::<InterruptHandle>(),
        round_trip::<IoPortHandle>(),
        round_trip::<SystemControlHandle>(),
    ];
    for (back, raw, _) in cases {
        assert_eq!(back, handle());
        assert_eq!(raw, handle().raw());
    }
}

#[test]
fn the_nine_types_name_the_nine_object_types_and_no_two_name_the_same() {
    let named = [
        ProcessHandle::TYPE,
        ThreadHandle::TYPE,
        MemoryHandle::TYPE,
        EndpointHandle::TYPE,
        ReplyHandle::TYPE,
        NotificationHandle::TYPE,
        InterruptHandle::TYPE,
        IoPortHandle::TYPE,
        SystemControlHandle::TYPE,
    ];
    assert_eq!(named.len(), ObjectType::ALL.len());
    for object in ObjectType::ALL {
        assert!(
            named.contains(object),
            "no typed handle names {}",
            object.name()
        );
    }
}

#[test]
fn a_word_that_is_no_handle_names_no_capability() {
    assert_eq!(EndpointHandle::from_raw(0), None);
    assert_eq!(
        EndpointHandle::from_raw(handle().raw()),
        Some(EndpointHandle::from_handle(handle()))
    );
}

#[test]
fn a_typed_handle_converts_back_to_a_plain_one() {
    let typed = MemoryHandle::from_handle(handle());
    assert_eq!(Handle::from(typed), handle());
    let typed = SystemControlHandle::from_handle(handle());
    assert_eq!(Handle::from(typed), handle());
}
