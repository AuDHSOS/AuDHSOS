// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A kernel stack overflow lands on the double fault stack, and the
//! handler reports it. The image expects that report instead of a pass.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_hal_x86_64::testing::run_tests)]
#![reexport_test_harness_main = "test_main"]

// The kernel image uses these crates; a test image does not.
use audhsos_abi as _;
use kernel_core as _;

kernel_hal_x86_64::test_kernel!(should_panic);

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
    let _ = overflow(0);
}
