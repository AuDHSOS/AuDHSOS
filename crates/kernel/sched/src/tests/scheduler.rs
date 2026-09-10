// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::scheduler`, covering the scheduler items of the
//! catalog 6.6.7.

use audhsos_abi::layout::{DEFAULT_TIME_SLICE_TICKS, PRIORITY_COUNT};
use audhsos_abi::{Error, ThreadState};
use kernel_objects::object::{Thread, ThreadId};
use kernel_objects::pool::{ObjectId, Pool};
use kernel_types::{PhysAddr, PhysFrame};

use crate::scheduler::{Outcome, Scheduler};
use crate::transition::Event;

/// How many threads the tests keep.
const THREADS: usize = 8;

/// The pool the tests run on.
type Threads = Pool<Thread, THREADS>;

/// A frame to give a thread as its IPC buffer; the scheduler never reads
/// it.
fn frame() -> PhysFrame {
    PhysFrame::containing(PhysAddr::new(0x20_0000).unwrap())
}

/// Adds a thread of `priority` to `threads` and returns its id.
fn add(threads: &mut Threads, priority: u8) -> ThreadId {
    let process = ObjectId::new(0, 1);
    let thread = Thread::new(process, priority, PRIORITY_COUNT - 1, 0, frame()).unwrap();
    threads.allocate(thread).unwrap()
}

/// Adds a thread and starts it, so that it is ready.
fn ready(scheduler: &mut Scheduler, threads: &mut Threads, priority: u8) -> ThreadId {
    let id = add(threads, priority);
    scheduler.start(threads, id).unwrap();
    id
}

/// The state of `id`.
fn state(threads: &Threads, id: ThreadId) -> ThreadState {
    threads.get(id).unwrap().state
}

#[test]
fn an_empty_scheduler_picks_the_idle_thread() {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    let idle = add(&mut threads, 0);
    scheduler.set_idle(idle);
    assert!(scheduler.is_idle());
    assert_eq!(scheduler.highest_ready(), None);
    assert_eq!(scheduler.pick_next(&mut threads), Ok(idle));
    assert_eq!(scheduler.current(), Some(idle));
    assert_eq!(scheduler.idle(), Some(idle));
}

#[test]
fn an_empty_scheduler_without_an_idle_thread_has_nothing_to_run() {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    assert_eq!(scheduler.pick_next(&mut threads), Err(Error::NotRunnable));
}

#[test]
fn a_single_ready_thread_is_picked_again_and_again() {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    let idle = add(&mut threads, 0);
    scheduler.set_idle(idle);
    let only = ready(&mut scheduler, &mut threads, 5);
    for _ in 0..4 {
        assert_eq!(scheduler.pick_next(&mut threads), Ok(only));
        assert_eq!(state(&threads, only), ThreadState::Running);
    }
}

#[test]
fn equal_priorities_alternate_in_the_order_they_arrived() {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    let first = ready(&mut scheduler, &mut threads, 3);
    let second = ready(&mut scheduler, &mut threads, 3);
    let third = ready(&mut scheduler, &mut threads, 3);
    let picked: Vec<ThreadId> = (0..6)
        .map(|_| scheduler.pick_next(&mut threads).unwrap())
        .collect();
    assert_eq!(
        picked,
        vec![first, second, third, first, second, third],
        "round robin in first-in first-out order"
    );
}

#[test]
fn a_higher_priority_thread_takes_the_processor_and_a_lower_one_does_not() {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    let low = ready(&mut scheduler, &mut threads, 2);
    assert_eq!(scheduler.pick_next(&mut threads), Ok(low));

    let high = add(&mut threads, 9);
    assert_eq!(scheduler.start(&mut threads, high), Ok(Outcome::RESCHEDULE));
    assert_eq!(scheduler.pick_next(&mut threads), Ok(high));
    assert_eq!(state(&threads, low), ThreadState::Ready);

    let lower = add(&mut threads, 1);
    assert_eq!(scheduler.start(&mut threads, lower), Ok(Outcome::NOTHING));
    assert_eq!(
        scheduler.highest_ready(),
        Some(2),
        "the low thread is still the best waiting one"
    );
}

#[test]
fn the_highest_priority_queue_is_served_first() {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    let low = ready(&mut scheduler, &mut threads, 1);
    let middle = ready(&mut scheduler, &mut threads, 7);
    let high = ready(&mut scheduler, &mut threads, 31);
    assert_eq!(scheduler.highest_ready(), Some(31));
    assert_eq!(scheduler.pick_next(&mut threads), Ok(high));
    assert_eq!(scheduler.pick_next(&mut threads), Ok(high), "it stays on");
    scheduler.exit(&mut threads, high).unwrap();
    assert_eq!(scheduler.pick_next(&mut threads), Ok(middle));
    scheduler.exit(&mut threads, middle).unwrap();
    assert_eq!(scheduler.pick_next(&mut threads), Ok(low));
}

#[test]
fn a_blocked_thread_is_never_picked() {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    let idle = add(&mut threads, 0);
    scheduler.set_idle(idle);
    let waiting = ready(&mut scheduler, &mut threads, 4);
    assert_eq!(scheduler.pick_next(&mut threads), Ok(waiting));
    assert_eq!(
        scheduler.on_block(&mut threads, waiting, Event::BlockRecv),
        Ok(Outcome::RESCHEDULE)
    );
    assert_eq!(state(&threads, waiting), ThreadState::BlockedRecv);
    assert!(scheduler.is_idle());
    assert_eq!(scheduler.pick_next(&mut threads), Ok(idle));
}

#[test]
fn blocking_asks_for_an_event_that_blocks() {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    let id = ready(&mut scheduler, &mut threads, 4);
    scheduler.pick_next(&mut threads).unwrap();
    assert_eq!(
        scheduler.on_block(&mut threads, id, Event::Yield),
        Err(Error::InvalidArgument)
    );
    assert_eq!(state(&threads, id), ThreadState::Running);
}

#[test]
fn a_woken_thread_is_ready_again_and_waking_twice_changes_nothing() {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    let id = ready(&mut scheduler, &mut threads, 4);
    scheduler.pick_next(&mut threads).unwrap();
    scheduler
        .on_block(&mut threads, id, Event::BlockNotification)
        .unwrap();
    assert_eq!(scheduler.on_wake(&mut threads, id), Ok(Outcome::RESCHEDULE));
    assert_eq!(state(&threads, id), ThreadState::Ready);
    assert_eq!(
        scheduler.on_wake(&mut threads, id),
        Ok(Outcome::NOTHING),
        "waking a ready thread is idempotent"
    );
    assert_eq!(scheduler.pick_next(&mut threads), Ok(id));
}

#[test]
fn a_suspended_thread_leaves_the_queue_and_comes_back_on_resume() {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    let idle = add(&mut threads, 0);
    scheduler.set_idle(idle);
    let id = ready(&mut scheduler, &mut threads, 6);
    assert_eq!(scheduler.suspend(&mut threads, id), Ok(Outcome::NOTHING));
    assert_eq!(state(&threads, id), ThreadState::Suspended);
    assert!(scheduler.is_idle());
    assert_eq!(scheduler.pick_next(&mut threads), Ok(idle));

    assert_eq!(scheduler.resume(&mut threads, id), Ok(Outcome::RESCHEDULE));
    assert_eq!(state(&threads, id), ThreadState::Ready);
    assert_eq!(scheduler.pick_next(&mut threads), Ok(id));
    assert_eq!(
        scheduler.resume(&mut threads, id),
        Err(Error::InvalidState),
        "a running thread does not resume"
    );
}

#[test]
fn a_faulted_thread_stops_and_the_rest_keeps_running() {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    let other = ready(&mut scheduler, &mut threads, 3);
    let falling = ready(&mut scheduler, &mut threads, 5);
    assert_eq!(scheduler.pick_next(&mut threads), Ok(falling));
    assert_eq!(
        scheduler.fault(&mut threads, falling),
        Ok(Outcome::RESCHEDULE)
    );
    assert_eq!(state(&threads, falling), ThreadState::Faulted);
    assert_eq!(scheduler.pick_next(&mut threads), Ok(other));
    assert_eq!(
        scheduler.resume(&mut threads, falling),
        Ok(Outcome::RESCHEDULE),
        "a repaired thread of higher priority takes the processor back"
    );
    assert_eq!(state(&threads, falling), ThreadState::Ready);
    assert_eq!(scheduler.pick_next(&mut threads), Ok(falling));
}

#[test]
fn a_thread_that_ends_is_out_of_every_queue_whatever_it_was_doing() {
    for stage in 0..3 {
        let mut threads = Threads::new();
        let mut scheduler = Scheduler::new();
        let idle = add(&mut threads, 0);
        scheduler.set_idle(idle);
        let id = ready(&mut scheduler, &mut threads, 4);
        match stage {
            0 => {}
            1 => {
                scheduler.pick_next(&mut threads).unwrap();
            }
            _ => {
                scheduler.pick_next(&mut threads).unwrap();
                scheduler
                    .on_block(&mut threads, id, Event::BlockSend)
                    .unwrap();
            }
        }
        scheduler.exit(&mut threads, id).unwrap();
        assert_eq!(state(&threads, id), ThreadState::Exited, "stage {stage}");
        assert!(scheduler.is_idle(), "stage {stage}");
        assert_eq!(scheduler.pick_next(&mut threads), Ok(idle), "stage {stage}");
        assert_eq!(
            scheduler.exit(&mut threads, id),
            Ok(Outcome::NOTHING),
            "ending a thread twice is no error"
        );
    }
}

#[test]
fn the_count_of_ended_threads_rises_once_a_thread_and_falls_when_it_is_cleared() {
    let mut threads = Threads::default();
    let mut scheduler = Scheduler::new();
    let first = ready(&mut scheduler, &mut threads, 4);
    let second = ready(&mut scheduler, &mut threads, 4);
    assert_eq!(scheduler.ended(), 0, "nothing has ended yet");

    scheduler.exit(&mut threads, first).unwrap();
    assert_eq!(scheduler.ended(), 1);
    scheduler.exit(&mut threads, first).unwrap();
    assert_eq!(
        scheduler.ended(),
        1,
        "ending a thread twice ends one thread"
    );

    scheduler.exit(&mut threads, second).unwrap();
    assert_eq!(scheduler.ended(), 2);

    scheduler.cleared();
    scheduler.cleared();
    assert_eq!(scheduler.ended(), 0);
    scheduler.cleared();
    assert_eq!(scheduler.ended(), 0, "the count does not go below nothing");
}

#[test]
fn no_event_but_an_end_raises_the_count() {
    let mut threads = Threads::default();
    let mut scheduler = Scheduler::new();
    let id = ready(&mut scheduler, &mut threads, 4);
    scheduler.pick_next(&mut threads).unwrap();
    scheduler
        .on_block(&mut threads, id, Event::BlockSend)
        .unwrap();
    scheduler.on_wake(&mut threads, id).unwrap();
    scheduler.pick_next(&mut threads).unwrap();
    scheduler.yield_now(&mut threads, id).unwrap();
    scheduler.suspend(&mut threads, id).unwrap();
    scheduler.resume(&mut threads, id).unwrap();
    assert_eq!(scheduler.ended(), 0);
    scheduler.exit(&mut threads, id).unwrap();
    assert_eq!(scheduler.ended(), 1);
}

#[test]
fn the_time_slice_runs_out_exactly_at_its_last_tick() {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    let id = ready(&mut scheduler, &mut threads, 4);
    assert_eq!(scheduler.pick_next(&mut threads), Ok(id));
    assert_eq!(
        threads.get(id).unwrap().time_slice,
        DEFAULT_TIME_SLICE_TICKS
    );
    for tick in 1..DEFAULT_TIME_SLICE_TICKS {
        assert_eq!(
            scheduler.tick(&mut threads),
            Outcome::NOTHING,
            "tick {tick} of {DEFAULT_TIME_SLICE_TICKS}"
        );
    }
    assert_eq!(
        scheduler.tick(&mut threads),
        Outcome::RESCHEDULE,
        "the last tick of the slice"
    );
    assert_eq!(threads.get(id).unwrap().time_slice, 0);
}

#[test]
fn ticking_past_the_end_of_a_slice_keeps_asking_for_a_switch() {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    let id = ready(&mut scheduler, &mut threads, 4);
    scheduler.pick_next(&mut threads).unwrap();
    for _ in 0..DEFAULT_TIME_SLICE_TICKS {
        scheduler.tick(&mut threads);
    }
    // However many ticks arrive afterwards, the count stays at zero and
    // the answer stays the same: nothing wraps into a fresh slice.
    for _ in 0..1000 {
        assert_eq!(scheduler.tick(&mut threads), Outcome::RESCHEDULE);
    }
    assert_eq!(threads.get(id).unwrap().time_slice, 0);
}

#[test]
fn a_thread_that_blocks_before_its_slice_ends_keeps_no_credit() {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    let id = ready(&mut scheduler, &mut threads, 4);
    scheduler.pick_next(&mut threads).unwrap();
    scheduler.tick(&mut threads);
    scheduler
        .on_block(&mut threads, id, Event::BlockRecv)
        .unwrap();
    assert_eq!(threads.get(id).unwrap().time_slice, 0);
    scheduler.on_wake(&mut threads, id).unwrap();
    assert_eq!(scheduler.pick_next(&mut threads), Ok(id));
    assert_eq!(
        threads.get(id).unwrap().time_slice,
        DEFAULT_TIME_SLICE_TICKS,
        "it starts its next run with a whole slice, not with what was left"
    );
}

#[test]
fn the_idle_thread_has_no_slice_and_yields_as_soon_as_anyone_is_ready() {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    let idle = add(&mut threads, 0);
    scheduler.set_idle(idle);
    assert_eq!(scheduler.pick_next(&mut threads), Ok(idle));
    for _ in 0..100 {
        assert_eq!(scheduler.tick(&mut threads), Outcome::NOTHING);
    }
    let woken = ready(&mut scheduler, &mut threads, 1);
    assert_eq!(scheduler.tick(&mut threads), Outcome::RESCHEDULE);
    assert_eq!(scheduler.pick_next(&mut threads), Ok(woken));
}

#[test]
fn a_tick_without_a_current_thread_asks_for_a_switch_only_when_someone_waits() {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    assert_eq!(scheduler.tick(&mut threads), Outcome::NOTHING);
    ready(&mut scheduler, &mut threads, 2);
    assert_eq!(scheduler.tick(&mut threads), Outcome::RESCHEDULE);
}

#[test]
fn changing_the_priority_of_a_ready_thread_moves_it_between_queues() {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    let low = ready(&mut scheduler, &mut threads, 2);
    let high = ready(&mut scheduler, &mut threads, 8);
    assert_eq!(scheduler.highest_ready(), Some(8));
    assert_eq!(
        scheduler.set_priority(&mut threads, low, 9),
        Ok(Outcome::RESCHEDULE)
    );
    assert_eq!(scheduler.highest_ready(), Some(9));
    assert_eq!(scheduler.pick_next(&mut threads), Ok(low));
    assert_eq!(threads.get(low).unwrap().priority, 9);
    assert_eq!(
        scheduler.pick_next(&mut threads),
        Ok(low),
        "it now stands above the thread that used to outrank it"
    );
    scheduler.exit(&mut threads, low).unwrap();
    assert_eq!(scheduler.pick_next(&mut threads), Ok(high));
}

#[test]
fn lowering_the_running_thread_below_a_waiting_one_asks_for_a_switch() {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    let waiting = ready(&mut scheduler, &mut threads, 5);
    let running = ready(&mut scheduler, &mut threads, 9);
    assert_eq!(scheduler.pick_next(&mut threads), Ok(running));
    assert_eq!(
        scheduler.set_priority(&mut threads, running, 6),
        Ok(Outcome::NOTHING),
        "still above the waiting thread"
    );
    assert_eq!(
        scheduler.set_priority(&mut threads, running, 4),
        Ok(Outcome::RESCHEDULE)
    );
    assert_eq!(scheduler.highest_ready(), Some(5));
    assert_eq!(scheduler.pick_next(&mut threads), Ok(waiting));
}

#[test]
fn a_priority_above_the_maximum_of_the_thread_is_refused() {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    let process = ObjectId::new(0, 1);
    let thread = Thread::new(process, 2, 5, 0, frame()).unwrap();
    let id = threads.allocate(thread).unwrap();
    scheduler.start(&mut threads, id).unwrap();
    assert_eq!(
        scheduler.set_priority(&mut threads, id, 6),
        Err(Error::InvalidArgument)
    );
    assert_eq!(
        scheduler.set_priority(&mut threads, id, PRIORITY_COUNT),
        Err(Error::InvalidArgument),
        "and one outside the priorities of this system"
    );
    assert_eq!(threads.get(id).unwrap().priority, 2);
    assert_eq!(
        scheduler.set_priority(&mut threads, id, 5),
        Ok(Outcome::RESCHEDULE),
        "its own maximum is allowed, and nobody holds the processor"
    );
    assert_eq!(threads.get(id).unwrap().priority, 5);
}

#[test]
fn the_idle_thread_never_enters_a_queue() {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    let idle = add(&mut threads, 0);
    scheduler.set_idle(idle);
    assert_eq!(
        scheduler.enqueue(&mut threads, idle),
        Err(Error::InvalidArgument)
    );
    assert!(scheduler.is_idle());
}

#[test]
fn a_thread_the_pool_does_not_hold_is_no_thread_of_the_scheduler() {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    let ghost: ThreadId = ObjectId::new(6, 1);
    assert_eq!(
        scheduler.enqueue(&mut threads, ghost),
        Err(Error::InvalidHandle)
    );
    assert_eq!(
        scheduler.dequeue(&mut threads, ghost),
        Err(Error::InvalidHandle)
    );
    assert_eq!(
        scheduler.start(&mut threads, ghost),
        Err(Error::InvalidHandle)
    );
    assert_eq!(
        scheduler.exit(&mut threads, ghost),
        Err(Error::InvalidHandle)
    );
}

#[test]
fn a_thread_enters_a_queue_at_most_once() {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    let id = ready(&mut scheduler, &mut threads, 3);
    assert_eq!(
        scheduler.enqueue(&mut threads, id),
        Err(Error::InvalidState)
    );
    assert_eq!(
        scheduler.start(&mut threads, id),
        Err(Error::InvalidState),
        "and starting it twice is refused by the table"
    );
}

#[test]
fn dequeueing_a_thread_that_is_in_no_queue_changes_nothing() {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    let id = add(&mut threads, 3);
    assert_eq!(scheduler.dequeue(&mut threads, id), Ok(()));
    assert!(scheduler.is_idle());
}

#[test]
fn yielding_puts_the_thread_behind_its_equals() {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    let first = ready(&mut scheduler, &mut threads, 4);
    let second = ready(&mut scheduler, &mut threads, 4);
    assert_eq!(scheduler.pick_next(&mut threads), Ok(first));
    assert_eq!(
        scheduler.yield_now(&mut threads, first),
        Ok(Outcome::RESCHEDULE)
    );
    assert_eq!(state(&threads, first), ThreadState::Ready);
    assert_eq!(threads.get(first).unwrap().time_slice, 0);
    assert_eq!(scheduler.pick_next(&mut threads), Ok(second));
    assert_eq!(scheduler.pick_next(&mut threads), Ok(first));
}

#[test]
fn the_ready_bitmap_names_exactly_the_priorities_that_hold_someone() {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    assert_eq!(scheduler.ready_bitmap(), 0);
    let low = ready(&mut scheduler, &mut threads, 0);
    let high = ready(&mut scheduler, &mut threads, 31);
    assert_eq!(scheduler.ready_bitmap(), (1 << 31) | 1);
    assert_eq!(scheduler.highest_ready(), Some(31));
    scheduler.dequeue(&mut threads, high).unwrap();
    assert_eq!(scheduler.ready_bitmap(), 1);
    scheduler.dequeue(&mut threads, low).unwrap();
    assert_eq!(scheduler.ready_bitmap(), 0);
    assert!(scheduler.is_idle());
}

#[test]
fn a_queue_survives_a_thread_leaving_from_the_middle() {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    let first = ready(&mut scheduler, &mut threads, 4);
    let middle = ready(&mut scheduler, &mut threads, 4);
    let last = ready(&mut scheduler, &mut threads, 4);
    scheduler.dequeue(&mut threads, middle).unwrap();
    assert_eq!(scheduler.pick_next(&mut threads), Ok(first));
    assert_eq!(scheduler.pick_next(&mut threads), Ok(last));
    assert_eq!(scheduler.pick_next(&mut threads), Ok(first));
}

#[test]
fn the_default_scheduler_is_the_empty_one() {
    let scheduler = Scheduler::default();
    assert!(scheduler.is_idle());
    assert_eq!(scheduler.current(), None);
    assert_eq!(scheduler.idle(), None);
    assert_eq!(Outcome::default(), Outcome::NOTHING);
}

#[test]
fn a_priority_outside_the_table_has_no_queue() {
    // `Thread::new` refuses such a priority, so only a thread whose field
    // was written past it can reach this; the scheduler still answers
    // rather than reaching past its queues.
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    let id = add(&mut threads, 3);
    threads.get_mut(id).unwrap().priority = PRIORITY_COUNT + 8;
    assert_eq!(
        scheduler.enqueue(&mut threads, id),
        Err(Error::InvalidArgument)
    );
    assert_eq!(scheduler.dequeue(&mut threads, id), Ok(()));
    assert!(scheduler.is_idle());
}

#[test]
fn a_tick_for_a_thread_the_pool_lost_asks_for_a_switch() {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    let id = ready(&mut scheduler, &mut threads, 4);
    assert_eq!(scheduler.pick_next(&mut threads), Ok(id));
    threads.release(id).unwrap();
    assert_eq!(
        scheduler.tick(&mut threads),
        Outcome::RESCHEDULE,
        "the processor is running something the kernel no longer knows"
    );
}

#[test]
fn a_thread_that_became_ready_while_the_pool_lost_the_running_one_takes_over() {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    let running = ready(&mut scheduler, &mut threads, 4);
    assert_eq!(scheduler.pick_next(&mut threads), Ok(running));
    threads.release(running).unwrap();
    let fresh = add(&mut threads, 1);
    assert_eq!(
        scheduler.start(&mut threads, fresh),
        Ok(Outcome::RESCHEDULE),
        "whatever holds the processor cannot be asked about its priority"
    );
}

#[test]
fn changing_the_priority_of_a_thread_that_neither_runs_nor_waits_changes_nothing() {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    let other = ready(&mut scheduler, &mut threads, 6);
    assert_eq!(scheduler.pick_next(&mut threads), Ok(other));
    let sleeping = add(&mut threads, 2);
    scheduler.suspend(&mut threads, sleeping).unwrap();
    assert_eq!(
        scheduler.set_priority(&mut threads, sleeping, 9),
        Ok(Outcome::NOTHING),
        "a suspended thread takes nothing from the one that runs"
    );
    assert_eq!(threads.get(sleeping).unwrap().priority, 9);
    assert_eq!(scheduler.current(), Some(other));
}

#[test]
fn lowering_the_running_thread_with_nobody_waiting_changes_nothing() {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    let id = ready(&mut scheduler, &mut threads, 9);
    assert_eq!(scheduler.pick_next(&mut threads), Ok(id));
    assert_eq!(
        scheduler.set_priority(&mut threads, id, 1),
        Ok(Outcome::NOTHING)
    );
    assert_eq!(threads.get(id).unwrap().priority, 1);
}

#[test]
fn setting_the_priority_of_a_thread_the_pool_does_not_hold_fails() {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    let ghost: ThreadId = ObjectId::new(4, 1);
    assert_eq!(
        scheduler.set_priority(&mut threads, ghost, 1),
        Err(Error::InvalidHandle)
    );
    assert_eq!(
        scheduler.on_wake(&mut threads, ghost),
        Err(Error::InvalidHandle)
    );
    assert_eq!(
        scheduler.suspend(&mut threads, ghost),
        Err(Error::InvalidHandle)
    );
}

/// Every event that takes a thread off the processor, and the state a
/// ready thread would have to be in for the table to allow it. A ready
/// thread is the only kind that stands in a queue, so it is the only kind
/// a refused event could strand outside one.
const LEAVING: &[(Event, bool)] = &[
    (Event::Suspend, true),
    (Event::Exit, true),
    (Event::Fault, false),
    (Event::BlockSend, false),
    (Event::BlockRecv, false),
    (Event::BlockReply, false),
    (Event::BlockNotification, false),
];

/// Applies `event` to `id` through the method that owns it.
fn leave(
    scheduler: &mut Scheduler,
    threads: &mut Threads,
    id: ThreadId,
    event: Event,
) -> Result<Outcome, Error> {
    match event {
        Event::Suspend => scheduler.suspend(threads, id),
        Event::Exit => scheduler.exit(threads, id),
        Event::Fault => scheduler.fault(threads, id),
        other => scheduler.on_block(threads, id, other),
    }
}

#[test]
fn the_table_decides_before_a_thread_leaves_its_queue() {
    for (event, allowed) in LEAVING {
        let mut threads = Threads::new();
        let mut scheduler = Scheduler::new();
        let idle = add(&mut threads, 0);
        scheduler.set_idle(idle);
        let waiting = ready(&mut scheduler, &mut threads, 4);
        let queued = scheduler.ready_bitmap();
        assert_ne!(queued, 0, "{event:?}: the thread waits in a queue");

        let outcome = leave(&mut scheduler, &mut threads, waiting, *event);

        if *allowed {
            assert!(outcome.is_ok(), "{event:?}: the table allows it from Ready");
            assert_ne!(state(&threads, waiting), ThreadState::Ready);
            assert_eq!(
                scheduler.ready_bitmap(),
                0,
                "{event:?}: a thread that left `Ready` leaves its queue with it"
            );
            continue;
        }

        assert_eq!(
            outcome,
            Err(Error::InvalidState),
            "{event:?}: the table has no row out of Ready"
        );
        assert_eq!(
            state(&threads, waiting),
            ThreadState::Ready,
            "{event:?}: a refused event changes no state"
        );
        assert_eq!(
            scheduler.ready_bitmap(),
            queued,
            "{event:?}: and takes nothing out of a queue"
        );
        assert_eq!(
            scheduler.pick_next(&mut threads),
            Ok(waiting),
            "{event:?}: the thread is still the one that runs next"
        );
    }
}

#[test]
fn a_refused_event_leaves_the_time_slice_of_the_running_thread_alone() {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    let idle = add(&mut threads, 0);
    scheduler.set_idle(idle);
    let waiting = ready(&mut scheduler, &mut threads, 4);
    let running = ready(&mut scheduler, &mut threads, 6);
    assert_eq!(scheduler.pick_next(&mut threads), Ok(running));
    let slice = threads.get(waiting).unwrap().time_slice;

    assert_eq!(
        scheduler.fault(&mut threads, waiting),
        Err(Error::InvalidState)
    );

    assert_eq!(
        threads.get(waiting).unwrap().time_slice,
        slice,
        "a fault the table refused spent no slice"
    );
    assert_eq!(
        scheduler.current(),
        Some(running),
        "and took nobody off the processor"
    );
}

#[test]
fn a_thread_the_pool_does_not_hold_leaves_no_queue() {
    let mut threads = Threads::new();
    let mut scheduler = Scheduler::new();
    let idle = add(&mut threads, 0);
    scheduler.set_idle(idle);
    let waiting = ready(&mut scheduler, &mut threads, 4);
    let stale: ThreadId = ObjectId::new(waiting.index(), waiting.generation() + 1);
    let queued = scheduler.ready_bitmap();

    for (event, _) in LEAVING {
        assert_eq!(
            leave(&mut scheduler, &mut threads, stale, *event),
            Err(Error::InvalidHandle),
            "{event:?}"
        );
    }

    assert_eq!(scheduler.ready_bitmap(), queued);
    assert_eq!(state(&threads, waiting), ThreadState::Ready);
}
