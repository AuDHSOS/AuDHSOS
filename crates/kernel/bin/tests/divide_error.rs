// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A division by zero reaches the divide error handler.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_hal_x86_64::testing::run_tests)]
#![reexport_test_harness_main = "test_main"]

// The kernel image uses these crates; a test image does not.
// The user test image uses these; this one does not.
use audhsos_sync as _;
use kernel_objects as _;
use kernel_syscall as _;

use audhsos_abi as _;
use kernel_core as _;
// The memory test image uses these crates; the other test images do not.
use kernel_hal_api as _;
use kernel_mm as _;
use kernel_types as _;

use kernel_hal_x86_64::testing;
use kernel_hal_x86_64::traps::TrapReport;

kernel_hal_x86_64::test_kernel!();

/// The vector a division by zero raises.
const DIVIDE_ERROR: u8 = 0;

/// Ends the machine: the exception is a fault, so returning would run the
/// division again.
fn on_divide_error(report: TrapReport) -> ! {
    if report.vector != DIVIDE_ERROR {
        testing::fail(format_args!(
            "trap {} instead of a divide error",
            report.vector
        ));
    }
    testing::pass();
    testing::finish()
}

/// The handler sees vector zero and ends the machine.
#[test_case]
fn a_division_by_zero_raises_the_divide_error_exception() {
    testing::set_trap_hook(|report| on_divide_error(report));
    testing::raise_divide_error();
}
