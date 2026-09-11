// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The deadline list of `crate::scheduler`, covering the deadline items of
//! the catalog 6.6.59.

use audhsos_abi::layout::PRIORITY_COUNT;
use audhsos_abi::{Error, ThreadState};
use kernel_objects::object::{Thread, ThreadId};
use kernel_objects::pool::{ObjectId, Pool};
use kernel_types::{PhysAddr, PhysFrame};

use crate::scheduler::Scheduler;
use crate::transition::Event;

/// How many threads the tests keep.
const THREADS: usize = 8;

/// The pool the tests run on.
type Threads = Pool<Thread, THREADS>;

/// A thread of the pool, running, which is the state a wait starts from.
fn running(scheduler: &mut Scheduler, threads: &mut Threads) -> ThreadId {
    let frame = PhysFrame::containing(PhysAddr::new(0x20_0000).unwrap());
    let process = ObjectId::new(0, 1);
    let thread = Thread::new(process, 1, PRIORITY_COUNT - 1, 0, frame).unwrap();
    let id = threads.allocate(thread).unwrap();
    scheduler.start(threads, id).unwrap();
    scheduler.pick_next(threads).unwrap();
    id
}

/// A scheduler with an idle thread, so that a wait has somewhere to go.
fn machine() -> (Scheduler, Threads) {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    let frame = PhysFrame::containing(PhysAddr::new(0x20_0000).unwrap());
    let process = ObjectId::new(0, 1);
    let idle = Thread::new(process, 0, PRIORITY_COUNT - 1, 0, frame).unwrap();
    let idle = threads.allocate(idle).unwrap();
    scheduler.set_idle(idle);
    (scheduler, threads)
}

/// Blocks `id` until `deadline`.
fn block_until(scheduler: &mut Scheduler, threads: &mut Threads, id: ThreadId, deadline: u64) {
    scheduler
        .on_block_until(threads, id, Event::BlockNotification, deadline)
        .unwrap();
}

/// Every thread that comes out at `now`, in the order it comes out.
fn expire(scheduler: &mut Scheduler, threads: &mut Threads, now: u64) -> Vec<ThreadId> {
    let mut woken = Vec::new();
    while let Some(id) = scheduler.expired(threads, now) {
        woken.push(id);
    }
    woken
}

#[test]
fn a_fresh_scheduler_has_nobody_waiting() {
    let (scheduler, _threads) = machine();
    assert_eq!(scheduler.waiting_until(), 0);
    assert_eq!(scheduler.next_deadline(), None);
}

#[test]
fn a_thread_that_blocks_with_a_deadline_carries_it() {
    let (mut scheduler, mut threads) = machine();
    let id = running(&mut scheduler, &mut threads);
    block_until(&mut scheduler, &mut threads, id, 5_000);
    assert_eq!(
        threads.get(id).unwrap().state,
        ThreadState::BlockedNotification
    );
    assert_eq!(threads.get(id).unwrap().deadline, Some(5_000));
    assert_eq!(scheduler.waiting_until(), 1);
    assert_eq!(scheduler.next_deadline(), Some(id));
}

#[test]
fn inserts_land_in_order_wherever_they_come_from() {
    let (mut scheduler, mut threads) = machine();
    // Into an empty list, at the back, at the front, and in the middle.
    let middle = running(&mut scheduler, &mut threads);
    block_until(&mut scheduler, &mut threads, middle, 200);
    let last = running(&mut scheduler, &mut threads);
    block_until(&mut scheduler, &mut threads, last, 300);
    let first = running(&mut scheduler, &mut threads);
    block_until(&mut scheduler, &mut threads, first, 100);
    let between = running(&mut scheduler, &mut threads);
    block_until(&mut scheduler, &mut threads, between, 250);

    assert_eq!(scheduler.waiting_until(), 4);
    assert_eq!(scheduler.next_deadline(), Some(first));
    assert_eq!(
        expire(&mut scheduler, &mut threads, 1_000),
        vec![first, middle, between, last]
    );
}

#[test]
fn two_threads_with_one_deadline_come_out_in_the_order_they_went_in() {
    let (mut scheduler, mut threads) = machine();
    let first = running(&mut scheduler, &mut threads);
    block_until(&mut scheduler, &mut threads, first, 100);
    let second = running(&mut scheduler, &mut threads);
    block_until(&mut scheduler, &mut threads, second, 100);
    let third = running(&mut scheduler, &mut threads);
    block_until(&mut scheduler, &mut threads, third, 100);
    assert_eq!(
        expire(&mut scheduler, &mut threads, 100),
        vec![first, second, third]
    );
}

#[test]
fn a_deadline_at_now_expires_and_one_a_microsecond_later_does_not() {
    let (mut scheduler, mut threads) = machine();
    let due = running(&mut scheduler, &mut threads);
    block_until(&mut scheduler, &mut threads, due, 100);
    let later = running(&mut scheduler, &mut threads);
    block_until(&mut scheduler, &mut threads, later, 101);
    assert_eq!(expire(&mut scheduler, &mut threads, 100), vec![due]);
    assert_eq!(scheduler.waiting_until(), 1);
    assert_eq!(expire(&mut scheduler, &mut threads, 101), vec![later]);
    assert_eq!(scheduler.waiting_until(), 0);
}

#[test]
fn a_tick_that_expires_nothing_leaves_the_list_alone() {
    let (mut scheduler, mut threads) = machine();
    let id = running(&mut scheduler, &mut threads);
    block_until(&mut scheduler, &mut threads, id, 1_000);
    assert_eq!(scheduler.expired(&mut threads, 999), None);
    assert_eq!(scheduler.waiting_until(), 1);
    assert_eq!(scheduler.next_deadline(), Some(id));
}

#[test]
fn an_expired_thread_is_out_of_the_list_and_carries_no_deadline() {
    let (mut scheduler, mut threads) = machine();
    let id = running(&mut scheduler, &mut threads);
    block_until(&mut scheduler, &mut threads, id, 100);
    assert_eq!(scheduler.expired(&mut threads, 100), Some(id));
    assert_eq!(threads.get(id).unwrap().deadline, None);
    assert_eq!(scheduler.waiting_until(), 0);
    assert_eq!(scheduler.next_deadline(), None);
    // Making it ready is the caller's; the list has let go either way.
    assert_eq!(
        threads.get(id).unwrap().state,
        ThreadState::BlockedNotification
    );
    scheduler.on_wake(&mut threads, id).unwrap();
    assert_eq!(threads.get(id).unwrap().state, ThreadState::Ready);
}

#[test]
fn a_thread_woken_before_its_deadline_leaves_the_list_with_its_state() {
    let (mut scheduler, mut threads) = machine();
    let signalled = running(&mut scheduler, &mut threads);
    block_until(&mut scheduler, &mut threads, signalled, 100);
    let other = running(&mut scheduler, &mut threads);
    block_until(&mut scheduler, &mut threads, other, 200);

    scheduler.on_wake(&mut threads, signalled).unwrap();
    assert_eq!(threads.get(signalled).unwrap().deadline, None);
    assert_eq!(scheduler.waiting_until(), 1);
    assert_eq!(scheduler.next_deadline(), Some(other));
    // The tick that follows finds only the thread that still waits, so a
    // thread cannot be woken twice.
    assert_eq!(expire(&mut scheduler, &mut threads, 1_000), vec![other]);
}

#[test]
fn a_thread_that_leaves_the_wait_any_other_way_leaves_no_entry_behind() {
    for event in [Event::Suspend, Event::Exit] {
        let (mut scheduler, mut threads) = machine();
        let leaving = running(&mut scheduler, &mut threads);
        block_until(&mut scheduler, &mut threads, leaving, 100);
        let staying = running(&mut scheduler, &mut threads);
        block_until(&mut scheduler, &mut threads, staying, 200);

        match event {
            Event::Suspend => scheduler.suspend(&mut threads, leaving).unwrap(),
            _ => scheduler.exit(&mut threads, leaving).unwrap(),
        };
        assert_eq!(threads.get(leaving).unwrap().deadline, None, "{event:?}");
        assert_eq!(scheduler.waiting_until(), 1, "{event:?}");
        assert_eq!(expire(&mut scheduler, &mut threads, 1_000), vec![staying]);
    }
}

#[test]
fn every_thread_of_a_list_whose_deadlines_have_passed_comes_out_at_once() {
    let (mut scheduler, mut threads) = machine();
    let mut waiting = Vec::new();
    for step in 1..=5_u64 {
        let id = running(&mut scheduler, &mut threads);
        block_until(&mut scheduler, &mut threads, id, step.saturating_mul(10));
        waiting.push(id);
    }
    assert_eq!(scheduler.waiting_until(), 5);
    assert_eq!(expire(&mut scheduler, &mut threads, 50), waiting);
    assert_eq!(scheduler.waiting_until(), 0);
}

#[test]
fn a_refused_block_leaves_the_thread_out_of_the_list() {
    let (mut scheduler, mut threads) = machine();
    let id = running(&mut scheduler, &mut threads);
    scheduler.suspend(&mut threads, id).unwrap();
    // A suspended thread cannot block, and the table is what says so.
    assert_eq!(
        scheduler.on_block_until(&mut threads, id, Event::BlockNotification, 100),
        Err(Error::InvalidState)
    );
    assert_eq!(threads.get(id).unwrap().deadline, None);
    assert_eq!(scheduler.waiting_until(), 0);
}

#[test]
fn a_thread_the_pool_has_dropped_stops_the_walk() {
    // The invariant says a thread in the list is a thread of the pool. If
    // it is not, the walk ends rather than looping over a torn link.
    let (mut scheduler, mut threads) = machine();
    let id = running(&mut scheduler, &mut threads);
    block_until(&mut scheduler, &mut threads, id, 100);
    threads.force_release(id);
    assert_eq!(scheduler.expired(&mut threads, 1_000), None);
}

#[test]
fn an_entry_without_a_deadline_stops_the_walk() {
    // The same invariant from the other side: the deadline is what says
    // the thread is in the list, so an entry without one is not expired.
    let (mut scheduler, mut threads) = machine();
    let id = running(&mut scheduler, &mut threads);
    block_until(&mut scheduler, &mut threads, id, 100);
    threads.get_mut(id).unwrap().deadline = None;
    assert_eq!(scheduler.expired(&mut threads, 1_000), None);
}

/// The list against a sorted vector of the same deadlines.
#[test]
fn model_the_list_agrees_with_a_sorted_vector() {
    let (mut scheduler, mut threads) = machine();
    let deadlines = [70_u64, 10, 50, 10, 90, 30];
    let mut model: Vec<(u64, ThreadId)> = Vec::new();
    for (order, deadline) in deadlines.iter().enumerate() {
        let id = running(&mut scheduler, &mut threads);
        block_until(&mut scheduler, &mut threads, id, *deadline);
        model.push((*deadline, id));
        let _ = order;
    }
    // A stable sort by deadline is the order the list is in, because the
    // insert puts an equal deadline behind the one that was there.
    model.sort_by_key(|(deadline, _)| *deadline);

    // One removal, which both sides make.
    let (_, removed) = model.remove(2);
    scheduler.on_wake(&mut threads, removed).unwrap();

    for now in [9_u64, 10, 29, 30, 49, 50, 69, 70, 89, 90] {
        let mut expected = Vec::new();
        while model.first().is_some_and(|(deadline, _)| *deadline <= now) {
            expected.push(model.remove(0).1);
        }
        assert_eq!(expire(&mut scheduler, &mut threads, now), expected, "{now}");
    }
    assert!(model.is_empty());
    assert_eq!(scheduler.waiting_until(), 0);
}
