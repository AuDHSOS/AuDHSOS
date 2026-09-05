// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
#![doc = include_str!("../README.md")]

use core::panic::PanicInfo;

use audhsos_abi::layout::BOOT_STACK_TOP;
use kernel_core::memory::MemoryError;
use kernel_core::state::KernelState;
use kernel_core::trap::Exception;
use kernel_core::{boot, memory, trap};
use kernel_hal_x86_64::bootinfo::X86Platform;
use kernel_hal_x86_64::entry;
use kernel_hal_x86_64::exit::QemuExit;
use kernel_hal_x86_64::paging::{LocalTlb, X86Entry, active_root};
use kernel_hal_x86_64::traps::TrapReport;
use kernel_hal_x86_64::window::PhysicalWindow;

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
    let outcome = entry::with_console(|console| {
        boot::run(platform, console)?;
        take_memory(platform)?;
        memory::with_memory(|memory| memory::report(memory, console));
        Ok(())
    });
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

/// Takes the memory of the machine over: the reserve, the regions of the
/// kernel address space, and the end of the loader's identity mapping.
fn take_memory(platform: &X86Platform) -> Result<(), boot::BootError> {
    let root = active_root().map_err(|_| MemoryError::Address)?;
    // SAFETY: the tables the loader built are active, so the window maps
    // every physical frame read and write; the kernel is the only writer,
    // because nothing else runs yet and interrupts are off.
    let mut window = unsafe { PhysicalWindow::kernel() };
    let mut tlb = LocalTlb::new();
    let override_bytes = requested_reserve(platform);
    memory::initialize::<X86Entry, _, _, _>(platform, root, &mut window, &mut tlb, override_bytes)?;
    Ok(())
}

/// The reserve size the header of the boot image asks for, or zero for the
/// default. A boot image the kernel cannot read asks for nothing.
fn requested_reserve(platform: &X86Platform) -> u64 {
    let Some((start, _)) = memory::boot_image(platform) else {
        return 0;
    };
    // SAFETY: the tables the loader built are active, so the window maps
    // every physical frame read and write.
    let Some(bytes) = (unsafe { kernel_hal_x86_64::memory::read_frame(start) }) else {
        return 0;
    };
    memory::requested_reserve(platform, &bytes)
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
