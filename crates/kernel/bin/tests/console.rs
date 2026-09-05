// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The debug console carries what the kernel writes to the runner.

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

kernel_hal_x86_64::test_kernel!();

/// The string the runner looks for in the serial output.
const MARKER: &[u8] = b"audhsos console marker 0123456789\n";

/// The marker reaches the runner over the serial port.
#[test_case]
fn the_debug_console_writes_the_marker() {
    testing::print(MARKER);
}
