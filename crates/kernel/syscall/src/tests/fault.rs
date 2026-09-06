// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::fault`.

use audhsos_abi::ipc_buffer::{Buffer, SIZE, fault_kind_of};
use audhsos_abi::{Error, Fault, FaultKind, Handle, Rights, Syscall, ThreadState};
use kernel_objects::object::{AnyObjectId, Endpoint, EndpointId};

use crate::fault::{deliver, is_faulted, stop};
use crate::reaper::has_work;
use crate::tests::double::{Fixture, call, error_of, request, value_of};

#[test]
fn a_thread_that_faults_stops_and_leaves_the_processor() {
    let mut fixture = Fixture::new();
    let id = fixture.thread;
    assert!(!is_faulted(&fixture.machine(), id));

    let outcome = stop(&mut fixture.machine(), id);

    assert!(outcome.reschedule, "the kernel cannot return to it");
    assert!(is_faulted(&fixture.machine(), id));
    assert_eq!(
        fixture.objects.threads.get(id).unwrap().state,
        ThreadState::Faulted
    );
    assert_eq!(fixture.scheduler.current(), None);
    assert_eq!(fixture.scheduler.ready_bitmap(), 0, "and no queue holds it");
}

#[test]
fn a_thread_that_faulted_keeps_everything_it_holds() {
    let mut fixture = Fixture::new();
    let creation = request(
        Syscall::ThreadCreate,
        &[fixture.own_process.raw(), 0x40_0000, 0x50_0000, 2, 4, 0],
    );
    let raw = value_of(&mut fixture, creation);
    assert_eq!(fixture.environment.stacks_out(), 1);
    let other = fixture.thread;

    stop(&mut fixture.machine(), other);

    assert_eq!(
        fixture.environment.stacks_out(),
        1,
        "a fault gives no kernel stack back"
    );
    assert!(
        !has_work(&fixture.machine(), None),
        "a faulted thread is not a thread the reaper takes"
    );
    assert!(fixture.objects.threads.get(other).is_ok());
    let _ = raw;
}

#[test]
fn a_faulted_thread_runs_again_when_it_is_resumed() {
    let mut fixture = Fixture::new();
    let id = fixture.thread;
    stop(&mut fixture.machine(), id);

    fixture
        .scheduler
        .resume(&mut fixture.objects.threads, id)
        .unwrap();

    assert!(!is_faulted(&fixture.machine(), id));
    assert_eq!(
        fixture.objects.threads.get(id).unwrap().state,
        ThreadState::Ready
    );
}

#[test]
fn a_faulted_thread_can_be_killed_by_whoever_created_it() {
    let mut fixture = Fixture::new();
    let id = fixture.thread;
    stop(&mut fixture.machine(), id);

    let handle = fixture.own_thread.raw();
    assert!(error_of(&mut fixture, request(Syscall::ThreadKill, &[handle])).is_none());

    assert_eq!(
        fixture.objects.threads.get(id).unwrap().state,
        ThreadState::Exited
    );
    assert!(has_work(&fixture.machine(), None));
}

#[test]
fn a_thread_that_has_ended_cannot_fault() {
    let mut fixture = Fixture::new();
    let id = fixture.thread;
    fixture
        .scheduler
        .exit(&mut fixture.objects.threads, id)
        .unwrap();

    let outcome = stop(&mut fixture.machine(), id);

    assert!(!outcome.reschedule, "it had already left the processor");
    assert!(!is_faulted(&fixture.machine(), id));
    assert_eq!(
        fixture.objects.threads.get(id).unwrap().state,
        ThreadState::Exited,
        "the transition table names no way out of `Exited`"
    );
}

#[test]
fn a_ready_thread_that_is_faulted_stays_ready_and_stays_in_its_queue() {
    let mut fixture = Fixture::new();
    let creation = request(
        Syscall::ThreadCreate,
        &[fixture.own_process.raw(), 0x40_0000, 0x50_0000, 2, 4, 0],
    );
    let raw = value_of(&mut fixture, creation);
    let other = thread_of(&fixture, raw);
    assert!(error_of(&mut fixture, request(Syscall::ThreadStart, &[raw])).is_none());
    let queued = fixture.scheduler.ready_bitmap();
    assert_ne!(queued, 0, "the started thread waits in a queue");

    let outcome = stop(&mut fixture.machine(), other);

    assert!(!outcome.reschedule);
    assert_eq!(
        fixture.objects.threads.get(other).unwrap().state,
        ThreadState::Ready,
        "only a running thread may fault"
    );
    assert_eq!(
        fixture.scheduler.ready_bitmap(),
        queued,
        "and a fault it cannot take must not take it out of its queue"
    );
    // Reachable, not merely ready: once the caller is off the processor
    // the scheduler finds it, which a thread stranded outside every queue
    // would not be.
    let caller = fixture.thread;
    fixture
        .scheduler
        .exit(&mut fixture.objects.threads, caller)
        .unwrap();
    assert_eq!(
        fixture.scheduler.pick_next(&mut fixture.objects.threads),
        Ok(other),
        "the thread the fault could not take is still the one that runs next"
    );
}

/// The thread `raw` names, for a handle the fixture handed out.
fn thread_of(fixture: &Fixture, raw: u64) -> kernel_objects::object::ThreadId {
    fixture
        .objects
        .entry(fixture.process, audhsos_abi::Handle::from_raw(raw).unwrap())
        .unwrap()
        .object
        .typed::<kernel_objects::object::Thread>()
        .unwrap()
}

#[test]
fn a_fault_of_a_thread_that_is_not_running_asks_for_no_switch() {
    let mut fixture = Fixture::new();
    let creation = request(
        Syscall::ThreadCreate,
        &[fixture.own_process.raw(), 0x40_0000, 0x50_0000, 2, 4, 0],
    );
    let raw = value_of(&mut fixture, creation);
    let other = thread_of(&fixture, raw);
    // A thread that was never started is `Inactive`, which the table
    // gives no fault transition: nothing happens to it and the thread on
    // the processor keeps running.
    let running = fixture.scheduler.current();

    let outcome = stop(&mut fixture.machine(), other);

    assert!(!outcome.reschedule);
    assert!(!is_faulted(&fixture.machine(), other));
    assert_eq!(fixture.scheduler.current(), running);
}

/// The fault every test of the delivery reports.
fn page_fault() -> Fault {
    Fault {
        kind: FaultKind::PageFault,
        address: 0x0DE_AD000,
        instruction_pointer: 0x40_1234,
        error_code: 0b110,
    }
}

/// An endpoint of the calling process, named as the fault handler of its
/// own process, and the handle to it.
fn handler(fixture: &mut Fixture) -> (EndpointId, Handle) {
    let raw = value_of(fixture, request(Syscall::EndpointCreate, &[]));
    let handle = Handle::from_raw(raw).unwrap();
    let id = fixture
        .objects
        .entry(fixture.process, handle)
        .unwrap()
        .object
        .typed::<Endpoint>()
        .unwrap();
    let own = fixture.own_process.raw();
    assert!(
        error_of(
            fixture,
            request(Syscall::ProcessSetFaultHandler, &[own, raw])
        )
        .is_none()
    );
    (id, handle)
}

#[test]
fn a_fault_handler_is_retained_replaced_and_cleared() {
    let mut fixture = Fixture::new();
    let (first, _handle) = handler(&mut fixture);
    assert_eq!(
        fixture
            .objects
            .processes
            .get(fixture.process)
            .unwrap()
            .fault_handler
            .map(|(endpoint, _badge)| endpoint),
        Some(first)
    );
    assert_eq!(
        fixture.objects.endpoints.references(first),
        Ok(2),
        "the handle and the handler are two references"
    );

    let second_raw = value_of(&mut fixture, request(Syscall::EndpointCreate, &[]));
    let own = fixture.own_process.raw();
    assert!(
        error_of(
            &mut fixture,
            request(Syscall::ProcessSetFaultHandler, &[own, second_raw])
        )
        .is_none()
    );
    assert_eq!(
        fixture.objects.endpoints.references(first),
        Ok(1),
        "the one it replaced gave its reference back"
    );

    assert!(
        error_of(
            &mut fixture,
            request(Syscall::ProcessSetFaultHandler, &[own, 0])
        )
        .is_none()
    );
    assert_eq!(
        fixture
            .objects
            .processes
            .get(fixture.process)
            .unwrap()
            .fault_handler,
        None
    );
}

#[test]
fn a_fault_handler_has_to_be_an_endpoint_the_caller_may_send_on() {
    let mut fixture = Fixture::new();
    let own = fixture.own_process.raw();
    let wrong = fixture.own_thread.raw();
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::ProcessSetFaultHandler, &[own, wrong])
        ),
        Some(Error::WrongObjectType)
    );
    let raw = value_of(&mut fixture, request(Syscall::EndpointCreate, &[]));
    let id = fixture
        .objects
        .entry(fixture.process, Handle::from_raw(raw).unwrap())
        .unwrap()
        .object
        .typed::<Endpoint>()
        .unwrap();
    let weak = fixture.install(AnyObjectId::of(id), Rights::RECV).raw();
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::ProcessSetFaultHandler, &[own, weak])
        ),
        Some(Error::AccessDenied)
    );
}

#[test]
fn a_fault_reaches_the_handler_as_a_call_with_the_reserved_label() {
    let mut fixture = Fixture::new();
    let (_id, handle) = handler(&mut fixture);
    // A thread of the same process waits on the handler endpoint.
    let own = fixture.process;
    let taker = fixture.running(own, 4);
    let mut waiting = [0; SIZE];
    {
        let mut writer = audhsos_abi::ipc_buffer::BufferMut::new(&mut waiting);
        writer.set_syscall_number(u64::from(Syscall::IpcRecv.number()));
        assert!(writer.set_argument(0, handle.raw()));
    }
    fixture.write_buffer(taker, &waiting);
    crate::dispatch::dispatch(&mut fixture.machine(), taker, &mut waiting);
    assert_eq!(
        fixture.objects.threads.get(taker).unwrap().state,
        ThreadState::BlockedRecv
    );

    let faulted = fixture.thread;
    let mut buffer = [0; SIZE];
    let outcome = deliver(&mut fixture.machine(), faulted, page_fault(), &mut buffer);
    assert!(outcome.reschedule);
    assert_eq!(
        fixture.objects.threads.get(faulted).unwrap().state,
        ThreadState::BlockedReply,
        "the faulting thread is blocked as any caller is"
    );
    assert_eq!(
        fixture.objects.threads.get(faulted).unwrap().fault,
        Some(page_fault())
    );

    let arrived = *fixture.buffer_of(taker);
    let view = Buffer::new(&arrived);
    let message = view.message().unwrap();
    assert!(message.is_kernel_label());
    assert_eq!(fault_kind_of(message.label), Some(FaultKind::PageFault));
    assert_eq!(message.word_count, 3);
    assert_eq!(message.handle_count, 0);
    assert_eq!(view.word(0), Some(page_fault().address));
    assert_eq!(view.word(1), Some(page_fault().instruction_pointer));
    assert_eq!(view.word(2), Some(page_fault().error_code));
    let reply = Handle::from_raw(fixture.returns_of(taker)[1]).expect("a reply handle");

    // The reply resumes the thread at the instruction it faulted on: the
    // handler answers, and the faulting thread is ready again.
    let mut answer = request(Syscall::IpcReply, &[reply.raw()]);
    fixture.write_buffer(taker, &answer);
    crate::dispatch::dispatch(&mut fixture.machine(), taker, &mut answer);
    assert_eq!(
        Buffer::new(&answer).status().unwrap().error(),
        None,
        "the handler answered"
    );
    assert_eq!(
        fixture.objects.threads.get(faulted).unwrap().state,
        ThreadState::Ready,
        "the thread the fault stopped runs again"
    );
    assert_eq!(fixture.status_of(faulted).error(), None);
}

#[test]
fn a_process_with_no_handler_stops_the_thread_that_faulted() {
    let mut fixture = Fixture::new();
    let faulted = fixture.thread;
    let mut buffer = [0; SIZE];
    let outcome = deliver(&mut fixture.machine(), faulted, page_fault(), &mut buffer);
    assert!(outcome.reschedule);
    assert!(is_faulted(&fixture.machine(), faulted));
    assert_eq!(
        fixture.objects.threads.get(faulted).unwrap().fault,
        Some(page_fault()),
        "the fault is recorded whether anybody takes it or not"
    );
}

#[test]
fn a_handler_endpoint_that_is_gone_stops_the_thread_as_no_handler_would() {
    let mut fixture = Fixture::new();
    let (id, handle) = handler(&mut fixture);
    // Both references go: the handle and the handler itself.
    let (status, _, _) = call(
        &mut fixture,
        &mut request(Syscall::HandleClose, &[handle.raw()]),
    );
    assert_eq!(status.error(), None);
    fixture.objects.endpoints.force_release(id);
    let faulted = fixture.thread;
    let mut buffer = [0; SIZE];
    deliver(&mut fixture.machine(), faulted, page_fault(), &mut buffer);
    assert!(is_faulted(&fixture.machine(), faulted));
}

#[test]
fn a_fault_nobody_has_a_reply_slot_for_stops_the_thread() {
    let mut fixture = Fixture::new();
    let (_id, handle) = handler(&mut fixture);
    let own = fixture.process;
    let taker = fixture.running(own, 4);
    let mut waiting = [0; SIZE];
    {
        let mut writer = audhsos_abi::ipc_buffer::BufferMut::new(&mut waiting);
        writer.set_syscall_number(u64::from(Syscall::IpcRecv.number()));
        assert!(writer.set_argument(0, handle.raw()));
    }
    fixture.write_buffer(taker, &waiting);
    crate::dispatch::dispatch(&mut fixture.machine(), taker, &mut waiting);
    // No handle slot left for the reply object the call needs.
    fixture.fill_handles();

    let faulted = fixture.thread;
    let mut buffer = [0; SIZE];
    deliver(&mut fixture.machine(), faulted, page_fault(), &mut buffer);
    assert!(
        is_faulted(&fixture.machine(), faulted),
        "a fault nobody can take ends where a fault nobody takes ends"
    );
    assert_eq!(
        fixture.objects.threads.get(taker).unwrap().state,
        ThreadState::BlockedRecv,
        "and the handler is back in its queue"
    );
}
