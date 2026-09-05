// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! An undefined instruction reaches the invalid opcode handler.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_hal_x86_64::testing::run_tests)]
#![reexport_test_harness_main = "test_main"]

// The kernel image uses these crates; a test image does not.
use audhsos_abi as _;
use kernel_core as _;
// The memory test image uses these crates; the other test images do not.
use kernel_hal_api as _;
use kernel_mm as _;
use kernel_types as _;

use kernel_hal_x86_64::testing;
use kernel_hal_x86_64::traps::TrapReport;

kernel_hal_x86_64::test_kernel!();

/// The vector an undefined instruction raises.
const INVALID_OPCODE: u8 = 6;

fn on_invalid_opcode(report: TrapReport) -> ! {
    if report.vector != INVALID_OPCODE {
        testing::fail(format_args!(
            "trap {} instead of an invalid opcode",
            report.vector
        ));
    }
    testing::pass();
    testing::finish()
}

/// The handler sees vector six and ends the machine.
#[test_case]
fn an_undefined_instruction_raises_the_invalid_opcode_exception() {
    testing::set_trap_hook(|report| on_invalid_opcode(report));
    testing::raise_invalid_opcode();
}
