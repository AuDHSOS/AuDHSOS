// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::calls::notify`, covering the notification items of
//! catalog 6.6.8 as a user thread reaches them.

use audhsos_abi::{Error, Handle, ObjectType, Rights, Syscall, ThreadState};
use kernel_objects::object::{AnyObjectId, Notification, NotificationId};

use super::double::{Fixture, call, error_of, request, value_of};

/// A notification of the calling process and the handle to it.
fn notification(fixture: &mut Fixture) -> (NotificationId, Handle) {
    let raw = value_of(fixture, request(Syscall::NotificationCreate, &[]));
    let handle = Handle::from_raw(raw).unwrap();
    let entry = fixture.objects.entry(fixture.process, handle).unwrap();
    (entry.object.typed::<Notification>().unwrap(), handle)
}

#[test]
fn a_notification_comes_with_the_rights_of_one() {
    let mut fixture = Fixture::new();
    let (id, handle) = notification(&mut fixture);
    let entry = fixture.objects.entry(fixture.process, handle).unwrap();
    assert_eq!(entry.object_type(), ObjectType::Notification);
    assert_eq!(
        entry.rights,
        Rights::SIGNAL | Rights::WAIT | Rights::BIND | Rights::DUPLICATE | Rights::TRANSFER
    );
    assert_eq!(fixture.objects.notifications.references(id), Ok(1));
}

#[test]
fn a_notification_that_has_nowhere_to_go_leaves_no_slot_behind() {
    let mut fixture = Fixture::new();
    fixture.fill_handles();
    assert_eq!(
        error_of(&mut fixture, request(Syscall::NotificationCreate, &[])),
        Some(Error::QuotaExceeded)
    );
    assert_eq!(fixture.objects.notifications.live(), 0);
    assert_eq!(
        fixture
            .objects
            .processes
            .get(fixture.process)
            .unwrap()
            .kernel_object_quota
            .used(),
        0
    );
}

#[test]
fn signals_accumulate_until_a_wait_takes_them_all() {
    let mut fixture = Fixture::new();
    let (_id, handle) = notification(&mut fixture);
    for bits in [0b0010_u64, 0b1000] {
        assert!(
            error_of(
                &mut fixture,
                request(Syscall::NotificationSignal, &[handle.raw(), bits])
            )
            .is_none()
        );
    }
    let word = value_of(
        &mut fixture,
        request(Syscall::NotificationWait, &[handle.raw()]),
    );
    assert_eq!(word, 0b1010, "the two signals are merged");
    let word = value_of(
        &mut fixture,
        request(Syscall::NotificationPoll, &[handle.raw()]),
    );
    assert_eq!(word, 0, "and nothing is left");
}

#[test]
fn signalling_zero_succeeds_and_changes_nothing() {
    let mut fixture = Fixture::new();
    let (id, handle) = notification(&mut fixture);
    assert!(
        error_of(
            &mut fixture,
            request(Syscall::NotificationSignal, &[handle.raw(), 0])
        )
        .is_none()
    );
    assert_eq!(fixture.objects.notifications.get(id).unwrap().word, 0);
}

#[test]
fn a_wait_on_nothing_blocks_and_a_signal_wakes_it_with_the_word() {
    let mut fixture = Fixture::new();
    let (_id, handle) = notification(&mut fixture);
    // A thread of its own waits, so that the caller here can signal.
    let waiter = fixture.running(fixture.process, 4);
    let mut buffer = request(Syscall::NotificationWait, &[handle.raw()]);
    let before = audhsos_abi::ipc_buffer::Buffer::new(&buffer)
        .status()
        .unwrap();
    fixture.write_buffer(waiter, &buffer);
    crate::dispatch::dispatch(&mut fixture.machine(), waiter, &mut buffer);
    assert_eq!(
        fixture.objects.threads.get(waiter).unwrap().state,
        ThreadState::BlockedNotification
    );
    assert_eq!(fixture.status_of(waiter), before);

    let (status, _, _) = call(
        &mut fixture,
        &mut request(Syscall::NotificationSignal, &[handle.raw(), 0b100]),
    );
    assert_eq!(status.error(), None);
    assert_eq!(
        fixture.objects.threads.get(waiter).unwrap().state,
        ThreadState::Ready
    );
    assert_eq!(fixture.returns_of(waiter), [0b100, 0]);
    assert_eq!(fixture.status_of(waiter).error(), None);
}

#[test]
fn a_second_waiter_is_told_that_the_notification_is_busy() {
    let mut fixture = Fixture::new();
    let (_id, handle) = notification(&mut fixture);
    let waiter = fixture.running(fixture.process, 4);
    let mut buffer = request(Syscall::NotificationWait, &[handle.raw()]);
    fixture.write_buffer(waiter, &buffer);
    crate::dispatch::dispatch(&mut fixture.machine(), waiter, &mut buffer);
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::NotificationWait, &[handle.raw()])
        ),
        Some(Error::Busy)
    );
}

#[test]
fn a_notification_needs_the_right_the_operation_asks_for() {
    let mut fixture = Fixture::new();
    let (id, _handle) = notification(&mut fixture);
    let weak = fixture.install(AnyObjectId::of(id), Rights::WAIT).raw();
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::NotificationSignal, &[weak, 1])
        ),
        Some(Error::AccessDenied)
    );
    let other = fixture.install(AnyObjectId::of(id), Rights::SIGNAL).raw();
    assert_eq!(
        error_of(&mut fixture, request(Syscall::NotificationWait, &[other])),
        Some(Error::AccessDenied)
    );
    assert_eq!(
        error_of(&mut fixture, request(Syscall::NotificationPoll, &[other])),
        Some(Error::AccessDenied)
    );
}

#[test]
fn the_last_handle_to_a_notification_wakes_its_waiter() {
    let mut fixture = Fixture::new();
    let (id, handle) = notification(&mut fixture);
    let waiter = fixture.running(fixture.process, 4);
    let mut buffer = request(Syscall::NotificationWait, &[handle.raw()]);
    fixture.write_buffer(waiter, &buffer);
    crate::dispatch::dispatch(&mut fixture.machine(), waiter, &mut buffer);

    let (status, _, _) = call(
        &mut fixture,
        &mut request(Syscall::HandleClose, &[handle.raw()]),
    );
    assert_eq!(status.error(), None);
    assert!(fixture.objects.notifications.get(id).is_err());
    assert_eq!(
        fixture.status_of(waiter).error(),
        Some(Error::ObjectDestroyed)
    );
    assert_eq!(
        fixture.objects.threads.get(waiter).unwrap().state,
        ThreadState::Ready
    );
}
