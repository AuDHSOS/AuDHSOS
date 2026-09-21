// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Fixed-home scheduling over independent processor queues.
use crate::{Event, Processors};
use audhsos_abi::ThreadState;
use kernel_objects::{Pool, ProcessId, Thread, ThreadId};
use kernel_types::PhysFrame;

fn thread(threads: &mut Pool<Thread, 16>, cpu: u8, priority: u8) -> ThreadId {
    let mut thread = Thread::new(ProcessId::new(0, 1), priority, 31, 0, PhysFrame::ZERO).unwrap();
    thread.cpu = cpu;
    threads.allocate(thread).unwrap()
}

#[test]
fn four_queues_keep_threads_on_their_home_and_notify_only_remote_processors() {
    let mut queues = Processors::new();
    let mut threads = Pool::<Thread, 16>::new();
    let mut ids = Vec::new();
    for cpu in 0..4 {
        queues.online(cpu);
        queues.select(cpu);
        let idle = thread(&mut threads, cpu, 0);
        queues.set_idle(idle);
        queues.adopt(idle);
        let id = thread(&mut threads, cpu, 7);
        ids.push(id);
    }
    queues.select(0);
    for id in &ids {
        queues.start(&mut threads, *id).unwrap();
    }
    assert_eq!(queues.take_pending(), 0b1110);
    assert_eq!(queues.take_pending(), 0);
    for (cpu, id) in (0..4).zip(&ids) {
        queues.select(cpu);
        assert_eq!(queues.pick_next(&mut threads).unwrap(), *id);
        queues
            .on_block(&mut threads, *id, Event::BlockNotification)
            .unwrap();
    }
    queues.select(2);
    for id in &ids {
        queues.on_wake(&mut threads, *id).unwrap();
    }
    assert_eq!(queues.take_pending(), 0b1011);
    for (cpu, id) in (0..4).zip(ids) {
        queues.select(cpu);
        assert_eq!(queues.pick_next(&mut threads).unwrap(), id);
        assert_eq!(threads.get(id).unwrap().state, ThreadState::Running);
    }
}

#[test]
fn round_robin_skips_offline_processors() {
    let mut queues = Processors::new();
    queues.online(2);
    queues.online(5);
    assert_eq!(
        (0..6).map(|_| queues.assign()).collect::<Vec<_>>(),
        [0, 2, 5, 0, 2, 5]
    );
}

#[test]
fn remote_lifecycle_routes_deadlines_priorities_and_reclamation() {
    let mut queues = Processors::default();
    let mut threads = Pool::<Thread, 16>::new();
    queues.online(1);
    queues.online(255);
    queues.select(1);
    assert_eq!(queues.caller(), 1);
    let idle = thread(&mut threads, 1, 0);
    queues.set_idle(idle);
    queues.adopt(idle);
    assert!(queues.is_idle());
    let id = thread(&mut threads, 1, 7);
    queues.select(0);
    queues.start(&mut threads, id).unwrap();
    queues.dequeue(&mut threads, id).unwrap();
    queues.enqueue(&mut threads, id).unwrap();
    assert_eq!(queues.take_pending(), 2);
    queues.select(1);
    queues.dequeue(&mut threads, id).unwrap();
    queues.enqueue(&mut threads, id).unwrap();
    assert_eq!(queues.take_pending(), 0);
    assert_ne!(queues.ready_bitmap(), 0);
    assert_eq!(queues.highest_ready(), Some(7));
    queues.pick_next(&mut threads).unwrap();
    assert!(queues.is_idle());
    assert_eq!(queues.current(), Some(id));
    assert!(!queues.tick(&mut threads).reschedule);
    queues.select(0);
    queues.set_priority(&mut threads, id, 8).unwrap();
    queues.yield_now(&mut threads, id).unwrap();
    queues.suspend(&mut threads, id).unwrap();
    queues.resume(&mut threads, id).unwrap();
    queues.select(1);
    queues.pick_next(&mut threads).unwrap();
    queues.select(0);
    queues
        .on_block_until(&mut threads, id, Event::BlockNotification, 50)
        .unwrap();
    queues.select(1);
    assert_eq!(queues.next_deadline(), Some(id));
    assert_eq!(queues.waiting_until(), 1);
    queues.select(0);
    assert_eq!(queues.expired(&mut threads, 49), None);
    assert_eq!(queues.expired(&mut threads, 50), Some(id));
    queues.on_wake(&mut threads, id).unwrap();
    queues.select(1);
    queues.pick_next(&mut threads).unwrap();
    queues.select(0);
    queues.fault(&mut threads, id).unwrap();
    queues.exit(&mut threads, id).unwrap();
    queues.select(1);
    assert_eq!(queues.ended(), 1);
    queues.cleared();
    assert_eq!(queues.ended(), 0);
    assert_eq!(queues.pick_next(&mut threads).unwrap(), idle);
    queues.select(255);
    assert_eq!(queues.caller(), 15);
}

#[test]
fn invalid_threads_and_homes_leave_the_queues_unchanged() {
    let mut queues = Processors::new();
    let mut threads = Pool::<Thread, 16>::new();
    let bad_home = thread(&mut threads, 16, 1);
    let stale = ThreadId::new(15, 100);
    for id in [bad_home, stale] {
        assert!(queues.enqueue(&mut threads, id).is_err());
        assert!(queues.dequeue(&mut threads, id).is_err());
        assert!(queues.start(&mut threads, id).is_err());
        assert!(queues.on_wake(&mut threads, id).is_err());
        assert!(
            queues
                .on_block(&mut threads, id, Event::BlockNotification)
                .is_err()
        );
        assert!(
            queues
                .on_block_until(&mut threads, id, Event::BlockNotification, 1)
                .is_err()
        );
        assert!(queues.suspend(&mut threads, id).is_err());
        assert!(queues.resume(&mut threads, id).is_err());
        assert!(queues.fault(&mut threads, id).is_err());
        assert!(queues.exit(&mut threads, id).is_err());
        assert!(queues.yield_now(&mut threads, id).is_err());
        assert!(queues.set_priority(&mut threads, id, 1).is_err());
    }
    assert_eq!(queues.take_pending(), 0);
    assert_eq!(queues.ready_bitmap(), 0);
}
