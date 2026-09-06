// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::wait_queue`.

use audhsos_abi::Error;
use kernel_types::{PhysAddr, PhysFrame};

use crate::object::{ProcessId, Thread, ThreadId};
use crate::pool::{ObjectId, Pool};
use crate::wait_queue::WaitQueue;

/// The pool the links thread through.
type Threads = Pool<Thread, 8>;

/// The process every thread of these tests belongs to.
fn process() -> ProcessId {
    ObjectId::new(0, 1)
}

/// A frame for the IPC buffer a thread needs to exist.
fn frame() -> PhysFrame {
    PhysFrame::containing(PhysAddr::new(0x20_0000).unwrap())
}

/// A thread of `priority` in `threads`.
fn spawn(threads: &mut Threads, priority: u8) -> ThreadId {
    threads
        .allocate(Thread::new(process(), priority, 31, 0, frame()).unwrap())
        .unwrap()
}

/// The queue in order, as ids.
fn order(queue: &WaitQueue, threads: &Threads) -> Vec<ThreadId> {
    queue.iter(threads).collect()
}

#[test]
fn an_empty_queue_names_nobody() {
    let threads = Threads::new();
    let queue = WaitQueue::EMPTY;
    assert!(queue.is_empty());
    assert_eq!(queue.len(), 0);
    assert_eq!(queue.head(), None);
    assert_eq!(queue.tail(), None);
    assert_eq!(order(&queue, &threads), Vec::new());
    assert_eq!(WaitQueue::default(), WaitQueue::EMPTY);
}

#[test]
fn threads_of_equal_priority_are_served_in_the_order_they_arrived() {
    let mut threads = Threads::new();
    let mut queue = WaitQueue::EMPTY;
    let first = spawn(&mut threads, 4);
    let second = spawn(&mut threads, 4);
    let third = spawn(&mut threads, 4);
    for id in [first, second, third] {
        queue.enqueue(&mut threads, id).unwrap();
    }
    assert_eq!(queue.len(), 3);
    assert_eq!(order(&queue, &threads), vec![first, second, third]);
    assert_eq!(queue.head(), Some(first));
    assert_eq!(queue.tail(), Some(third));
}

#[test]
fn a_higher_priority_goes_before_every_lower_one_and_behind_its_equals() {
    let mut threads = Threads::new();
    let mut queue = WaitQueue::EMPTY;
    let low = spawn(&mut threads, 1);
    let high = spawn(&mut threads, 9);
    let middle = spawn(&mut threads, 5);
    let second_high = spawn(&mut threads, 9);
    for id in [low, high, middle, second_high] {
        queue.enqueue(&mut threads, id).unwrap();
    }
    assert_eq!(
        order(&queue, &threads),
        vec![high, second_high, middle, low],
        "priority first, then the order of arrival"
    );
}

#[test]
fn a_thread_that_enters_the_front_becomes_the_head() {
    let mut threads = Threads::new();
    let mut queue = WaitQueue::EMPTY;
    let low = spawn(&mut threads, 2);
    let high = spawn(&mut threads, 8);
    queue.enqueue(&mut threads, low).unwrap();
    queue.enqueue(&mut threads, high).unwrap();
    assert_eq!(queue.head(), Some(high));
    assert_eq!(queue.tail(), Some(low));
    assert_eq!(order(&queue, &threads), vec![high, low]);
}

#[test]
fn dequeueing_the_front_hands_out_the_head_and_unlinks_it() {
    let mut threads = Threads::new();
    let mut queue = WaitQueue::EMPTY;
    let first = spawn(&mut threads, 3);
    let second = spawn(&mut threads, 3);
    queue.enqueue(&mut threads, first).unwrap();
    queue.enqueue(&mut threads, second).unwrap();
    assert_eq!(queue.dequeue_front(&mut threads), Some(first));
    assert!(threads.get(first).unwrap().wait_links.is_unlinked());
    assert_eq!(queue.len(), 1);
    assert_eq!(queue.dequeue_front(&mut threads), Some(second));
    assert!(queue.is_empty());
    assert_eq!(queue.tail(), None);
    assert_eq!(queue.dequeue_front(&mut threads), None);
}

#[test]
fn a_thread_can_be_taken_out_of_the_middle() {
    let mut threads = Threads::new();
    let mut queue = WaitQueue::EMPTY;
    let first = spawn(&mut threads, 3);
    let second = spawn(&mut threads, 3);
    let third = spawn(&mut threads, 3);
    for id in [first, second, third] {
        queue.enqueue(&mut threads, id).unwrap();
    }
    assert!(queue.unlink(&mut threads, second));
    assert_eq!(order(&queue, &threads), vec![first, third]);
    assert_eq!(queue.len(), 2);
    assert!(threads.get(second).unwrap().wait_links.is_unlinked());
}

#[test]
fn taking_out_the_tail_corrects_the_tail() {
    let mut threads = Threads::new();
    let mut queue = WaitQueue::EMPTY;
    let first = spawn(&mut threads, 3);
    let second = spawn(&mut threads, 3);
    queue.enqueue(&mut threads, first).unwrap();
    queue.enqueue(&mut threads, second).unwrap();
    assert!(queue.unlink(&mut threads, second));
    assert_eq!(queue.tail(), Some(first));
    assert_eq!(queue.head(), Some(first));
}

#[test]
fn taking_out_a_thread_that_waits_nowhere_changes_nothing() {
    let mut threads = Threads::new();
    let mut queue = WaitQueue::EMPTY;
    let waiting = spawn(&mut threads, 3);
    let elsewhere = spawn(&mut threads, 3);
    queue.enqueue(&mut threads, waiting).unwrap();
    assert!(!queue.unlink(&mut threads, elsewhere));
    assert_eq!(order(&queue, &threads), vec![waiting]);
    assert_eq!(queue.len(), 1);
}

#[test]
fn a_thread_the_pool_does_not_hold_is_refused() {
    let mut threads = Threads::new();
    let mut queue = WaitQueue::EMPTY;
    let stale: ThreadId = ObjectId::new(7, 3);
    assert_eq!(
        queue.enqueue(&mut threads, stale),
        Err(Error::InvalidHandle)
    );
    assert!(!queue.unlink(&mut threads, stale));
    assert!(queue.is_empty());
}

#[test]
fn a_thread_that_already_waits_is_refused_a_second_place() {
    let mut threads = Threads::new();
    let mut queue = WaitQueue::EMPTY;
    let only = spawn(&mut threads, 3);
    queue.enqueue(&mut threads, only).unwrap();
    assert_eq!(
        queue.enqueue(&mut threads, only),
        Err(Error::InvalidState),
        "the head of a queue of one is unlinked and in it all the same"
    );
    let second = spawn(&mut threads, 3);
    queue.enqueue(&mut threads, second).unwrap();
    assert_eq!(
        queue.enqueue(&mut threads, second),
        Err(Error::InvalidState)
    );
    assert_eq!(queue.len(), 2);
}

#[test]
fn every_thread_of_a_full_queue_comes_back_out_in_order() {
    let mut threads = Threads::new();
    let mut queue = WaitQueue::EMPTY;
    let mut expected = Vec::new();
    for priority in 0..8u8 {
        let id = spawn(&mut threads, priority);
        expected.push(id);
        queue.enqueue(&mut threads, id).unwrap();
    }
    expected.reverse();
    let mut served = Vec::new();
    while let Some(id) = queue.dequeue_front(&mut threads) {
        served.push(id);
    }
    assert_eq!(served, expected, "the highest priority first");
    assert!(queue.is_empty());
    assert_eq!(queue.len(), 0);
}
