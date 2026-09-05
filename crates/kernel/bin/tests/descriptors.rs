// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The descriptor tables are in place once and refuse a second load.

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

use audhsos_abi::layout::BOOT_STACK_TOP;
use kernel_hal_x86_64::descriptors::{self, InstallError};

kernel_hal_x86_64::test_kernel!();

/// The tables are in place, so a second load is refused.
#[test_case]
fn a_second_install_of_the_tables_is_refused() {
    // SAFETY: the tables are already in place, so the call reports that
    // and touches no register.
    let outcome = unsafe { descriptors::install(BOOT_STACK_TOP) };
    assert_eq!(outcome, Err(InstallError::AlreadyInstalled));
}
