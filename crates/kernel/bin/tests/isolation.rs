// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What a user thread cannot do, and what happens to it when it tries.
//!
//! Two programs go looking for the boundary: one reads an address of the
//! kernel half, which its page tables carry but not for ring three, and
//! one runs `hlt`, which only ring zero may. The processor stops both, the
//! kernel stops the thread, and the machine goes on — which is the whole
//! point, and what every test here checks after the fault: another thread
//! is started, runs, and ends in the same machine.
//!
//! Phase 6 turns the same two faults into a message on the fault handler
//! endpoint of the process; this is the isolation item of
//! [6.6.21](../../../docs/06-testing-strategy.md) that names no handler.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_hal_x86_64::testing::run_tests)]
#![reexport_test_harness_main = "test_main"]

use audhsos_abi::ThreadState;
use kernel_hal_x86_64::testing;
use kernel_objects::object::ThreadId;

use crate::support::say;

mod support;

kernel_hal_x86_64::test_kernel!();

/// The program that reads the first byte of the kernel image.
static READ_KERNEL_MEMORY: &[u8] = include_bytes!(concat!(
    env!("AUDHSOS_USER_TESTS_DIR"),
    "/read_kernel_memory.bin"
));

/// The program that halts, which a user thread may not do.
static HLT_IN_USER: &[u8] =
    include_bytes!(concat!(env!("AUDHSOS_USER_TESTS_DIR"), "/hlt_in_user.bin"));

/// The program that ends itself and nothing else, which is what runs after
/// a fault to show that the machine still runs threads.
static THREAD_EXIT: &[u8] =
    include_bytes!(concat!(env!("AUDHSOS_USER_TESTS_DIR"), "/thread_exit.bin"));

/// The vector of a page fault.
const PAGE_FAULT: u32 = 14;

/// The vector of a general protection fault.
const GENERAL_PROTECTION: u32 = 13;

/// Runs `program` in a thread of its own until it faults, and returns that
/// thread. Fails the image when the thread came back without faulting.
fn until_it_faults(program: &[u8], what: &str) -> ThreadId {
    support::bring_up();
    let before = support::faults();
    let spawned = support::spawn(program);
    say!("switching into a thread that {what}");
    support::run_threads(None);
    say!("back in the kernel");
    if support::faults() != before.saturating_add(1) {
        testing::fail(format_args!(
            "the thread that {what} raised no fault: {} before, {} after",
            before,
            support::faults()
        ));
    }
    spawned.thread
}

/// The thread stopped where a faulted thread stops, and it still holds
/// everything it held.
fn assert_faulted(thread: ThreadId, what: &str) {
    match support::state_of(thread) {
        Some(ThreadState::Faulted) => {}
        Some(state) => testing::fail(format_args!(
            "the thread that {what} is {state:?}, not Faulted"
        )),
        None => testing::fail(format_args!(
            "the thread that {what} was cleared away; a faulted thread keeps its slot"
        )),
    }
    say!("the thread that {what} stopped in Faulted and kept its slot");
}

/// The machine runs threads after a fault: a new process starts, runs, and
/// ends.
fn assert_the_machine_runs_on() {
    let after = support::spawn(THREAD_EXIT);
    support::run_threads(None);
    if support::state_of(after.thread).is_some() {
        testing::fail(format_args!(
            "the thread started after the fault did not end"
        ));
    }
    say!("a thread started after the fault ran and ended");
}

/// A user thread that reads a kernel address faults, stops in `Faulted`,
/// and leaves the rest of the system running.
#[test_case]
fn a_thread_that_reads_kernel_memory_faults_and_the_machine_runs_on() {
    let thread = until_it_faults(READ_KERNEL_MEMORY, "reads the kernel half");
    assert_faulted(thread, "reads the kernel half");
    assert_the_machine_runs_on();
}

/// A user thread that executes `hlt` faults the same way.
#[test_case]
fn a_thread_that_halts_faults_and_the_machine_runs_on() {
    let thread = until_it_faults(HLT_IN_USER, "halts");
    assert_faulted(thread, "halts");
    assert_the_machine_runs_on();
}

/// The two faults are the two the processor raises for them: a page fault
/// for the read of a page that is mapped but not for ring three, a general
/// protection fault for the instruction that ring three may not run.
#[test_case]
fn the_two_faults_are_the_two_vectors_the_processor_raises() {
    support::bring_up();
    let read = support::spawn(READ_KERNEL_MEMORY);
    support::run_threads(None);
    let read_vector = support::last_fault().vector;

    let halt = support::spawn(HLT_IN_USER);
    support::run_threads(None);
    let halt_vector = support::last_fault().vector;

    if read_vector != PAGE_FAULT {
        testing::fail(format_args!(
            "reading the kernel half raised vector {read_vector}, not {PAGE_FAULT}"
        ));
    }
    if halt_vector != GENERAL_PROTECTION {
        testing::fail(format_args!(
            "halting raised vector {halt_vector}, not {GENERAL_PROTECTION}"
        ));
    }
    say!("the read faulted with vector {read_vector}, the halt with vector {halt_vector}");
    let _ = (read.thread, halt.thread);
}

/// Two threads that fault one after the other are both stopped: the kernel
/// reports the second fault as readily as the first, and neither of them
/// takes the machine with it.
#[test_case]
fn a_second_fault_is_handled_like_the_first() {
    support::bring_up();
    let first = support::spawn(READ_KERNEL_MEMORY);
    support::run_threads(None);
    let second = support::spawn(HLT_IN_USER);
    support::run_threads(None);

    assert_faulted(first.thread, "read the kernel half first");
    assert_faulted(second.thread, "halted second");
    assert_the_machine_runs_on();
}

/// A faulted thread is not a thread the reaper takes: its kernel stack,
/// its buffer, and its slot stay where they are, because whoever created
/// it may still want to look at it.
#[test_case]
fn a_faulted_thread_keeps_what_it_holds() {
    let thread = until_it_faults(HLT_IN_USER, "halts");
    support::sweep();
    assert_faulted(thread, "halts");
}
