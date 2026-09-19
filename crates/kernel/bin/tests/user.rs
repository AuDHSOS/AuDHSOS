// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The first user thread of this system: a program of its own, in an
//! address space of its own, at privilege level three, asking the kernel
//! for something through the one gate it may enter.
//!
//! The image builds a process by hand, the way the root task will be built
//! in Phase 7: an address space that carries the kernel half, the flat
//! binary of a test program mapped read and execute, a stack, and a thread
//! whose kernel stack carries the frame the first switch returns through.
//! Then it switches into it and waits to be switched back. All of that is
//! `support`, which the other Phase 5 images share.
//!
//! What the tests watch is what came back: the state of the thread, what
//! it left in its own IPC buffer, and what the kernel did with the call it
//! made. Nothing here reads a register.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_hal_x86_64::testing::run_tests)]
#![reexport_test_harness_main = "test_main"]

use kernel_hal_x86_64::testing;

use crate::support::say;

mod support;

kernel_hal_x86_64::test_kernel!();

/// The program that writes a mark into its buffer, gives up the processor
/// once, and ends itself.
static COUNT_AND_EXIT: &[u8] = include_bytes!(concat!(
    env!("AUDHSOS_USER_TESTS_DIR"),
    "/count_and_exit.bin"
));

/// The program that ends itself and nothing else.
static THREAD_EXIT: &[u8] =
    include_bytes!(concat!(env!("AUDHSOS_USER_TESTS_DIR"), "/thread_exit.bin"));

/// The program that reports the registers it started with.
static ENTRY_REGISTERS: &[u8] = include_bytes!(concat!(
    env!("AUDHSOS_USER_TESTS_DIR"),
    "/entry_registers.bin"
));

/// What `entry_registers` leaves in the second payload word of its buffer.
const REGISTERS_MARK: u64 = 0x5EED_0002;

/// What `count_and_exit` leaves in the first payload word of its buffer.
const MARK: u64 = 0x5EED_0001;

/// The first user thread of this system runs its own program in its own
/// address space, writes into its own buffer, and ends itself.
#[test_case]
fn a_user_thread_runs_its_own_program_and_ends_itself() {
    support::bring_up();
    let spawned = support::spawn(COUNT_AND_EXIT);
    say!("switching into user mode");
    support::run_threads(None);
    say!("back in the kernel");

    let mark = support::buffer_word(spawned.buffer, 0);
    if mark != MARK {
        testing::fail(format_args!(
            "the thread left {mark:#x} in its buffer, not {MARK:#x}"
        ));
    }
    say!("the thread wrote {mark:#x} into its own buffer");
    let state = support::state_of(spawned.thread);
    if state.is_some() {
        testing::fail(format_args!("the thread is still in the pool: {state:?}"));
    }
    say!("the thread ended and the kernel cleared it away");
    let _ = spawned.process;
}

/// A second process runs in the same machine after the first one ended.
#[test_case]
fn a_second_thread_runs_after_the_first_one_ended() {
    support::bring_up();
    let spawned = support::spawn(THREAD_EXIT);
    support::run_threads(None);
    if support::state_of(spawned.thread).is_some() {
        testing::fail(format_args!("the second thread did not end"));
    }
    say!("a second process ran and ended in the same machine");
}

/// A new thread starts with every general register but the one that
/// carries its buffer address at zero.
///
/// Regression test for issue #82: the trampoline used to run `iretq` with
/// `rsi` holding the kernel stack address `switch_to` was given, and with
/// `rax`, `rcx`, `rdx` and `r8` to `r11` holding what the kernel left.
#[test_case]
fn a_user_thread_starts_with_no_kernel_register() {
    support::bring_up();
    let spawned = support::spawn(ENTRY_REGISTERS);
    support::run_threads(None);

    let mark = support::buffer_word(spawned.buffer, 1);
    if mark != REGISTERS_MARK {
        testing::fail(format_args!(
            "the thread left {mark:#x} in its buffer, not {REGISTERS_MARK:#x}"
        ));
    }
    let combined = support::buffer_word(spawned.buffer, 0);
    if combined != 0 {
        testing::fail(format_args!(
            "the thread started with {combined:#x} in its registers, not 0"
        ));
    }
    say!("the thread started with every register but its buffer address at zero");
    let _ = spawned.process;
}
