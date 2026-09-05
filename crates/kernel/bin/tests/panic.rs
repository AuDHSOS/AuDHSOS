// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A panic in a test image reaches the panic handler, which reports it.
//! The image expects the panic, so the report is a pass.

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

kernel_hal_x86_64::test_kernel!(should_panic);

/// The panic handler names the running test and reports the message.
#[expect(
    clippy::panic,
    reason = "the panic is the test: the image is marked `should_panic`"
)]
#[test_case]
fn a_panic_reaches_the_panic_handler() {
    panic!("the panic handler of a test image works");
}
