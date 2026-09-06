// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The wrappers of the gate against the table they claim to cover.
//!
//! `Gate` carries one method per system call, written out by hand (D-92),
//! and the constant assertion beside them holds a list to the table: it
//! cannot see whether a method exists for each entry, nor whether a method
//! passes the entry it belongs to. Two wrappers that swapped their calls
//! would pass it, and so would a call of the same arity written in the
//! wrong method — the kernel's argument-count check would not notice
//! either.
//!
//! What notices is this image. `every_wrapper` calls all forty-two methods
//! in the order of the table, and the kernel writes down the number each
//! call arrived under. The number is what the wrapper wrote into the
//! buffer, so the sequence the kernel saw is the sequence of calls the
//! wrappers made, and it has to be the table.
//!
//! `thread_exit` is the exception of the order and not of the check: it
//! does not come back, so the program makes it last and the expected
//! sequence is the table with that one call moved to the end.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_hal_x86_64::testing::run_tests)]
#![reexport_test_harness_main = "test_main"]

use audhsos_abi::{Rights, Syscall};
use kernel_hal_x86_64::testing;
use kernel_objects::object::AnyObjectId;

use crate::support::say;

mod support;

kernel_hal_x86_64::test_kernel!();

/// The program that calls every wrapper.
static EVERY_WRAPPER: &[u8] = include_bytes!(concat!(
    env!("AUDHSOS_USER_TESTS_DIR"),
    "/every_wrapper.bin"
));

/// Whether the run has been made; a second test reads the same numbers.
static DONE: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// The numbers the kernel saw, once the run has been made.
static SEEN: audhsos_sync::Global<Seen> = audhsos_sync::Global::new();

/// What the kernel saw, as a run of system call numbers.
struct Seen {
    numbers: [u32; support::WATCH_CAPACITY],
    len: usize,
}

/// Runs the program once and keeps what the kernel saw.
fn run_once() {
    if DONE.swap(1, core::sync::atomic::Ordering::SeqCst) == 1 {
        return;
    }
    support::bring_up();
    // The interrupt controller and the ports, without the timer: the image
    // wants the calls that touch them to reach a machine, and no
    // preemption in the middle of a run whose order is the whole point.
    support::bring_up_devices();
    let mut process = support::create_process(EVERY_WRAPPER);
    let spawned = support::add_thread(&mut process, support::DEFAULT_PRIORITY);
    // The program needs no handle of its own: every call it makes is made
    // to be refused, and the one handle it uses names nothing. What it
    // does need is room in its table for the three calls that succeed.
    let _own = support::install(
        process.id,
        AnyObjectId::of(process.id),
        Rights::MANAGE | Rights::DUPLICATE,
    );

    support::watch_syscalls();
    support::start(spawned.thread);
    say!("switching into a thread that calls every wrapper of the gate");
    support::run_threads(None);
    say!("back in the kernel");

    if support::state_of(spawned.thread).is_some() {
        testing::fail(format_args!("the thread did not end"));
    }
    let mut numbers = [0u32; support::WATCH_CAPACITY];
    let len = support::watched_syscalls(&mut numbers);
    if SEEN.init(Seen { numbers, len }).is_err() {
        testing::fail(format_args!("the numbers are already in place"));
    }
}

/// Runs `body` with what the kernel saw.
fn with_seen(body: impl FnOnce(&Seen)) {
    run_once();
    let Ok(seen) = SEEN.borrow(&audhsos_sync::UncontendedToken) else {
        testing::fail(format_args!("the numbers are not reachable"));
    };
    body(&seen);
}

/// The table in the order the program calls it: every entry but
/// `thread_exit`, and then `thread_exit`.
fn expected(index: usize) -> Option<Syscall> {
    let rest: usize = Syscall::ALL.len().saturating_sub(1);
    if index == rest {
        return Some(Syscall::ThreadExit);
    }
    Syscall::ALL
        .iter()
        .filter(|call| **call != Syscall::ThreadExit)
        .nth(index)
        .copied()
}

/// The gate made as many calls as the table has entries: one wrapper for
/// each, and no wrapper twice.
#[test_case]
fn the_gate_made_exactly_as_many_calls_as_the_table_has() {
    with_seen(|seen| {
        if seen.len != Syscall::ALL.len() {
            testing::fail(format_args!(
                "the kernel saw {} calls, the table has {}",
                seen.len,
                Syscall::ALL.len()
            ));
        }
    });
}

/// Every wrapper passed the entry it belongs to, which is what the
/// constant assertion beside them cannot see.
#[test_case]
fn every_wrapper_passed_the_call_it_belongs_to() {
    with_seen(|seen| {
        for index in 0..seen.len {
            let Some(wanted) = expected(index) else {
                testing::fail(format_args!("call {index} is past the table"));
            };
            let Some(number) = seen.numbers.get(index) else {
                testing::fail(format_args!("call {index} was not written down"));
            };
            if u64::from(*number) != u64::from(wanted.number()) {
                let name = Syscall::from_word(u64::from(*number))
                    .map_or("no call of the table", Syscall::name);
                testing::fail(format_args!(
                    "wrapper {index} passed number {number} ({name}), and the table has {} ({})",
                    wanted.number(),
                    wanted.name()
                ));
            }
        }
    });
}

/// No call of the table is without a wrapper that reached the kernel.
#[test_case]
fn no_call_of_the_table_was_left_out() {
    with_seen(|seen| {
        for call in Syscall::ALL {
            let number = u32::from(call.number());
            if !seen
                .numbers
                .iter()
                .take(seen.len)
                .any(|seen| *seen == number)
            {
                testing::fail(format_args!(
                    "{} (number {}) has no wrapper that reached the kernel",
                    call.name(),
                    call.number()
                ));
            }
        }
    });
}
