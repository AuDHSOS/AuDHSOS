// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What the two round trips of this system cost, in ticks of the
//! time-stamp counter.
//!
//! Two figures are wanted and one mechanism produces both. The entry of a
//! system call is the single point of the kernel that a round trip passes
//! through exactly once, so the difference between two entries of the same
//! call, made by a thread that does nothing else in between, is one round
//! trip and nothing more. `bench_yield` gives the shorter of the two: the
//! trap, the dispatch of the shortest call in the table, and the return to
//! ring three. `bench_caller` with `bench_replier` gives the longer one: a
//! message sent, a receiver woken, an answer, and the two switches between
//! them.
//!
//! What the numbers are not: a figure of the hardware. The reference
//! machine is QEMU without hardware virtualization, and every instruction
//! of the measured path is translated. The figures compare with each other
//! and with themselves across a change of this kernel; they do not compare
//! with a processor. [D-102](../../../../docs/09-decisions.md) says what
//! was decided from them.
//!
//! The measurement is inside what it measures: every round trip carries
//! the lookup of the hook and one `rdtsc`. That cost is the same in every
//! sample and is not subtracted, because a figure that has had something
//! taken out of it is no longer a figure of anything that ran.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_hal_x86_64::testing::run_tests)]
#![reexport_test_harness_main = "test_main"]

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use audhsos_abi::{Rights, Syscall};
use kernel_hal_x86_64::instructions::read_tsc;
use kernel_hal_x86_64::testing;
use kernel_objects::object::AnyObjectId;

use crate::support::say;

mod support;

kernel_hal_x86_64::test_kernel!();

/// The thread that enters the kernel and comes straight back out.
static BENCH_YIELD: &[u8] =
    include_bytes!(concat!(env!("AUDHSOS_USER_TESTS_DIR"), "/bench_yield.bin"));

/// The calling half of the pair.
static BENCH_CALLER: &[u8] =
    include_bytes!(concat!(env!("AUDHSOS_USER_TESTS_DIR"), "/bench_caller.bin"));

/// The answering half of the pair.
static BENCH_REPLIER: &[u8] = include_bytes!(concat!(
    env!("AUDHSOS_USER_TESTS_DIR"),
    "/bench_replier.bin"
));

/// How many round trips each figure is the median of.
const SAMPLES: usize = 10_000;

/// How many calls a thread has to make for [`SAMPLES`] differences to
/// stand between them.
#[expect(
    clippy::as_conversions,
    reason = "a const fn has no TryFrom, and the count is a literal that fits either width"
)]
const fn rounds() -> u64 {
    (SAMPLES as u64).saturating_add(1)
}

/// The rights the calling half holds on the endpoint.
const CALLING: Rights = Rights::SEND;

/// The rights the answering half holds on it.
const ANSWERING: Rights = Rights::RECV;

/// A call number no thread makes, which is what the hook holds while
/// nothing is being measured.
const NO_CALL: u64 = u64::MAX;

/// The differences, in ticks, one per round trip.
///
/// Every one of these is touched by the kernel of one processor inside the
/// system call gate and by nobody else, so the ordering is the loosest one
/// there is: a locked instruction here would stand in every figure the
/// image writes.
static DIFFERENCES: [AtomicU32; SAMPLES] = [const { AtomicU32::new(0) }; SAMPLES];

/// How many of [`DIFFERENCES`] hold one.
static TAKEN: AtomicU32 = AtomicU32::new(0);

/// How many differences have been taken so far.
fn taken() -> usize {
    usize::try_from(TAKEN.load(Ordering::Relaxed)).unwrap_or(0)
}

/// The counter at the entry before this one, or zero before the first.
static PREVIOUS: AtomicU64 = AtomicU64::new(0);

/// The counter at the first entry of the measurement.
static FIRST: AtomicU64 = AtomicU64::new(0);

/// The counter at the entry that closed the last difference taken.
static LAST: AtomicU64 = AtomicU64::new(0);

/// The call the differences are taken between.
static WANTED: AtomicU64 = AtomicU64::new(NO_CALL);

/// Takes the difference of two entries of the wanted call.
///
/// The counter is read before anything is decided, so that every sample
/// holds the same work of this function and none holds the branch that
/// rejected the call before it.
fn on_call(number: u64) {
    let now = read_tsc();
    if number != WANTED.load(Ordering::Relaxed) {
        return;
    }
    let previous = PREVIOUS.swap(now, Ordering::Relaxed);
    if previous == 0 {
        FIRST.store(now, Ordering::Relaxed);
        return;
    }
    let at = usize::try_from(TAKEN.load(Ordering::Relaxed)).unwrap_or(usize::MAX);
    let Some(slot) = DIFFERENCES.get(at) else {
        return;
    };
    let difference = now.saturating_sub(previous);
    slot.store(
        u32::try_from(difference).unwrap_or(u32::MAX),
        Ordering::Relaxed,
    );
    TAKEN.store(
        u32::try_from(at.saturating_add(1)).unwrap_or(u32::MAX),
        Ordering::Relaxed,
    );
    LAST.store(now, Ordering::Relaxed);
}

/// Starts a measurement of the round trips of `call`.
fn measuring(call: Syscall) {
    TAKEN.store(0, Ordering::Relaxed);
    PREVIOUS.store(0, Ordering::Relaxed);
    FIRST.store(0, Ordering::Relaxed);
    LAST.store(0, Ordering::Relaxed);
    WANTED.store(u64::from(call.number()), Ordering::Relaxed);
}

/// Ends it, so that the calls of whatever runs next are not counted.
fn measured() -> usize {
    WANTED.store(NO_CALL, Ordering::Relaxed);
    taken()
}

/// The whole span of the measurement divided by the round trips in it.
///
/// The counter of the reference machine does not advance by one: under
/// emulation it steps, and the step is wider than the shorter of the two
/// round trips. A single difference is therefore quantized, and the median
/// of ten thousand of them is quantized with it. This figure is not: it is
/// one difference, taken across the whole run, and its error is the step
/// divided by the number of round trips. The median says what a round trip
/// typically was; this says what they cost together.
fn mean(len: usize) -> u64 {
    let span = LAST
        .load(Ordering::Relaxed)
        .saturating_sub(FIRST.load(Ordering::Relaxed));
    u64::try_from(len)
        .ok()
        .and_then(|count| span.checked_div(count))
        .unwrap_or(0)
}

/// How many of the first `len` differences are at most `value`.
fn at_most(value: u32, len: usize) -> usize {
    DIFFERENCES
        .iter()
        .take(len)
        .filter(|slot| slot.load(Ordering::Relaxed) <= value)
        .count()
}

/// The largest of the first `len` differences.
fn largest(len: usize) -> u32 {
    DIFFERENCES
        .iter()
        .take(len)
        .map(|slot| slot.load(Ordering::Relaxed))
        .max()
        .unwrap_or(0)
}

/// The smallest of them.
fn smallest(len: usize) -> u32 {
    DIFFERENCES
        .iter()
        .take(len)
        .map(|slot| slot.load(Ordering::Relaxed))
        .min()
        .unwrap_or(0)
}

/// The median of the first `len` differences: the smallest value that at
/// least half of them are at most.
///
/// It is found by halving the range of values rather than by sorting.
/// Sorting ten thousand differences in a kernel that has no allocator
/// would be a hundred million comparisons on an emulated processor; this
/// is thirty-two passes over the differences, and it moves nothing.
fn median(len: usize) -> u32 {
    if len == 0 {
        return 0;
    }
    let wanted = len.div_ceil(2);
    let mut low: u32 = 0;
    let mut high: u32 = largest(len);
    while low < high {
        let middle = low.saturating_add(high.saturating_sub(low) / 2);
        if at_most(middle, len) >= wanted {
            high = middle;
        } else {
            low = middle.saturating_add(1);
        }
    }
    low
}

/// Writes the figure of one measurement, and what its ends were.
fn report(name: &str, len: usize) {
    if len != SAMPLES {
        testing::fail(format_args!(
            "{name}: {len} round trips of {SAMPLES} were measured"
        ));
    }
    let middle = median(len);
    if middle == 0 {
        testing::fail(format_args!(
            "{name}: the median is zero, so the counter did not move"
        ));
    }
    if middle == u32::MAX {
        testing::fail(format_args!(
            "{name}: the median is the widest a difference can be, so one of them overflowed"
        ));
    }
    say!(
        "{name}: shortest {} ticks, longest {} ticks, {} ticks a round trip over the whole run",
        smallest(len),
        largest(len),
        mean(len)
    );
    testing::measure(name, u64::from(middle), u32::try_from(len).unwrap_or(0));
}

/// The shortest round trip: into the kernel and back out, ten thousand
/// times, with nothing in between.
#[test_case]
fn a_system_call_round_trip_takes_the_ticks_it_takes() {
    support::bring_up();
    support::set_call_hook(on_call);
    let mut process = support::create_process(BENCH_YIELD);
    let spawned = support::add_thread(&mut process, support::DEFAULT_PRIORITY);
    support::set_buffer_word(spawned.buffer, 0, rounds());
    measuring(Syscall::ThreadYield);
    support::start(spawned.thread);
    support::run_until(|| taken() >= SAMPLES);
    let len = measured();
    report("bench::syscall_round_trip", len);
}

/// The round trip of a call and its answer: two threads, an empty message,
/// and the two switches between them.
#[test_case]
fn an_ipc_round_trip_takes_the_ticks_it_takes() {
    support::bring_up();
    support::set_call_hook(on_call);
    let endpoint = support::endpoint();

    let mut replier = support::create_process(BENCH_REPLIER);
    let answering = support::add_thread(&mut replier, support::DEFAULT_PRIORITY);
    let received = support::install(replier.id, AnyObjectId::of(endpoint), ANSWERING);
    support::set_buffer_word(answering.buffer, 0, received.raw());

    let mut caller = support::create_process(BENCH_CALLER);
    let calling = support::add_thread(&mut caller, support::DEFAULT_PRIORITY);
    let sent = support::install(caller.id, AnyObjectId::of(endpoint), CALLING);
    support::set_buffer_word(calling.buffer, 0, sent.raw());
    support::set_buffer_word(calling.buffer, 1, rounds());

    measuring(Syscall::IpcCall);
    support::start(answering.thread);
    support::start(calling.thread);
    support::run_until(|| taken() >= SAMPLES);
    let len = measured();
    report("bench::ipc_round_trip", len);
}
