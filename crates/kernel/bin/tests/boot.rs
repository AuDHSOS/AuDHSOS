// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The machine boots: the loader hands over, the descriptor tables load,
//! the boot information parses, and the harness reports.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_hal_x86_64::testing::run_tests)]
#![reexport_test_harness_main = "test_main"]

// The kernel image uses these crates; a test image does not.
use audhsos_abi as _;
use kernel_core as _;

kernel_hal_x86_64::test_kernel!();

/// The image reaches the harness and reports one line.
#[test_case]
fn the_kernel_reaches_the_test_harness() {}

/// The loader left at least one memory region behind.
#[test_case]
fn the_loader_reports_usable_memory() {
    let regions = kernel_hal_x86_64::testing::platform_regions();
    assert!(regions > 0, "the loader reported no memory region");
}
