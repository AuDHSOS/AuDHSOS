// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
#![doc = include_str!("../README.md")]

use core::panic::PanicInfo;

use audhsos_abi::layout::BOOT_STACK_TOP;
use kernel_core::state::KernelState;
use kernel_core::trap::Exception;
use kernel_core::{boot, trap};
use kernel_hal_x86_64::bootinfo::X86Platform;
use kernel_hal_x86_64::entry;
use kernel_hal_x86_64::exit::QemuExit;
use kernel_hal_x86_64::traps::TrapReport;

/// The entry the loader jumps to, with the address of the boot information
/// page in the first argument.
///
/// # Safety
///
/// The loader calls this once, with interrupts off, on the stack it set
/// up, and never returns to it.
#[unsafe(no_mangle)]
pub extern "C" fn kernel_entry(boot_info: u64) -> ! {
    // SAFETY: the loader passes the address of the page it wrote and
    // mapped readable, sets up the boot stack, and calls this once on the
    // boot processor with interrupts off.
    unsafe { entry::start(boot_info, BOOT_STACK_TOP, on_trap, run) }
}

/// What the kernel does once the machine is ready.
fn run(platform: &X86Platform) {
    let outcome = entry::with_console(|console| boot::run(platform, console));
    match outcome {
        Some(Ok(())) => {
            entry::with_console(|console| boot::finish(console, &mut QemuExit::new()));
        }
        Some(Err(error)) => {
            entry::with_console(|console| boot::abort(error, console, &mut QemuExit::new()));
        }
        None => entry::fail(b"[boot] the console is not reachable\n"),
    }
}

/// Reports a processor exception through the kernel and ends the machine.
fn on_trap(report: TrapReport) {
    let exception = Exception {
        vector: report.vector,
        error_code: report.error_code,
        ip: report.ip,
        sp: report.sp,
        cr2: report.fault_address,
    };
    let mut state = KernelState::new();
    let reported = entry::with_console(|console| {
        trap::on_exception(exception, &mut state, console, &mut QemuExit::new());
    });
    if reported.is_none() {
        entry::fail(b"[trap] a trap arrived while one was being reported\n");
    }
}

/// Reports a panic through the console and ends the machine.
#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    entry::with_console(|console| {
        use kernel_core::println;
        println!(console, "[panic] {}", info.message());
    });
    entry::fail(b"")
}
