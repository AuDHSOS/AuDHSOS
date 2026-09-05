// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What the timer does to threads that never ask for anything.
//!
//! `spin` makes no system call at all: it counts in its own IPC buffer and
//! never stops. A thread like that keeps the processor until something
//! takes it away, so everything the tests here see is the scheduler acting
//! without the thread's consent — the time slice running out, and a thread
//! of higher priority becoming ready.
//!
//! The kernel writes down which thread the timer found on the processor,
//! one entry per turn, and that log is the evidence: it is the order the
//! processor went round, seen from outside the threads.
//!
//! A run is bounded in ticks and not in rounds of a loop, because how far
//! a user thread counts in ten milliseconds is a fact about the machine
//! the image runs on and not about the scheduler. The tick hook is what
//! ends it: threads that never end themselves are ended by the kernel.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_hal_x86_64::testing::run_tests)]
#![reexport_test_harness_main = "test_main"]

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use audhsos_abi::ThreadState;
use audhsos_abi::layout::DEFAULT_TIME_SLICE_TICKS;
use kernel_hal_x86_64::testing;
use kernel_objects::object::ThreadId;

use crate::support::{NO_THREAD_WORD, Spawned, say};

mod support;

kernel_hal_x86_64::test_kernel!();

/// The program that counts and never stops.
static SPIN: &[u8] = include_bytes!(concat!(env!("AUDHSOS_USER_TESTS_DIR"), "/spin.bin"));

/// The program that writes a mark into its buffer, yields once, and ends.
static COUNT_AND_EXIT: &[u8] = include_bytes!(concat!(
    env!("AUDHSOS_USER_TESTS_DIR"),
    "/count_and_exit.bin"
));

/// The payload word `spin` counts in.
const COUNT_WORD: usize = 0;

/// What `count_and_exit` leaves in the first payload word of its buffer.
const MARK: u64 = 0x5EED_0001;

/// How many time slices a run lasts. Six slices are five chances for the
/// scheduler to hand the processor on.
const SLICES: u64 = 6;

/// How many ticks that is.
#[expect(
    clippy::as_conversions,
    reason = "widening the time slice of the layout, in a constant"
)]
const RUN_TICKS: u64 = SLICES * DEFAULT_TIME_SLICE_TICKS as u64;

/// The tick a run ends at. A run that has not been set up yet ends at a
/// tick the timer will not reach, so that a tick between two runs finds
/// nothing to do.
static END_AT: AtomicU64 = AtomicU64::new(u64::MAX);

/// The tick the thread held back is started at; zero for none.
static START_AT: AtomicU64 = AtomicU64::new(0);

/// The thread the tick hook starts when that tick comes.
static TO_START: AtomicU64 = AtomicU64::new(NO_THREAD_WORD);

/// The count the displaced thread stood at when the held back thread was
/// started.
static DISPLACED_AT: AtomicU64 = AtomicU64::new(0);

/// The thread whose count that is.
static DISPLACED: AtomicU64 = AtomicU64::new(NO_THREAD_WORD);

/// The threads the tick hook ends when the run is over.
static TO_END: [AtomicU64; 2] = [const { AtomicU64::new(NO_THREAD_WORD) }; 2];

/// Whether the run has ended.
static DONE: AtomicU32 = AtomicU32::new(0);

/// Clears the plan of the last run and the log of turns. Every static this
/// image keeps is set here, so that a tick between two runs finds a plan
/// that asks for nothing.
fn prepare() {
    support::bring_up();
    END_AT.store(u64::MAX, Ordering::SeqCst);
    START_AT.store(0, Ordering::SeqCst);
    TO_START.store(NO_THREAD_WORD, Ordering::SeqCst);
    DISPLACED.store(NO_THREAD_WORD, Ordering::SeqCst);
    DISPLACED_AT.store(0, Ordering::SeqCst);
    for slot in &TO_END {
        slot.store(NO_THREAD_WORD, Ordering::SeqCst);
    }
    DONE.store(0, Ordering::SeqCst);
    support::forget_turns();
}

/// A process of its own with one thread of `priority`, created and not yet
/// started: nothing may run before the plan of the run is in place.
fn thread_of(program: &[u8], priority: u8) -> Spawned {
    let mut process = support::create_process(program);
    support::add_thread(&mut process, priority)
}

/// Names the threads the tick hook ends when the run is over.
fn end_these(threads: &[ThreadId]) {
    for (slot, thread) in TO_END.iter().zip(threads) {
        slot.store(support::pack(*thread), Ordering::SeqCst);
    }
}

/// Starts the timer and sets the end of the run `RUN_TICKS` from now.
/// Nothing of the run may be running yet.
fn begin_run() {
    support::start_timer(on_tick);
    END_AT.store(support::ticks().saturating_add(RUN_TICKS), Ordering::SeqCst);
}

/// Waits until the tick hook says the run is over.
fn finish_run() {
    support::idle_until(|| DONE.load(Ordering::SeqCst) == 1);
}

/// What the kernel does on every tick of a run: start the thread the plan
/// holds back, and end the run when its ticks are up.
///
/// An operation the machine is too busy for is left for the next tick: a
/// tick that arrives while the kernel holds the machine is a tick inside a
/// system call, and there is another one a millisecond later.
fn on_tick(ticks: u64) -> bool {
    let mut reschedule = false;
    let start_at = START_AT.load(Ordering::SeqCst);
    if start_at != 0 && ticks >= start_at {
        if let Some(thread) = support::unpack(TO_START.load(Ordering::SeqCst))
            && let Some(asked) = support::try_start(thread)
        {
            note_displaced();
            START_AT.store(0, Ordering::SeqCst);
            TO_START.store(NO_THREAD_WORD, Ordering::SeqCst);
            reschedule = asked || reschedule;
        }
    }
    if ticks < END_AT.load(Ordering::SeqCst) {
        return reschedule;
    }
    let mut left = false;
    for slot in &TO_END {
        let Some(thread) = support::unpack(slot.load(Ordering::SeqCst)) else {
            continue;
        };
        match support::try_kill(thread) {
            Some(asked) => {
                slot.store(NO_THREAD_WORD, Ordering::SeqCst);
                reschedule = asked || reschedule;
            }
            None => left = true,
        }
    }
    if !left {
        DONE.store(1, Ordering::SeqCst);
    }
    reschedule
}

/// Writes down how far the thread that is about to be displaced had
/// counted, which is the count it has to pass again once it gets the
/// processor back.
fn note_displaced() {
    if let Some(thread) = support::unpack(DISPLACED.load(Ordering::SeqCst)) {
        let count = support::thread_word(thread, COUNT_WORD);
        DISPLACED_AT.store(count, Ordering::SeqCst);
    }
}

/// The count a `spin` thread reached.
fn count_of(thread: &Spawned) -> u64 {
    support::buffer_word(thread.buffer, COUNT_WORD)
}

/// Two threads of equal priority that never give the processor up both get
/// it, again and again: the time slice runs out and the scheduler hands it
/// to the other one.
#[test_case]
fn the_time_slice_rotates_two_threads_that_never_yield() {
    prepare();
    let first = thread_of(SPIN, support::DEFAULT_PRIORITY);
    let second = thread_of(SPIN, support::DEFAULT_PRIORITY);
    end_these(&[first.thread, second.thread]);
    begin_run();
    support::start(first.thread);
    support::start(second.thread);
    finish_run();

    for (name, thread) in [("the first", &first), ("the second", &second)] {
        if count_of(thread) == 0 {
            testing::fail(format_args!("{name} thread never ran"));
        }
        if !support::took_a_turn(thread.thread) {
            testing::fail(format_args!("the timer never found {name} thread running"));
        }
    }
    let turns = support::turns_seen();
    if turns < 3 {
        testing::fail(format_args!(
            "the processor changed hands {turns} times in {RUN_TICKS} ticks; \
             two threads that never yield need it to change hands"
        ));
    }
    say!(
        "the processor changed hands {turns} times: {} against {}",
        count_of(&first),
        count_of(&second)
    );
}

/// A thread of higher priority that becomes ready takes the processor from
/// a thread of a lower one that asked for nothing and would not have given
/// it up.
///
/// That the high thread ran at all is the proof: the low thread makes no
/// system call, so nothing but the scheduler could have taken the
/// processor off it. That the low thread counts past where it stood when
/// the high one arrived is the other half: it was displaced, not ended.
#[test_case]
fn a_higher_priority_thread_takes_the_processor_from_a_lower_one() {
    prepare();
    let low = thread_of(SPIN, 2);
    let high = thread_of(COUNT_AND_EXIT, 6);
    end_these(&[low.thread]);
    DISPLACED.store(support::pack(low.thread), Ordering::SeqCst);
    TO_START.store(support::pack(high.thread), Ordering::SeqCst);
    begin_run();
    // Halfway through the run, so that the low thread has counted before
    // it is displaced and can count again afterwards.
    START_AT.store(
        support::ticks().saturating_add(RUN_TICKS / 2),
        Ordering::SeqCst,
    );
    support::start(low.thread);
    finish_run();

    let mark = support::buffer_word(high.buffer, 0);
    if mark != MARK {
        testing::fail(format_args!(
            "the high thread left {mark:#x} in its buffer, not {MARK:#x}: it never ran"
        ));
    }
    if support::state_of(high.thread).is_some() {
        testing::fail(format_args!("the high thread did not end"));
    }
    let displaced_at = DISPLACED_AT.load(Ordering::SeqCst);
    if displaced_at == 0 {
        testing::fail(format_args!(
            "the low thread had counted to nothing when the high one arrived"
        ));
    }
    let after = count_of(&low);
    if after <= displaced_at {
        testing::fail(format_args!(
            "the low thread stood at {displaced_at} when it was displaced and at {after} \
             at the end: it never got the processor back"
        ));
    }
    say!("the low thread was displaced at {displaced_at} and stood at {after} afterwards");
}

/// A thread the kernel ends while it is running leaves the processor and
/// everything it held; the machine goes on without it.
#[test_case]
fn a_thread_the_kernel_ends_while_it_runs_leaves_everything() {
    prepare();
    let spinning = thread_of(SPIN, support::DEFAULT_PRIORITY);
    end_these(&[spinning.thread]);
    begin_run();
    support::start(spinning.thread);
    finish_run();

    match support::state_of(spinning.thread) {
        None => {}
        Some(ThreadState::Exited) => {
            testing::fail(format_args!("the thread ended and was not cleared away"));
        }
        Some(state) => testing::fail(format_args!("the thread is {state:?}, not gone")),
    }
    say!("the thread the kernel ended is gone and the machine runs on");
}
