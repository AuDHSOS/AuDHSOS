// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A read from an unmapped address reaches the page fault handler, which
//! reports the address the processor named.

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

/// The vector a missing translation raises.
const PAGE_FAULT: u8 = 14;

/// The address nothing is mapped at.
const UNMAPPED: u64 = 0xdead_beef;

fn on_page_fault(report: TrapReport) -> ! {
    if report.vector != PAGE_FAULT {
        testing::fail(format_args!(
            "trap {} instead of a page fault",
            report.vector
        ));
    }
    if report.fault_address != UNMAPPED {
        testing::fail(format_args!(
            "the page fault names {:#x}, not {UNMAPPED:#x}",
            report.fault_address
        ));
    }
    testing::pass();
    testing::finish()
}

/// The handler sees vector fourteen and the address that faulted.
#[test_case]
fn a_read_from_an_unmapped_address_reports_that_address() {
    testing::set_trap_hook(|report| on_page_fault(report));
    let _ = testing::read_byte(UNMAPPED);
}
