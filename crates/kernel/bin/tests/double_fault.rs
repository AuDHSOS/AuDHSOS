// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A kernel stack overflow lands on the double fault stack. That the
//! handler runs at all is the test: without its own stack the processor
//! would triple fault and the machine would be gone.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_hal_x86_64::testing::run_tests)]
#![reexport_test_harness_main = "test_main"]

// The kernel image uses these crates; a test image does not.
use audhsos_abi as _;
use kernel_core as _;

use kernel_hal_x86_64::testing;
use kernel_hal_x86_64::traps::TrapReport;

kernel_hal_x86_64::test_kernel!();

/// The vector a fault inside a fault handler raises.
const DOUBLE_FAULT: u8 = 8;

fn on_double_fault(report: TrapReport) -> ! {
    if report.vector != DOUBLE_FAULT {
        testing::fail(format_args!(
            "trap {} instead of a double fault",
            report.vector
        ));
    }
    testing::pass();
    testing::finish()
}

/// Recurses until the stack reaches its guard page. `black_box` keeps the
/// compiler from turning the recursion into a loop.
#[expect(
    unconditional_recursion,
    reason = "the endless recursion is the test: it overflows the kernel stack"
)]
fn overflow(depth: u64) -> u64 {
    let next = core::hint::black_box(depth).wrapping_add(1);
    core::hint::black_box(overflow(next)).wrapping_add(next)
}

/// The overflow reaches the guard page and the machine ends.
#[test_case]
fn a_kernel_stack_overflow_ends_in_the_double_fault_handler() {
    testing::set_trap_hook(|report| on_double_fault(report));
    let _ = overflow(0);
}
