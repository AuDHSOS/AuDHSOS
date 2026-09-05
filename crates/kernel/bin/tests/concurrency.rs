// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Two threads of equal priority take turns.
//!
//! One process, one program, one page they share, and two threads that
//! take tickets from a counter in that page: read it, write it back one
//! higher, write the ticket down in their own buffer, and give up the
//! processor. A run queue that hands the processor to the thread that has
//! waited longest gives the two of them alternate tickets, and the two
//! buffers are what the tests read back.
//!
//! No timer runs here, so nothing takes the processor from a thread that
//! did not ask to give it up: what the tests see is the order of the run
//! queue and nothing else. `tests/preemption.rs` is the other half, where
//! the timer decides.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_hal_x86_64::testing::run_tests)]
#![reexport_test_harness_main = "test_main"]

use kernel_hal_x86_64::testing;
use kernel_types::PhysFrame;

use crate::support::say;

mod support;

kernel_hal_x86_64::test_kernel!();

/// The program both threads run.
static TWO_THREADS: &[u8] =
    include_bytes!(concat!(env!("AUDHSOS_USER_TESTS_DIR"), "/two_threads.bin"));

/// The payload word the kernel leaves the address of the shared page in.
/// `two_threads.rs` reads it there.
const SHARED_WORD: usize = 0;

/// The payload word the first ticket goes into.
const FIRST_TICKET: usize = 1;

/// How many tickets each thread takes.
const ROUNDS: usize = 4;

/// The two threads of the run, and what they left behind.
struct Run {
    /// The thread that was started first.
    first: support::Spawned,
    /// The thread that was started second.
    second: support::Spawned,
    /// The page they shared.
    shared: PhysFrame,
}

/// Builds a process with two threads of `priority` over a page they share,
/// runs them until both have ended, and returns what they left.
fn run_two(priority: u8) -> Run {
    support::bring_up();
    let mut process = support::create_process(TWO_THREADS);
    let shared = support::share_page(&process);
    let first = support::add_thread(&mut process, priority);
    let second = support::add_thread(&mut process, priority);
    for thread in [&first, &second] {
        support::set_buffer_word(thread.buffer, SHARED_WORD, support::SHARED_BASE);
    }
    support::start(first.thread);
    support::start(second.thread);
    say!("two threads of priority {priority} in one address space");
    support::run_threads(None);
    say!("back in the kernel");
    Run {
        first,
        second,
        shared,
    }
}

/// The tickets a thread wrote down, in the order it took them.
fn tickets(thread: &support::Spawned) -> [u64; ROUNDS] {
    let mut taken = [0_u64; ROUNDS];
    for (round, slot) in taken.iter_mut().enumerate() {
        *slot = support::buffer_word(thread.buffer, FIRST_TICKET.saturating_add(round));
    }
    taken
}

/// Both threads ran to the end of their rounds and ended themselves.
#[test_case]
fn two_threads_of_one_process_both_run_to_the_end() {
    let run = run_two(support::DEFAULT_PRIORITY);
    for (name, thread) in [("the first", run.first), ("the second", run.second)] {
        if let Some(state) = support::state_of(thread.thread) {
            testing::fail(format_args!("{name} thread is {state:?}, not gone"));
        }
    }
    let counter = support::page_word(run.shared, 0);
    let wanted = u64::try_from(ROUNDS.saturating_mul(2)).unwrap_or(0);
    if counter != wanted {
        testing::fail(format_args!(
            "the shared counter stands at {counter}, not {wanted}"
        ));
    }
    say!("both threads ended and the counter they share stands at {counter}");
}

/// The two threads took alternate tickets: the first took the even ones,
/// the second the odd ones. That is what a run queue which hands the
/// processor to the thread that has waited longest does with two threads
/// of one priority.
#[test_case]
fn two_threads_of_equal_priority_take_alternate_turns() {
    let run = run_two(support::DEFAULT_PRIORITY);
    let first = tickets(&run.first);
    let second = tickets(&run.second);
    for round in 0..ROUNDS {
        let even = u64::try_from(round.saturating_mul(2)).unwrap_or(0);
        let odd = even.saturating_add(1);
        if first.get(round).copied() != Some(even) {
            testing::fail(format_args!(
                "round {round}: the first thread took {:?}, not ticket {even}",
                first.get(round)
            ));
        }
        if second.get(round).copied() != Some(odd) {
            testing::fail(format_args!(
                "round {round}: the second thread took {:?}, not ticket {odd}",
                second.get(round)
            ));
        }
    }
    say!("the tickets alternate: {first:?} against {second:?}");
}

/// Every ticket the counter handed out was taken by exactly one thread:
/// the two of them never read the same value, because a thread that reads
/// and writes the counter is not interrupted while it does.
#[test_case]
fn no_ticket_is_handed_out_twice() {
    let run = run_two(support::DEFAULT_PRIORITY);
    let mut taken = [false; ROUNDS * 2];
    for thread in [&run.first, &run.second] {
        for ticket in tickets(thread) {
            let index = usize::try_from(ticket).unwrap_or(usize::MAX);
            match taken.get_mut(index) {
                Some(slot) if !*slot => *slot = true,
                Some(_) => testing::fail(format_args!("ticket {ticket} was handed out twice")),
                None => testing::fail(format_args!("ticket {ticket} was never handed out")),
            }
        }
    }
    say!("every one of the {} tickets was taken once", taken.len());
}

/// A thread of a higher priority runs to its end before a thread of a
/// lower one takes a single turn: the queues are looked at from the top
/// and a thread that yields goes to the end of the queue of its own
/// priority, not into someone else's.
#[test_case]
fn a_thread_of_higher_priority_takes_every_turn_first() {
    support::bring_up();
    let mut process = support::create_process(TWO_THREADS);
    let shared = support::share_page(&process);
    let low = support::add_thread(&mut process, 2);
    let high = support::add_thread(&mut process, 6);
    for thread in [&low, &high] {
        support::set_buffer_word(thread.buffer, SHARED_WORD, support::SHARED_BASE);
    }
    support::start(low.thread);
    support::start(high.thread);
    support::run_threads(None);

    let taken_high = tickets(&high);
    let taken_low = tickets(&low);
    let last_of_high = taken_high.last().copied().unwrap_or(u64::MAX);
    let first_of_low = taken_low.first().copied().unwrap_or(0);
    if last_of_high >= first_of_low {
        testing::fail(format_args!(
            "the high thread took {taken_high:?} and the low one {taken_low:?}: \
             the low thread ran before the high one was done"
        ));
    }
    if support::state_of(low.thread).is_some() || support::state_of(high.thread).is_some() {
        testing::fail(format_args!("a thread of the run did not end"));
    }
    let counter = support::page_word(shared, 0);
    let wanted = u64::try_from(ROUNDS.saturating_mul(2)).unwrap_or(0);
    if counter != wanted {
        testing::fail(format_args!(
            "the counter stands at {counter}, not {wanted}"
        ));
    }
    say!("the high thread took {taken_high:?}, then the low one took {taken_low:?}");
}

/// A thread that yields when it is the only one runnable gets the
/// processor straight back: yielding is not blocking.
#[test_case]
fn a_lone_thread_that_yields_runs_on() {
    support::bring_up();
    let mut process = support::create_process(TWO_THREADS);
    let shared = support::share_page(&process);
    let alone = support::add_thread(&mut process, support::DEFAULT_PRIORITY);
    support::set_buffer_word(alone.buffer, SHARED_WORD, support::SHARED_BASE);
    support::start(alone.thread);
    support::run_threads(None);

    let taken = tickets(&alone);
    for (round, ticket) in taken.iter().enumerate() {
        let wanted = u64::try_from(round).unwrap_or(0);
        if *ticket != wanted {
            testing::fail(format_args!(
                "round {round}: the lone thread took {ticket}, not {wanted}"
            ));
        }
    }
    if let Some(state) = support::state_of(alone.thread) {
        testing::fail(format_args!(
            "the lone thread is {state:?}: it never reached the end of its rounds"
        ));
    }
    let counter = support::page_word(shared, 0);
    let wanted = u64::try_from(ROUNDS).unwrap_or(0);
    if counter != wanted {
        testing::fail(format_args!(
            "the counter stands at {counter} after one thread took {wanted} tickets"
        ));
    }
    say!("a thread alone in its queue took every ticket in a row: {taken:?}");
}
