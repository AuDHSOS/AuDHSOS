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
    assert!(!has_work(&fixture.machine()));
    assert_eq!(reap(&mut fixture.machine()), 0);
}

#[test]
fn what_an_ended_thread_held_goes_back_when_it_is_cleared_away() {
    let mut fixture = Fixture::new();
    let creation = request(Syscall::ThreadCreate, &creation(&fixture));
    let raw = value_of(&mut fixture, creation);
    assert!(error_of(&mut fixture, request(Syscall::ThreadKill, &[raw])).is_none());

    assert!(has_work(&fixture.machine()));
    assert_eq!(fixture.environment.stacks_out(), 1);
    assert_eq!(reap(&mut fixture.machine()), 1);
    assert_eq!(fixture.environment.stacks_out(), 0);
    assert_eq!(fixture.environment.frames_out(), 0);
    assert!(!has_work(&fixture.machine()));
    assert_eq!(reap(&mut fixture.machine()), 0, "a second sweep finds none");
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
    assert!(has_work(&fixture.machine()));
    assert_eq!(reap(&mut fixture.machine()), 1);
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
    assert!(!has_work(&fixture.machine()));
    assert_eq!(reap(&mut fixture.machine()), 0);
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
    assert_eq!(reap(&mut fixture.machine()), 3);
    assert_eq!(fixture.environment.stacks_out(), 0);
    assert_eq!(fixture.environment.frames_out(), 0);
    assert_eq!(fixture.objects.threads.live(), 1, "only the caller is left");
}
