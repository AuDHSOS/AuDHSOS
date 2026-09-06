// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::calls::ipc`, covering the endpoint items of catalog
//! 6.6.8 as a user thread reaches them.

use audhsos_abi::ipc_buffer::{Buffer, BufferMut, KERNEL_LABEL_BASE, SIZE, Status, fault_label};
use audhsos_abi::layout::{MAX_MESSAGE_HANDLES, MAX_MESSAGE_WORDS};
use audhsos_abi::{Error, FaultKind, Handle, ObjectType, Rights, Syscall, ThreadState};
use kernel_objects::object::{AnyObjectId, Endpoint, ThreadId};

use super::double::{Fixture, call, error_of, request, value_of};

/// The badge a badged capability of these tests carries.
const BADGE: u64 = 0x0BAD;

/// A buffer holding a message: a label, `words` payload words counting up
/// from one, and the handles.
fn message(label: u64, words: usize, handles: &[Handle]) -> [u8; SIZE] {
    let mut bytes = [0; SIZE];
    let mut writer = BufferMut::new(&mut bytes);
    writer.set_label(label);
    writer.set_counts(words, handles.len()).unwrap();
    for index in 0..words {
        writer.set_word(index, u64::try_from(index).unwrap().saturating_add(1));
    }
    for (index, handle) in handles.iter().enumerate() {
        writer.set_handle(index, *handle);
    }
    bytes
}

/// Puts a call and its arguments in front of a message already in `bytes`.
fn with_call(bytes: &mut [u8; SIZE], syscall: Syscall, arguments: &[u64]) {
    let mut writer = BufferMut::new(bytes);
    writer.set_syscall_number(u64::from(syscall.number()));
    for (index, value) in arguments.iter().enumerate() {
        assert!(writer.set_argument(index, *value));
    }
}

/// An endpoint of the calling process and the handle to it.
fn endpoint(fixture: &mut Fixture) -> (kernel_objects::object::EndpointId, Handle) {
    let raw = value_of(fixture, request(Syscall::EndpointCreate, &[]));
    let handle = Handle::from_raw(raw).unwrap();
    let entry = fixture.objects.entry(fixture.process, handle).unwrap();
    (entry.object.typed::<Endpoint>().unwrap(), handle)
}

#[test]
fn an_endpoint_comes_with_the_rights_of_an_endpoint_and_one_reference() {
    let mut fixture = Fixture::new();
    let (id, handle) = endpoint(&mut fixture);
    let entry = fixture.objects.entry(fixture.process, handle).unwrap();
    assert_eq!(entry.object_type(), ObjectType::Endpoint);
    assert_eq!(
        entry.rights,
        Rights::SEND | Rights::RECV | Rights::BADGE | Rights::DUPLICATE | Rights::TRANSFER
    );
    assert_eq!(entry.badge, 0);
    assert_eq!(fixture.objects.endpoints.references(id), Ok(1));
    assert_eq!(
        fixture
            .objects
            .processes
            .get(fixture.process)
            .unwrap()
            .kernel_object_quota
            .used(),
        1
    );
}

#[test]
fn an_endpoint_that_has_nowhere_to_go_leaves_no_slot_and_no_quota_behind() {
    let mut fixture = Fixture::new();
    fixture.fill_handles();
    assert_eq!(
        error_of(&mut fixture, request(Syscall::EndpointCreate, &[])),
        Some(Error::QuotaExceeded)
    );
    assert_eq!(fixture.objects.endpoints.live(), 0);
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
fn a_badged_capability_carries_send_alone_and_cannot_be_badged_again() {
    let mut fixture = Fixture::new();
    let (id, handle) = endpoint(&mut fixture);
    let badged = value_of(
        &mut fixture,
        request(Syscall::EndpointBadge, &[handle.raw(), BADGE]),
    );
    let badged = Handle::from_raw(badged).unwrap();
    let entry = fixture.objects.entry(fixture.process, badged).unwrap();
    assert_eq!(entry.badge, BADGE);
    assert!(!entry.rights.contains(Rights::RECV));
    assert!(!entry.rights.contains(Rights::BADGE));
    assert!(entry.rights.contains(Rights::SEND));
    assert_eq!(
        fixture.objects.endpoints.references(id),
        Ok(2),
        "the second handle is a second reference"
    );
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::EndpointBadge, &[badged.raw(), BADGE])
        ),
        Some(Error::AccessDenied),
        "the rights check refuses a second badging"
    );
}

#[test]
fn a_badge_of_zero_is_no_badge() {
    let mut fixture = Fixture::new();
    let (_id, handle) = endpoint(&mut fixture);
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::EndpointBadge, &[handle.raw(), 0])
        ),
        Some(Error::InvalidArgument)
    );
}

#[test]
fn a_badged_capability_keeps_its_badge_through_a_duplicate() {
    let mut fixture = Fixture::new();
    let (_id, handle) = endpoint(&mut fixture);
    let badged = Handle::from_raw(value_of(
        &mut fixture,
        request(Syscall::EndpointBadge, &[handle.raw(), BADGE]),
    ))
    .unwrap();
    let rights = u64::from((Rights::SEND | Rights::DUPLICATE).bits());
    let copy = Handle::from_raw(value_of(
        &mut fixture,
        request(Syscall::HandleDuplicate, &[badged.raw(), rights]),
    ))
    .unwrap();
    let entry = fixture.objects.entry(fixture.process, copy).unwrap();
    assert_eq!(entry.badge, BADGE);
    assert!(!entry.rights.contains(Rights::BADGE));
}

#[test]
fn a_call_with_no_receiver_writes_nothing_into_the_buffer_of_its_caller() {
    let mut fixture = Fixture::new();
    let (_id, handle) = endpoint(&mut fixture);
    let mut buffer = message(1, 2, &[]);
    with_call(&mut buffer, Syscall::IpcCall, &[handle.raw()]);
    let before = Buffer::new(&buffer).status().unwrap();
    let (status, values, outcome) = call(&mut fixture, &mut buffer);
    assert_eq!(status, before, "the status word is untouched");
    assert_eq!(values, [0, 0]);
    assert!(outcome.reschedule);
    assert_eq!(
        fixture.objects.threads.get(fixture.thread).unwrap().state,
        ThreadState::BlockedSend
    );
}

#[test]
fn a_send_that_meets_a_receiver_carries_the_words_the_badge_and_the_handles() {
    let mut fixture = Fixture::new();
    let (endpoint_id, handle) = endpoint(&mut fixture);
    let badged = Handle::from_raw(value_of(
        &mut fixture,
        request(Syscall::EndpointBadge, &[handle.raw(), BADGE]),
    ))
    .unwrap();
    // A server in a process of its own, waiting on the endpoint.
    let server = fixture.process(8);
    let receiver = fixture.running(server, 4);
    let theirs = fixture
        .objects
        .install_handle(
            server,
            kernel_objects::handle_table::Entry::new(AnyObjectId::of(endpoint_id), Rights::RECV),
        )
        .unwrap();
    fixture
        .objects
        .retain(AnyObjectId::of(endpoint_id))
        .unwrap();
    receive_on(&mut fixture, receiver, theirs);

    let (memory, memory_handle) = fixture.memory(0x40, 1, Rights::READ | Rights::TRANSFER);
    let mut buffer = message(9, 3, &[memory_handle]);
    with_call(&mut buffer, Syscall::IpcSend, &[badged.raw()]);
    let (status, _values, _) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), None);

    assert_eq!(fixture.status_of(receiver), Status::OK);
    assert_eq!(
        fixture.returns_of(receiver),
        [BADGE, 0],
        "the badge, no reply"
    );
    let theirs_buffer = *fixture.buffer_of(receiver);
    let view = Buffer::new(&theirs_buffer);
    let header = view.message().unwrap();
    assert_eq!(header.label, 9);
    assert_eq!(header.word_count, 3);
    assert_eq!(header.handle_count, 1);
    assert_eq!(view.word(2), Some(3));
    let arrived = view.handle(0).unwrap();
    let entry = fixture.objects.handles.lookup(server, arrived).unwrap();
    assert_eq!(entry.object, AnyObjectId::of(memory));
    assert_eq!(fixture.objects.memory.references(memory), Ok(2));
    assert_eq!(
        fixture.objects.threads.get(receiver).unwrap().state,
        ThreadState::Ready
    );
}

#[test]
fn a_call_that_meets_a_receiver_hands_it_a_reply_handle_and_blocks_the_caller() {
    let mut fixture = Fixture::new();
    let (endpoint_id, handle) = endpoint(&mut fixture);
    let server = fixture.process(8);
    let receiver = fixture.running(server, 4);
    let theirs = install_endpoint(&mut fixture, server, endpoint_id, Rights::RECV);
    receive_on(&mut fixture, receiver, theirs);

    let mut buffer = message(1, 1, &[]);
    with_call(&mut buffer, Syscall::IpcCall, &[handle.raw()]);
    let (_status, _values, outcome) = call(&mut fixture, &mut buffer);
    assert!(outcome.reschedule);
    assert_eq!(
        fixture.objects.threads.get(fixture.thread).unwrap().state,
        ThreadState::BlockedReply
    );
    let [badge, reply] = fixture.returns_of(receiver);
    assert_eq!(badge, 0, "an unbadged capability");
    let reply = Handle::from_raw(reply).expect("a reply handle");
    let entry = fixture.objects.handles.lookup(server, reply).unwrap();
    assert_eq!(entry.object_type(), ObjectType::Reply);
    assert_eq!(entry.rights, Rights::EMPTY);
}

#[test]
fn a_message_of_the_widest_payload_arrives_and_one_word_more_is_refused() {
    let mut fixture = Fixture::new();
    let (endpoint_id, handle) = endpoint(&mut fixture);
    let server = fixture.process(8);
    let receiver = fixture.running(server, 4);
    let theirs = install_endpoint(&mut fixture, server, endpoint_id, Rights::RECV);
    receive_on(&mut fixture, receiver, theirs);

    let mut buffer = message(1, MAX_MESSAGE_WORDS, &[]);
    with_call(&mut buffer, Syscall::IpcSend, &[handle.raw()]);
    let (status, _, _) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), None);
    let arrived = *fixture.buffer_of(receiver);
    assert_eq!(
        Buffer::new(&arrived).message().unwrap().word_count,
        MAX_MESSAGE_WORDS
    );

    // One word more, written by hand: the setter refuses the count, and a
    // thread that wrote its own header is what this is about.
    let mut buffer = message(1, 0, &[]);
    write_raw(
        &mut buffer,
        audhsos_abi::ipc_buffer::WORD_COUNT,
        u64::try_from(MAX_MESSAGE_WORDS).unwrap().saturating_add(1),
    );
    with_call(&mut buffer, Syscall::IpcSend, &[handle.raw()]);
    let (status, _, _) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), Some(Error::InvalidArgument));
    assert_eq!(
        fixture.objects.threads.get(fixture.thread).unwrap().state,
        ThreadState::Running,
        "the sender never left the processor"
    );
}

#[test]
fn a_handle_count_above_the_area_is_refused_before_anything_is_copied() {
    let mut fixture = Fixture::new();
    let (_id, handle) = endpoint(&mut fixture);
    let mut buffer = message(1, 0, &[]);
    write_raw(
        &mut buffer,
        audhsos_abi::ipc_buffer::HANDLE_COUNT,
        u64::try_from(MAX_MESSAGE_HANDLES)
            .unwrap()
            .saturating_add(1),
    );
    with_call(&mut buffer, Syscall::IpcSend, &[handle.raw()]);
    let (status, _, _) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), Some(Error::InvalidArgument));
}

#[test]
fn a_message_with_the_label_the_kernel_keeps_is_refused_before_anything_is_copied() {
    let mut fixture = Fixture::new();
    let (endpoint_id, handle) = endpoint(&mut fixture);
    let server = fixture.process(8);
    let receiver = fixture.running(server, 4);
    let theirs = install_endpoint(&mut fixture, server, endpoint_id, Rights::RECV);
    receive_on(&mut fixture, receiver, theirs);

    for label in [
        KERNEL_LABEL_BASE,
        fault_label(FaultKind::PageFault),
        u64::MAX,
    ] {
        let mut buffer = message(label, 1, &[]);
        with_call(&mut buffer, Syscall::IpcSend, &[handle.raw()]);
        let (status, _, _) = call(&mut fixture, &mut buffer);
        assert_eq!(
            status.error(),
            Some(Error::InvalidArgument),
            "label {label:#x}"
        );
        assert_eq!(
            Buffer::new(fixture.buffer_of(receiver))
                .message()
                .unwrap()
                .label,
            0,
            "nothing was copied"
        );
    }
    // One below the base is a label of the sender's own.
    let mut buffer = message(KERNEL_LABEL_BASE - 1, 1, &[]);
    with_call(&mut buffer, Syscall::IpcSend, &[handle.raw()]);
    let (status, _, _) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), None);
}

#[test]
fn a_handle_without_transfer_refuses_the_message_and_installs_no_other() {
    let mut fixture = Fixture::new();
    let (endpoint_id, handle) = endpoint(&mut fixture);
    let server = fixture.process(8);
    let receiver = fixture.running(server, 4);
    let theirs = install_endpoint(&mut fixture, server, endpoint_id, Rights::RECV);
    receive_on(&mut fixture, receiver, theirs);
    let before = fixture
        .objects
        .processes
        .get(server)
        .unwrap()
        .handles
        .count();

    let (_travels, first) = fixture.memory(0x50, 1, Rights::READ | Rights::TRANSFER);
    let (_stays, second) = fixture.memory(0x60, 1, Rights::READ);
    let mut buffer = message(1, 1, &[first, second]);
    with_call(&mut buffer, Syscall::IpcSend, &[handle.raw()]);
    let (status, _, _) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), Some(Error::AccessDenied));
    assert_eq!(
        fixture
            .objects
            .processes
            .get(server)
            .unwrap()
            .handles
            .count(),
        before,
        "not one handle of the message arrived"
    );
    assert_eq!(
        fixture.objects.threads.get(receiver).unwrap().state,
        ThreadState::BlockedRecv,
        "the receiver went back into its queue"
    );
}

#[test]
fn a_receiver_with_room_for_fewer_handles_gets_what_fits_and_the_partial_flag() {
    let mut fixture = Fixture::new();
    let (endpoint_id, handle) = endpoint(&mut fixture);
    // Two slots: the endpoint handle takes one, so one handle of two fits.
    let server = fixture.process(2);
    let receiver = fixture.running(server, 4);
    let theirs = install_endpoint(&mut fixture, server, endpoint_id, Rights::RECV);
    receive_on(&mut fixture, receiver, theirs);

    let (_first, first) = fixture.memory(0x70, 1, Rights::READ | Rights::TRANSFER);
    let (second_id, second) = fixture.memory(0x80, 1, Rights::READ | Rights::TRANSFER);
    let mut buffer = message(1, 1, &[first, second]);
    with_call(&mut buffer, Syscall::IpcSend, &[handle.raw()]);
    let (status, _, _) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), None, "the sender succeeded");
    assert!(!status.is_partial(), "what did not fit is not its business");
    assert!(
        fixture.status_of(receiver).is_partial(),
        "the receiver carries the flag"
    );
    let arrived = *fixture.buffer_of(receiver);
    assert_eq!(Buffer::new(&arrived).message().unwrap().handle_count, 1);
    assert_eq!(fixture.objects.memory.references(second_id), Ok(1));
}

#[test]
fn try_recv_on_an_empty_endpoint_answers_would_block() {
    let mut fixture = Fixture::new();
    let (_id, handle) = endpoint(&mut fixture);
    assert_eq!(
        error_of(&mut fixture, request(Syscall::IpcTryRecv, &[handle.raw()])),
        Some(Error::WouldBlock)
    );
    assert_eq!(
        fixture.objects.threads.get(fixture.thread).unwrap().state,
        ThreadState::Running
    );
}

#[test]
fn a_receive_that_meets_a_queued_call_answers_the_badge_and_the_reply() {
    let mut fixture = Fixture::new();
    let (endpoint_id, handle) = endpoint(&mut fixture);
    // A client of its own that calls and waits, through a badged capability
    // of the endpoint.
    let client = fixture.process(8);
    let caller = fixture.running(client, 4);
    let theirs = install_endpoint(&mut fixture, client, endpoint_id, Rights::SEND);
    fixture
        .objects
        .handles
        .lookup_mut(client, theirs)
        .unwrap()
        .badge = BADGE;
    let mut theirs_buffer = message(7, 2, &[]);
    with_call(&mut theirs_buffer, Syscall::IpcCall, &[theirs.raw()]);
    fixture.write_buffer(caller, &theirs_buffer);
    let outcome = crate::dispatch::dispatch(&mut fixture.machine(), caller, &mut theirs_buffer);
    assert!(outcome.reschedule);
    assert_eq!(
        fixture.objects.threads.get(caller).unwrap().state,
        ThreadState::BlockedSend
    );

    let mut buffer = [0; SIZE];
    with_call(&mut buffer, Syscall::IpcRecv, &[handle.raw()]);
    let (status, values, _) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), None);
    assert_eq!(values[0], BADGE, "the badge waited with the caller");
    let reply = Handle::from_raw(values[1]).expect("a reply handle");
    let view = Buffer::new(&buffer);
    assert_eq!(view.message().unwrap().label, 7);
    assert_eq!(view.word(1), Some(2));
    assert_eq!(
        fixture.objects.threads.get(caller).unwrap().state,
        ThreadState::BlockedReply
    );

    // The reply carries a message back and wakes the caller.
    let mut answer = message(8, 1, &[]);
    with_call(&mut answer, Syscall::IpcReply, &[reply.raw()]);
    let (status, _, _) = call(&mut fixture, &mut answer);
    assert_eq!(status.error(), None);
    assert_eq!(fixture.status_of(caller), Status::OK);
    assert_eq!(
        fixture.objects.threads.get(caller).unwrap().state,
        ThreadState::Ready
    );
    let back = *fixture.buffer_of(caller);
    assert_eq!(Buffer::new(&back).message().unwrap().label, 8);
    assert_eq!(Buffer::new(&back).word(0), Some(1));

    // A second reply through the same object is refused.
    let mut again = message(8, 0, &[]);
    with_call(&mut again, Syscall::IpcReply, &[reply.raw()]);
    let (status, _, _) = call(&mut fixture, &mut again);
    assert_eq!(status.error(), Some(Error::InvalidState));
}

#[test]
fn a_reply_recv_answers_first_and_then_waits_for_the_next_sender() {
    let mut fixture = Fixture::new();
    let (endpoint_id, handle) = endpoint(&mut fixture);
    let client = fixture.process(8);
    let caller = fixture.running(client, 4);
    let theirs = install_endpoint(&mut fixture, client, endpoint_id, Rights::SEND);
    let reply = queue_a_call(&mut fixture, caller, theirs, handle);

    let mut answer = message(3, 1, &[]);
    with_call(
        &mut answer,
        Syscall::IpcReplyRecv,
        &[reply.raw(), handle.raw()],
    );
    let (status, _, outcome) = call(&mut fixture, &mut answer);
    assert_eq!(status, Status::OK, "the receive that followed blocked");
    assert!(outcome.reschedule);
    assert_eq!(
        fixture.objects.threads.get(fixture.thread).unwrap().state,
        ThreadState::BlockedRecv,
        "the reply was delivered and then it waited"
    );
    assert_eq!(fixture.status_of(caller), Status::OK);
    assert_eq!(
        Buffer::new(fixture.buffer_of(caller))
            .message()
            .unwrap()
            .label,
        3
    );
}

#[test]
fn a_reply_recv_whose_reply_fails_does_not_become_a_receive() {
    let mut fixture = Fixture::new();
    let (_id, handle) = endpoint(&mut fixture);
    let stale = Handle::new(0x00FF_FFFF, 1).unwrap();
    let mut buffer = message(1, 0, &[]);
    with_call(
        &mut buffer,
        Syscall::IpcReplyRecv,
        &[stale.raw(), handle.raw()],
    );
    let (status, _, _) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), Some(Error::InvalidHandle));
    assert_eq!(
        fixture.objects.threads.get(fixture.thread).unwrap().state,
        ThreadState::Running
    );
}

#[test]
fn a_reply_object_whose_caller_was_killed_is_refused_without_a_copy() {
    let mut fixture = Fixture::new();
    let (endpoint_id, handle) = endpoint(&mut fixture);
    let client = fixture.process(8);
    let caller = fixture.running(client, 4);
    let theirs = install_endpoint(&mut fixture, client, endpoint_id, Rights::SEND);
    let reply = queue_a_call(&mut fixture, caller, theirs, handle);

    // The caller ends: the wait is cancelled and the reply object answers
    // nothing.
    kernel_ipc::cancel(&mut fixture.objects, caller);
    fixture
        .scheduler
        .exit(&mut fixture.objects.threads, caller)
        .unwrap();
    let mut answer = message(1, 1, &[]);
    with_call(&mut answer, Syscall::IpcReply, &[reply.raw()]);
    let (status, _, _) = call(&mut fixture, &mut answer);
    assert_eq!(status.error(), Some(Error::InvalidState));
    assert_eq!(
        Buffer::new(fixture.buffer_of(caller))
            .message()
            .unwrap()
            .label,
        7,
        "the buffer of the dead caller still holds what it sent"
    );
}

#[test]
fn the_last_handle_to_an_endpoint_wakes_everyone_who_waited_on_it() {
    let mut fixture = Fixture::new();
    let (endpoint_id, handle) = endpoint(&mut fixture);
    let client = fixture.process(8);
    let first = fixture.running(client, 4);
    let second = fixture.running(client, 2);
    let theirs = install_endpoint(&mut fixture, client, endpoint_id, Rights::SEND);
    for sender in [first, second] {
        let mut buffer = message(1, 0, &[]);
        with_call(&mut buffer, Syscall::IpcSend, &[theirs.raw()]);
        fixture.write_buffer(sender, &buffer);
        crate::dispatch::dispatch(&mut fixture.machine(), sender, &mut buffer);
        assert_eq!(
            fixture.objects.threads.get(sender).unwrap().state,
            ThreadState::BlockedSend
        );
    }
    // The client closes its handle and the server closes its own: the last
    // one destroys the endpoint under the two waiters (D-75).
    fixture.objects.close_handle(client, theirs).unwrap();
    let switch =
        crate::lifetime::release(&mut fixture.machine(), AnyObjectId::of(endpoint_id)).unwrap();
    assert!(!switch, "one reference is left");
    let (status, _, _) = call(
        &mut fixture,
        &mut request(Syscall::HandleClose, &[handle.raw()]),
    );
    assert_eq!(status.error(), None);
    assert!(fixture.objects.endpoints.get(endpoint_id).is_err());
    for sender in [first, second] {
        assert_eq!(
            fixture.objects.threads.get(sender).unwrap().state,
            ThreadState::Ready
        );
        assert_eq!(
            fixture.status_of(sender).error(),
            Some(Error::ObjectDestroyed)
        );
    }
}

#[test]
fn a_thread_suspended_while_it_waits_finds_that_it_was_cancelled() {
    let mut fixture = Fixture::new();
    let (endpoint_id, _handle) = endpoint(&mut fixture);
    let client = fixture.process(8);
    let waiting = fixture.running(client, 4);
    let theirs = install_endpoint(&mut fixture, client, endpoint_id, Rights::SEND);
    let mut buffer = message(1, 0, &[]);
    with_call(&mut buffer, Syscall::IpcSend, &[theirs.raw()]);
    fixture.write_buffer(waiting, &buffer);
    crate::dispatch::dispatch(&mut fixture.machine(), waiting, &mut buffer);

    let handle = fixture
        .install(AnyObjectId::of(waiting), Rights::MANAGE)
        .raw();
    let (status, _, _) = call(
        &mut fixture,
        &mut request(Syscall::ThreadSuspend, &[handle]),
    );
    assert_eq!(status.error(), None);
    assert_eq!(
        fixture.objects.threads.get(waiting).unwrap().state,
        ThreadState::Suspended
    );
    assert_eq!(
        fixture.status_of(waiting).error(),
        Some(Error::Cancelled),
        "the answer it waited for never came"
    );
    assert_eq!(
        fixture.objects.threads.get(waiting).unwrap().wait,
        kernel_objects::object::Wait::Nothing
    );
    assert!(
        fixture
            .objects
            .endpoints
            .get(endpoint_id)
            .unwrap()
            .senders
            .is_empty()
    );
}

#[test]
fn a_transfer_between_two_threads_of_one_process_needs_two_buffers() {
    // Two threads never share an IPC buffer, so a transfer whose two frames
    // are one frame is a check on the argument. It is reached by giving a
    // receiver the buffer of the sender.
    let mut fixture = Fixture::new();
    let (endpoint_id, handle) = endpoint(&mut fixture);
    let own = fixture.process;
    let other = fixture.running(own, 4);
    let sharing = fixture
        .objects
        .threads
        .get(fixture.thread)
        .unwrap()
        .ipc_buffer;
    fixture
        .objects
        .threads
        .with(other, |thread| thread.ipc_buffer = sharing);
    let theirs = install_endpoint(&mut fixture, own, endpoint_id, Rights::RECV);
    receive_on(&mut fixture, other, theirs);

    let mut buffer = message(1, 0, &[]);
    with_call(&mut buffer, Syscall::IpcSend, &[handle.raw()]);
    let (status, _, _) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), Some(Error::InvalidArgument));
    assert_eq!(
        fixture.objects.threads.get(other).unwrap().state,
        ThreadState::BlockedRecv,
        "the receiver is back in its queue"
    );
}

/// Installs a handle to `endpoint` with `rights` in `process` and counts the
/// reference it holds.
fn install_endpoint(
    fixture: &mut Fixture,
    process: kernel_objects::object::ProcessId,
    endpoint: kernel_objects::object::EndpointId,
    rights: Rights,
) -> Handle {
    let handle = fixture
        .objects
        .install_handle(
            process,
            kernel_objects::handle_table::Entry::new(AnyObjectId::of(endpoint), rights),
        )
        .unwrap();
    fixture.objects.retain(AnyObjectId::of(endpoint)).unwrap();
    handle
}

/// Makes `thread` wait for a sender on the endpoint `handle` names.
fn receive_on(fixture: &mut Fixture, thread: ThreadId, handle: Handle) {
    let mut buffer = [0; SIZE];
    with_call(&mut buffer, Syscall::IpcRecv, &[handle.raw()]);
    fixture.write_buffer(thread, &buffer);
    crate::dispatch::dispatch(&mut fixture.machine(), thread, &mut buffer);
    assert_eq!(
        fixture.objects.threads.get(thread).unwrap().state,
        ThreadState::BlockedRecv
    );
}

/// Lets `caller` make a call that queues, then receives it here and returns
/// the reply handle.
fn queue_a_call(fixture: &mut Fixture, caller: ThreadId, theirs: Handle, ours: Handle) -> Handle {
    let mut buffer = message(7, 1, &[]);
    with_call(&mut buffer, Syscall::IpcCall, &[theirs.raw()]);
    fixture.write_buffer(caller, &buffer);
    crate::dispatch::dispatch(&mut fixture.machine(), caller, &mut buffer);
    let mut ours_buffer = [0; SIZE];
    with_call(&mut ours_buffer, Syscall::IpcRecv, &[ours.raw()]);
    let (status, values, _) = call(fixture, &mut ours_buffer);
    assert_eq!(status.error(), None);
    Handle::from_raw(values[1]).expect("a reply handle")
}

/// Writes a raw word at `offset`, for the headers a setter refuses.
fn write_raw(bytes: &mut [u8; SIZE], offset: usize, value: u64) {
    let end = offset.saturating_add(8);
    bytes
        .get_mut(offset..end)
        .unwrap()
        .copy_from_slice(&value.to_le_bytes());
}

#[test]
fn an_endpoint_the_machine_has_no_slot_for_leaves_the_quota_as_it_was() {
    let mut fixture = Fixture::new();
    // Every slot of the pool is taken; the pool takes its size from the
    // configuration and not from a parameter of the machine (D-74's
    // neighbour in 10.6.0), so this is the real number.
    while fixture.objects.endpoints.allocate(Endpoint::new()).is_ok() {}
    assert_eq!(
        error_of(&mut fixture, request(Syscall::EndpointCreate, &[])),
        Some(Error::PoolExhausted)
    );
    assert_eq!(
        fixture
            .objects
            .processes
            .get(fixture.process)
            .unwrap()
            .kernel_object_quota
            .used(),
        0,
        "what the call charged, it gave back"
    );
}

#[test]
fn a_badged_capability_that_has_nowhere_to_go_takes_no_second_reference() {
    let mut fixture = Fixture::new();
    let (id, handle) = endpoint(&mut fixture);
    fixture.fill_handles();
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::EndpointBadge, &[handle.raw(), BADGE])
        ),
        Some(Error::QuotaExceeded)
    );
    assert_eq!(
        fixture.objects.endpoints.references(id),
        Ok(1),
        "the one handle is the one reference"
    );
}

#[test]
fn a_call_whose_message_is_refused_takes_back_the_reply_object_it_opened() {
    let mut fixture = Fixture::new();
    let (endpoint_id, handle) = endpoint(&mut fixture);
    let server = fixture.process(8);
    let receiver = fixture.running(server, 4);
    let theirs = install_endpoint(&mut fixture, server, endpoint_id, Rights::RECV);
    receive_on(&mut fixture, receiver, theirs);
    let before = fixture
        .objects
        .processes
        .get(server)
        .unwrap()
        .handles
        .count();

    let (_stays, refused) = fixture.memory(0x90, 1, Rights::READ);
    let mut buffer = message(1, 1, &[refused]);
    with_call(&mut buffer, Syscall::IpcCall, &[handle.raw()]);
    let (status, _, _) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), Some(Error::AccessDenied));
    assert_eq!(
        fixture.objects.replies.live(),
        0,
        "the reply object went with the message"
    );
    assert_eq!(
        fixture
            .objects
            .processes
            .get(server)
            .unwrap()
            .handles
            .count(),
        before,
        "and so did the handle to it"
    );
    assert_eq!(
        fixture.objects.threads.get(receiver).unwrap().state,
        ThreadState::BlockedRecv,
        "the receiver is back in its queue"
    );
    assert_eq!(
        fixture.objects.threads.get(fixture.thread).unwrap().state,
        ThreadState::Running,
        "and the caller never left the processor"
    );
}

#[test]
fn a_receive_whose_sender_shares_its_buffer_puts_the_sender_back() {
    // Two threads never share an IPC buffer, so this is a check on the
    // argument and not a state the kernel can reach; it is reached here by
    // giving the queued sender the buffer of the thread that receives.
    let mut fixture = Fixture::new();
    let (endpoint_id, handle) = endpoint(&mut fixture);
    let client = fixture.process(8);
    let sender = fixture.running(client, 4);
    let theirs = install_endpoint(&mut fixture, client, endpoint_id, Rights::SEND);
    fixture
        .objects
        .handles
        .lookup_mut(client, theirs)
        .unwrap()
        .badge = BADGE;
    let mut theirs_buffer = message(1, 1, &[]);
    with_call(&mut theirs_buffer, Syscall::IpcSend, &[theirs.raw()]);
    fixture.write_buffer(sender, &theirs_buffer);
    crate::dispatch::dispatch(&mut fixture.machine(), sender, &mut theirs_buffer);
    assert_eq!(
        fixture.objects.threads.get(sender).unwrap().state,
        ThreadState::BlockedSend
    );
    let sharing = fixture
        .objects
        .threads
        .get(fixture.thread)
        .unwrap()
        .ipc_buffer;
    fixture
        .objects
        .threads
        .with(sender, |thread| thread.ipc_buffer = sharing);

    let mut buffer = [0; SIZE];
    with_call(&mut buffer, Syscall::IpcRecv, &[handle.raw()]);
    let (status, _, _) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), Some(Error::InvalidArgument));
    assert_eq!(
        fixture.objects.threads.get(sender).unwrap().state,
        ThreadState::BlockedSend,
        "the sender is back in the queue it waited in"
    );
    assert_eq!(
        fixture.objects.threads.get(sender).unwrap().wait,
        kernel_objects::object::Wait::Endpoint {
            endpoint: endpoint_id,
            queue: kernel_objects::object::Queue::Senders,
            badge: BADGE
        },
        "with the badge it sent through"
    );
}

#[test]
fn closing_a_reply_handle_without_answering_wakes_the_caller() {
    let mut fixture = Fixture::new();
    let (endpoint_id, handle) = endpoint(&mut fixture);
    let client = fixture.process(8);
    let caller = fixture.running(client, 4);
    let theirs = install_endpoint(&mut fixture, client, endpoint_id, Rights::SEND);
    let reply = queue_a_call(&mut fixture, caller, theirs, handle);
    assert_eq!(
        fixture.objects.threads.get(caller).unwrap().state,
        ThreadState::BlockedReply
    );

    // The server gives up on the call instead of answering it.
    let (status, _, _) = call(
        &mut fixture,
        &mut request(Syscall::HandleClose, &[reply.raw()]),
    );
    assert_eq!(status.error(), None);
    assert_eq!(fixture.objects.replies.live(), 0, "the object went with it");
    assert_eq!(
        fixture.objects.threads.get(caller).unwrap().state,
        ThreadState::Ready
    );
    assert_eq!(
        fixture.status_of(caller).error(),
        Some(Error::ReplyDropped),
        "the caller learns that its answer will never come"
    );
    assert_eq!(
        fixture.objects.threads.get(caller).unwrap().wait,
        kernel_objects::object::Wait::Nothing
    );
}
