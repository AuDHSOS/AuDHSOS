// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::reaper`.

use audhsos_abi::{Syscall, ThreadState};

use crate::reaper::{has_work, reap};
use crate::tests::double::{Fixture, call, error_of, request, value_of};

/// The arguments of a `thread_create` that works.
fn creation(fixture: &Fixture) -> [u64; 6] {
    [fixture.own_process.raw(), 0x40_0000, 0x50_0000, 2, 4, 0]
}

#[test]
fn a_machine_where_nothing_ended_has_nothing_to_clear_away() {
    let mut fixture = Fixture::new();
    assert_eq!(fixture.scheduler.ended(), 0);
    assert!(!has_work(&fixture.machine(), None));
    assert_eq!(reap(&mut fixture.machine(), None), 0);
}

#[test]
fn the_count_of_what_is_left_to_clear_follows_the_sweep() {
    let mut fixture = Fixture::new();
    let creation = request(Syscall::ThreadCreate, &creation(&fixture));
    let raw = value_of(&mut fixture, creation);
    assert!(error_of(&mut fixture, request(Syscall::ThreadKill, &[raw])).is_none());
    assert_eq!(fixture.scheduler.ended(), 1, "the kill raised it");

    assert_eq!(reap(&mut fixture.machine(), None), 1);
    assert_eq!(
        fixture.scheduler.ended(),
        0,
        "clearing it away lowered it again"
    );
}

#[test]
fn a_thread_that_is_spared_leaves_the_count_standing() {
    let mut fixture = Fixture::new();
    let running = fixture.thread;
    fixture
        .scheduler
        .exit(&mut fixture.objects.threads, running)
        .unwrap();
    assert_eq!(fixture.scheduler.ended(), 1);

    // Named as the thread the kernel stands on, it is not cleared, and the
    // count still says there is something to come back for.
    assert_eq!(reap(&mut fixture.machine(), Some(running)), 0);
    assert_eq!(fixture.scheduler.ended(), 1);
    assert!(fixture.objects.threads.get(running).is_ok());

    assert_eq!(reap(&mut fixture.machine(), None), 1);
    assert_eq!(fixture.scheduler.ended(), 0);
}

#[test]
fn what_an_ended_thread_held_goes_back_when_it_is_cleared_away() {
    let mut fixture = Fixture::new();
    let creation = request(Syscall::ThreadCreate, &creation(&fixture));
    let raw = value_of(&mut fixture, creation);
    assert!(error_of(&mut fixture, request(Syscall::ThreadKill, &[raw])).is_none());

    assert!(has_work(&fixture.machine(), None));
    assert_eq!(fixture.environment.stacks_out(), 1);
    assert_eq!(reap(&mut fixture.machine(), None), 1);
    assert_eq!(fixture.environment.stacks_out(), 0);
    assert_eq!(fixture.environment.frames_out(), 0);
    assert!(!has_work(&fixture.machine(), None));
    assert_eq!(
        reap(&mut fixture.machine(), None),
        0,
        "a second sweep finds none"
    );
    assert_eq!(fixture.objects.threads.live(), 1, "the slot came back");
}

#[test]
fn the_thread_on_the_processor_keeps_its_stack_until_the_kernel_leaves_it() {
    let mut fixture = Fixture::new();
    // The caller ends itself: it is still standing on its kernel stack
    // while the kernel writes the answer, so nothing of it may go back yet.
    let mut buffer = request(Syscall::ThreadExit, &[]);
    let (status, _, outcome) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), None);
    assert!(outcome.reschedule);
    assert_eq!(
        fixture.objects.threads.get(fixture.thread).unwrap().state,
        ThreadState::Exited
    );
    // `thread_exit` left the processor, so the scheduler holds nobody and
    // the sweep may take the thread now.
    assert_eq!(fixture.scheduler.current(), None);
    assert!(has_work(&fixture.machine(), None));
    assert_eq!(reap(&mut fixture.machine(), None), 1);
    assert!(fixture.objects.threads.get(fixture.thread).is_err());
}

#[test]
fn a_thread_that_ended_while_it_still_holds_the_processor_is_left_alone() {
    let mut fixture = Fixture::new();
    let id = fixture.thread;
    // The state alone, without the scheduler leaving the thread: this is
    // the moment between the end of a thread and the switch away from it.
    fixture.objects.threads.get_mut(id).unwrap().state = ThreadState::Exited;
    assert_eq!(fixture.scheduler.current(), Some(id));
    assert!(!has_work(&fixture.machine(), None));
    assert_eq!(reap(&mut fixture.machine(), None), 0);
    assert!(
        fixture.objects.threads.get(id).is_ok(),
        "the thread on the processor keeps everything"
    );
}

#[test]
fn every_thread_of_a_killed_process_is_cleared_away_at_once() {
    let mut fixture = Fixture::new();
    let own = fixture.own_process.raw();
    let child = value_of(
        &mut fixture,
        request(Syscall::ProcessCreate, &[own, 4, 8, 8, 0]),
    );
    for _ in 0..3 {
        let _ = value_of(
            &mut fixture,
            request(
                Syscall::ThreadCreate,
                &[child, 0x40_0000, 0x50_0000, 1, 2, 0],
            ),
        );
    }
    assert_eq!(fixture.environment.stacks_out(), 3);
    assert!(error_of(&mut fixture, request(Syscall::ProcessKill, &[child])).is_none());
    assert_eq!(reap(&mut fixture.machine(), None), 3);
    assert_eq!(fixture.environment.stacks_out(), 0);
    assert_eq!(fixture.environment.frames_out(), 0);
    assert_eq!(fixture.objects.threads.live(), 1, "only the caller is left");
}

#[test]
fn the_caller_says_which_stack_the_kernel_is_standing_on() {
    let mut fixture = Fixture::new();
    let creation = request(Syscall::ThreadCreate, &creation(&fixture));
    let raw = value_of(&mut fixture, creation);
    let running = fixture.thread;
    let other = fixture
        .objects
        .entry(fixture.process, audhsos_abi::Handle::from_raw(raw).unwrap())
        .unwrap()
        .object
        .typed::<kernel_objects::object::Thread>()
        .unwrap();

    // Both threads end through the scheduler, which is the one place a
    // thread enters `Exited` and therefore the one place the count of what
    // is left to clear is raised. The scheduler forgets the one that was
    // on the processor, which is what `thread_exit` does.
    fixture
        .scheduler
        .exit(&mut fixture.objects.threads, other)
        .unwrap();
    fixture
        .scheduler
        .exit(&mut fixture.objects.threads, running)
        .unwrap();
    assert_eq!(fixture.scheduler.current(), None);

    // Named as the one running, the caller's thread is spared even though
    // the scheduler has forgotten it; the other one goes.
    assert!(has_work(&fixture.machine(), Some(running)));
    assert_eq!(reap(&mut fixture.machine(), Some(running)), 1);
    assert!(fixture.objects.threads.get(running).is_ok());
    assert!(fixture.objects.threads.get(other).is_err());

    // Once the kernel has left that stack, it goes too.
    assert!(has_work(&fixture.machine(), None));
    assert_eq!(reap(&mut fixture.machine(), None), 1);
    assert!(fixture.objects.threads.get(running).is_err());
}
