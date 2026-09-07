// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `process_watch` and of what the end of a process signals.

use audhsos_abi::layout::WATCHERS_PER_PROCESS;
use audhsos_abi::{Error, Handle, Rights, Syscall, ThreadState};
use kernel_objects::object::{AnyObjectId, Notification, NotificationId, ProcessId};

use super::double::{Fixture, error_of, request, value_of};

/// A notification of the calling process and the handle to it.
fn notification(fixture: &mut Fixture) -> (NotificationId, Handle) {
    let raw = value_of(fixture, request(Syscall::NotificationCreate, &[]));
    let handle = Handle::from_raw(raw).unwrap();
    let entry = fixture.objects.entry(fixture.process, handle).unwrap();
    (entry.object.typed::<Notification>().unwrap(), handle)
}

/// A second process with one thread, and a handle to it that carries
/// `rights`.
fn watched(fixture: &mut Fixture, rights: Rights) -> (ProcessId, Handle) {
    let process = fixture.process(8);
    let _thread = fixture.add_thread(process, 4);
    let handle = fixture.install(AnyObjectId::of(process), rights);
    (process, handle)
}

/// A handle to `process` that may end it, which the tests below do to make
/// the end happen.
fn killer(fixture: &mut Fixture, process: ProcessId) -> Handle {
    fixture.install(AnyObjectId::of(process), Rights::MANAGE)
}

/// The word of a notification.
fn word(fixture: &Fixture, id: NotificationId) -> u64 {
    fixture.objects.notifications.get(id).unwrap().word
}

#[test]
fn the_end_of_a_watched_process_signals_the_bit_it_was_watched_on() {
    let mut fixture = Fixture::new();
    let (id, note) = notification(&mut fixture);
    let (process, handle) = watched(&mut fixture, Rights::INFO);
    value_of(
        &mut fixture,
        request(Syscall::ProcessWatch, &[handle.raw(), note.raw(), 3]),
    );
    assert_eq!(word(&fixture, id), 0, "nothing has happened yet");
    let end = killer(&mut fixture, process);
    value_of(&mut fixture, request(Syscall::ProcessKill, &[end.raw()]));
    assert_eq!(word(&fixture, id), 1 << 3);
}

#[test]
fn the_last_thread_of_a_process_that_ends_signals_the_watchers() {
    let mut fixture = Fixture::new();
    let (id, note) = notification(&mut fixture);
    let process = fixture.process(8);
    let first = fixture.add_thread(process, 4);
    let second = fixture.add_thread(process, 4);
    let handle = fixture.install(AnyObjectId::of(process), Rights::INFO);
    value_of(
        &mut fixture,
        request(Syscall::ProcessWatch, &[handle.raw(), note.raw(), 0]),
    );
    let first_handle = fixture.install(AnyObjectId::of(first), Rights::MANAGE);
    value_of(
        &mut fixture,
        request(Syscall::ThreadKill, &[first_handle.raw()]),
    );
    assert_eq!(word(&fixture, id), 0, "one thread of it is still there");
    let second_handle = fixture.install(AnyObjectId::of(second), Rights::MANAGE);
    value_of(
        &mut fixture,
        request(Syscall::ThreadKill, &[second_handle.raw()]),
    );
    assert_eq!(
        word(&fixture, id),
        1,
        "the last one took the process with it"
    );
}

#[test]
fn a_process_that_has_already_ended_signals_at_once() {
    let mut fixture = Fixture::new();
    let (id, note) = notification(&mut fixture);
    let process = fixture.process(8);
    let thread = fixture.add_thread(process, 4);
    let handle = fixture.install(AnyObjectId::of(process), Rights::INFO);
    // The program ends: its last thread is gone, and nobody was watching.
    let thread_handle = fixture.install(AnyObjectId::of(thread), Rights::MANAGE);
    value_of(
        &mut fixture,
        request(Syscall::ThreadKill, &[thread_handle.raw()]),
    );
    assert_eq!(word(&fixture, id), 0, "nobody was watching it");
    // A watch placed now is answered by the fact itself, which is what
    // keeps a server from waiting for an end that has already happened.
    value_of(
        &mut fixture,
        request(Syscall::ProcessWatch, &[handle.raw(), note.raw(), 5]),
    );
    assert_eq!(word(&fixture, id), 1 << 5);
}

#[test]
fn a_process_ends_once_and_is_told_of_once() {
    let mut fixture = Fixture::new();
    let (id, note) = notification(&mut fixture);
    let (process, handle) = watched(&mut fixture, Rights::INFO);
    value_of(
        &mut fixture,
        request(Syscall::ProcessWatch, &[handle.raw(), note.raw(), 1]),
    );
    let end = killer(&mut fixture, process);
    value_of(&mut fixture, request(Syscall::ProcessKill, &[end.raw()]));
    assert_eq!(word(&fixture, id), 1 << 1);
    fixture.objects.notifications.get_mut(id).unwrap().word = 0;
    // Whatever walks past the fact afterwards says nothing a second time.
    let _ignored = crate::watch::ended(&mut fixture.machine(), process);
    assert_eq!(word(&fixture, id), 0);
}

#[test]
fn every_watcher_of_a_process_hears_of_its_end() {
    let mut fixture = Fixture::new();
    let (process, handle) = watched(&mut fixture, Rights::INFO);
    let mut notifications = Vec::new();
    for bit in 0..WATCHERS_PER_PROCESS {
        let (id, note) = notification(&mut fixture);
        value_of(
            &mut fixture,
            request(
                Syscall::ProcessWatch,
                &[handle.raw(), note.raw(), u64::try_from(bit).unwrap()],
            ),
        );
        notifications.push((id, bit));
    }
    let (_full, note) = notification(&mut fixture);
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::ProcessWatch, &[handle.raw(), note.raw(), 0])
        ),
        Some(Error::QuotaExceeded),
        "a fifth watcher is refused rather than dropped"
    );
    let end = killer(&mut fixture, process);
    value_of(&mut fixture, request(Syscall::ProcessKill, &[end.raw()]));
    for (id, bit) in notifications {
        assert_eq!(word(&fixture, id), 1 << bit, "watcher of bit {bit}");
    }
}

#[test]
fn the_same_watch_twice_is_refused() {
    let mut fixture = Fixture::new();
    let (_id, note) = notification(&mut fixture);
    let (_process, handle) = watched(&mut fixture, Rights::INFO);
    value_of(
        &mut fixture,
        request(Syscall::ProcessWatch, &[handle.raw(), note.raw(), 2]),
    );
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::ProcessWatch, &[handle.raw(), note.raw(), 2])
        ),
        Some(Error::AlreadyExists)
    );
    // Another bit of the same notification is another watch.
    value_of(
        &mut fixture,
        request(Syscall::ProcessWatch, &[handle.raw(), note.raw(), 3]),
    );
}

#[test]
fn a_watch_needs_the_right_to_learn_and_the_right_to_bind() {
    let mut fixture = Fixture::new();
    let (_id, note) = notification(&mut fixture);
    let (_process, without) = watched(&mut fixture, Rights::MANAGE);
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::ProcessWatch, &[without.raw(), note.raw(), 0])
        ),
        Some(Error::AccessDenied),
        "a process handle that says nothing may not be watched through"
    );
    let (_second, handle) = watched(&mut fixture, Rights::INFO);
    let (id, _note) = notification(&mut fixture);
    let unbindable = fixture.install(AnyObjectId::of(id), Rights::SIGNAL);
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::ProcessWatch, &[handle.raw(), unbindable.raw(), 0])
        ),
        Some(Error::AccessDenied),
        "a notification that may not be bound may not be watched with"
    );
}

#[test]
fn a_bit_above_the_sixty_four_of_a_notification_is_refused() {
    let mut fixture = Fixture::new();
    let (_id, note) = notification(&mut fixture);
    let (_process, handle) = watched(&mut fixture, Rights::INFO);
    for bit in [64, 255, 256, u64::MAX] {
        assert_eq!(
            error_of(
                &mut fixture,
                request(Syscall::ProcessWatch, &[handle.raw(), note.raw(), bit])
            ),
            Some(Error::InvalidArgument),
            "bit {bit}"
        );
    }
}

#[test]
fn a_handle_that_names_nothing_is_refused_on_either_side() {
    let mut fixture = Fixture::new();
    let (_id, note) = notification(&mut fixture);
    let (_process, handle) = watched(&mut fixture, Rights::INFO);
    let nothing = Handle::new(4000, 1).unwrap();
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::ProcessWatch, &[nothing.raw(), note.raw(), 0])
        ),
        Some(Error::InvalidHandle)
    );
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::ProcessWatch, &[handle.raw(), nothing.raw(), 0])
        ),
        Some(Error::InvalidHandle)
    );
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::ProcessWatch, &[handle.raw(), 0, 0])
        ),
        Some(Error::InvalidHandle),
        "a word that is no handle at all"
    );
}

#[test]
fn a_notification_that_is_gone_when_the_process_ends_signals_nothing() {
    let mut fixture = Fixture::new();
    let (_id, note) = notification(&mut fixture);
    let (process, handle) = watched(&mut fixture, Rights::INFO);
    value_of(
        &mut fixture,
        request(Syscall::ProcessWatch, &[handle.raw(), note.raw(), 0]),
    );
    value_of(&mut fixture, request(Syscall::HandleClose, &[note.raw()]));
    // The process ends with nobody to tell, and that is not an error.
    let end = killer(&mut fixture, process);
    value_of(&mut fixture, request(Syscall::ProcessKill, &[end.raw()]));
}

#[test]
fn a_thread_waiting_on_the_notification_wakes_with_the_bit() {
    let mut fixture = Fixture::new();
    let (id, note) = notification(&mut fixture);
    let (process, handle) = watched(&mut fixture, Rights::INFO);
    value_of(
        &mut fixture,
        request(Syscall::ProcessWatch, &[handle.raw(), note.raw(), 2]),
    );
    let waiter = fixture.running(fixture.process, 4);
    let mut buffer = request(Syscall::NotificationWait, &[note.raw()]);
    fixture.write_buffer(waiter, &buffer);
    crate::dispatch::dispatch(&mut fixture.machine(), waiter, &mut buffer);
    assert_eq!(
        fixture.objects.threads.get(waiter).unwrap().state,
        ThreadState::BlockedNotification
    );
    let end = killer(&mut fixture, process);
    value_of(&mut fixture, request(Syscall::ProcessKill, &[end.raw()]));
    assert_eq!(
        fixture.objects.threads.get(waiter).unwrap().state,
        ThreadState::Ready,
        "the end of the process woke the thread that waited for it"
    );
    assert_eq!(fixture.returns_of(waiter), [1 << 2, 0]);
    assert_eq!(fixture.status_of(waiter).error(), None);
    assert_eq!(word(&fixture, id), 0, "the waiter took the word with it");
}
