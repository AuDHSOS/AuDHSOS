// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A selector beyond the descriptor table reaches the general protection
//! handler.

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

/// The vector a bad selector raises.
const GENERAL_PROTECTION: u8 = 13;

fn on_general_protection(report: TrapReport) -> ! {
    if report.vector != GENERAL_PROTECTION {
        testing::fail(format_args!(
            "trap {} instead of a general protection fault",
            report.vector
        ));
    }
    testing::pass();
    testing::finish()
}

/// The handler sees vector thirteen and ends the machine.
#[test_case]
fn a_selector_outside_the_table_raises_the_general_protection_exception() {
    testing::set_trap_hook(|report| on_general_protection(report));
    testing::raise_general_protection();
}
