// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Bringing the machine into the state the kernel expects.
//!
//! Invariant: by the time `start` hands control on, the descriptor tables
//! are loaded, the trap handlers report through the function the caller
//! registered, and the debug console is programmed.

use kernel_hal_api::console::DebugConsole;
use kernel_hal_api::exit::{ExitStatus, TestExit};

use crate::bootinfo::X86Platform;
use crate::console::SerialConsole;
use crate::descriptors;
use crate::exit::QemuExit;
use crate::instructions::halt_forever;
use crate::traps::{TrapHandler, set_handler};

/// What the machine hands to the kernel once it is ready.
pub struct Machine {
    /// What the loader reported.
    pub platform: X86Platform,
    /// The debug console.
    pub console: SerialConsole,
    /// The exit device.
    pub exit: QemuExit,
}

/// Programs the console, loads the descriptor tables, registers `trap`,
/// reads the boot information, and hands the machine to `main`. Nothing
/// returns from here: `main` ends the machine, or the processor halts.
///
/// # Safety
///
/// `boot_info` must be the address the loader passed in the entry call,
/// `stack_top` must be the top of the stack the loader set up, and this
/// must run once, on the boot processor, with interrupts off.
pub unsafe fn start<F>(boot_info: u64, stack_top: u64, trap: TrapHandler, main: F) -> !
where
    F: FnOnce(&mut Machine),
{
    // SAFETY: the first serial controller belongs to the kernel while the
    // debug console is enabled; no userland driver exists yet.
    let mut console = unsafe { SerialConsole::new(crate::console::COM1) };
    let mut exit = QemuExit::new();
    set_handler(trap);
    // SAFETY: the caller promises that this runs once, on the boot
    // processor, with interrupts off.
    if unsafe { descriptors::install(stack_top) }.is_err() {
        console.write_bytes(b"[boot] the descriptor tables are already in place\n");
        exit.exit(ExitStatus::Failure);
        halt_forever();
    }
    // SAFETY: the caller promises that the address names the page the
    // loader wrote and mapped readable for the whole run.
    let Ok(platform) = (unsafe { X86Platform::from_address(boot_info) }) else {
        console.write_bytes(b"[boot] the boot information is not usable\n");
        exit.exit(ExitStatus::Failure);
        halt_forever();
    };
    let mut machine = Machine {
        platform,
        console,
        exit,
    };
    main(&mut machine);
    halt_forever();
}
