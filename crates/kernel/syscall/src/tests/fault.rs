// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::fault`.

use audhsos_abi::{Syscall, ThreadState};

use crate::fault::{is_faulted, stop};
use crate::reaper::has_work;
use crate::tests::double::{Fixture, error_of, request, value_of};

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
