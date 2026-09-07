// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The window a tick can land in, and that a switch may not happen inside
//! it.
//!
//! Every other image runs its kernel work either before the timer or from
//! inside a gate, where interrupts are off. This one does it in the open
//! on purpose: it starts the timer and then builds process after process
//! with interrupts on, while a thread that wants nothing but `thread_exit`
//! is already runnable. That is the shape the `ipc` image had, where it
//! wedged about one run in six.
//!
//! What the wedge was: `create_process`, `share_page` and `add_thread`
//! take the memory out of its cell for the length of real work, and this
//! image's own thread is not inside a gate while they run. A switch from
//! there left the cell borrowed by a thread that was no longer running,
//! and the next system call found it gone — `answer` then did nothing at
//! all, so `thread_exit` never ended its thread, the thread never gave the
//! processor up, and the image never came back: sixty seconds of silence
//! and no summary line.
//!
//! The image passes when it comes back from every round and says so. It is
//! a test of chance and not of order — it caught the fault in about three
//! runs in four — so what it is worth is one run of it per check, over
//! many checks.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_hal_x86_64::testing::run_tests)]
#![reexport_test_harness_main = "test_main"]

use crate::support::say;

mod support;

kernel_hal_x86_64::test_kernel!();

/// The program whose first and only act is to end itself, which is the
/// shape the threads the `ipc` tests left behind were in.
static EXITER: &[u8] = include_bytes!(concat!(env!("AUDHSOS_USER_TESTS_DIR"), "/thread_exit.bin"));

/// How many processes are built after the timer has started. Each one is
/// an address space, a page, a kernel stack and a thread — the work the
/// `ipc` image did once, done often enough that a tick lands inside it.
const ROUNDS: usize = 12;

/// Builds processes and threads after the timer has started, with a thread
/// that ends itself already runnable. The image has to come back from
/// every round.
#[test_case]
fn kernel_work_after_the_timer_started_comes_back() {
    support::bring_up();

    // Something for the scheduler to hand the processor to.
    let mut runner = support::create_process(EXITER);
    let spawned = support::add_thread(&mut runner, support::DEFAULT_PRIORITY);
    support::start(spawned.thread);

    // From here a tick may arrive at any instruction.
    support::start_timer(|_ticks| false);
    for round in 0..ROUNDS {
        let mut extra = support::create_process(EXITER);
        let _page = support::share_page(&extra);
        let _thread = support::add_thread(&mut extra, support::DEFAULT_PRIORITY);
        say!("round {round} done");
    }
    say!("the kernel work after the timer finished");
}
