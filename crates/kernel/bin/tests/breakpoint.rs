// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A breakpoint reaches the handler and returns to the next instruction.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_hal_x86_64::testing::run_tests)]
#![reexport_test_harness_main = "test_main"]

// The kernel image uses these crates; a test image does not.
// The user test image uses these; this one does not.
use audhsos_sync as _;
use kernel_ipc as _;
use kernel_objects as _;
use kernel_syscall as _;

use audhsos_abi as _;
use kernel_core as _;
// The memory test image uses these crates; the other test images do not.
use kernel_hal_api as _;
use kernel_mm as _;
use kernel_types as _;

use core::sync::atomic::{AtomicU8, Ordering};

use kernel_hal_x86_64::testing;
use kernel_hal_x86_64::traps::TrapReport;

kernel_hal_x86_64::test_kernel!();

/// The vector the handler saw, or zero.
static SEEN: AtomicU8 = AtomicU8::new(0);

/// Records the vector and returns, so that the processor resumes.
fn on_breakpoint(report: TrapReport) {
    SEEN.store(report.vector, Ordering::SeqCst);
}

/// The handler sees vector three and the test goes on.
#[test_case]
fn a_breakpoint_returns_to_the_next_instruction() {
    testing::set_trap_hook(on_breakpoint);
    testing::raise_breakpoint();
    assert_eq!(SEEN.load(Ordering::SeqCst), 3);
}
