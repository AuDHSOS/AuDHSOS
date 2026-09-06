// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Bringing the machine into the state the kernel expects, and keeping the
//! console and the exit device reachable from a trap handler.
//!
//! Invariant: by the time [`start`] hands control on, the descriptor
//! tables are loaded, the trap handlers report through the function the
//! caller registered, and the debug console is programmed.

use audhsos_sync::{Global, UncontendedToken};
use kernel_hal_api::console::DebugConsole;
use kernel_hal_api::exit::{ExitStatus, TestExit};

use crate::bootinfo::X86Platform;
use crate::console::{COM1, SerialConsole};
use crate::descriptors;
use crate::exit::QemuExit;
use crate::instructions::halt_forever;
use crate::traps::{TrapHandler, set_handler};

/// The debug console, reachable from a trap handler.
static CONSOLE: Global<SerialConsole> = Global::new();

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
    F: FnOnce(&X86Platform),
{
    // SAFETY: the first serial controller belongs to the kernel while the
    // debug console is enabled; no userland driver exists yet.
    let console = unsafe { SerialConsole::new(COM1) };
    let _ = CONSOLE.init(console);
    set_handler(trap);
    // SAFETY: the caller promises that this runs once, on the boot
    // processor, with interrupts off.
    if unsafe { descriptors::install(stack_top) }.is_err() {
        fail(b"[boot] the descriptor tables are already in place\n");
    }
    // SAFETY: the caller promises that the address names the page the
    // loader wrote and mapped readable for the whole run.
    let Ok(mut platform) = (unsafe { X86Platform::from_address(boot_info) }) else {
        fail(b"[boot] the boot information is not usable\n");
    };
    // SAFETY: the tables the loader built are active, so the window maps
    // every physical frame read and write.
    if !unsafe { crate::memory::register_boot_info(&mut platform) } {
        fail(b"[boot] the boot information page has no translation\n");
    }
    main(&platform);
    halt_forever();
}

/// Runs `body` with the debug console, if it is reachable. It is not while
/// another borrow is alive, which is what a trap inside a report looks
/// like.
pub fn with_console<R>(body: impl FnOnce(&mut SerialConsole) -> R) -> Option<R> {
    let mut console = CONSOLE.borrow(&UncontendedToken).ok()?;
    Some(body(&mut console))
}

/// Reports `message` on the console, if it is reachable, and ends the
/// machine with a failure.
pub fn fail(message: &[u8]) -> ! {
    // The machine ends here, so nothing else will write on the line and
    // the kernel may say the last word on it even after it gave the port
    // to a driver.
    crate::console::reclaim();
    with_console(|console| console.write_bytes(message));
    QemuExit::new().exit(ExitStatus::Failure);
    halt_forever();
}

/// Ends the machine with a success.
pub fn succeed() -> ! {
    QemuExit::new().exit(ExitStatus::Success);
    halt_forever();
}
