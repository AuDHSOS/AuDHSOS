// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Bringing the machine into the state the kernel expects, and keeping the
//! console and the exit device reachable from a trap handler.
//!
//! Invariant: by the time [`start`] hands control on, the descriptor
//! tables are loaded, the trap handlers report through the function the
//! caller registered, and the debug console is programmed.

#[cfg(feature = "debug-uart")]
use crate::processor::KernelToken;
#[cfg(feature = "debug-uart")]
use audhsos_sync::Global;
#[cfg(feature = "debug-uart")]
use kernel_hal_api::console::DebugConsole;
#[cfg(feature = "test-exit")]
use kernel_hal_api::exit::{ExitStatus, TestExit};

use crate::bootinfo::X86Platform;
#[cfg(feature = "debug-uart")]
use crate::console::{COM1, SerialConsole};
use crate::descriptors;
#[cfg(feature = "test-exit")]
use crate::exit::QemuExit;
use crate::instructions::halt_forever;
use crate::traps::{TrapHandler, set_handler};

/// The debug console, reachable from a trap handler.
#[cfg(feature = "debug-uart")]
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
    #[cfg(feature = "debug-uart")]
    {
        // SAFETY: the first serial controller belongs to the kernel while
        // the debug console is enabled; no userland driver exists yet.
        let console = unsafe { SerialConsole::new(COM1) };
        let _ = CONSOLE.init(console);
    }
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
    // SAFETY: the tables the loader built are active, so the window maps
    // every frame of the memory the firmware reported, which is where the
    // ACPI tables lie.
    if let Ok(mcfg) = unsafe { crate::acpi::find_mcfg(&platform) }
        && let Some(window) = mcfg.first()
    {
        platform.set_ecam(window);
    }
    main(&platform);
    halt_forever();
}

/// Runs `body` with the debug console, if it is reachable. It is not while
/// another borrow is alive, which is what a trap inside a report looks
/// like.
#[cfg(feature = "debug-uart")]
pub fn with_console<R>(body: impl FnOnce(&mut SerialConsole) -> R) -> Option<R> {
    let _guard = crate::instructions::InterruptGuard::new();
    let mut console = CONSOLE.borrow(&KernelToken).ok()?;
    Some(body(&mut console))
}

/// `true` while this processor holds the debug console; O(1), no wait.
#[cfg(feature = "debug-uart")]
#[must_use]
pub fn console_is_held() -> bool {
    CONSOLE.is_held_by(&KernelToken)
}

/// Reports `message` on the console with `debug-uart`, ends the machine
/// with a failure with `test-exit`, and halts.
pub fn fail(message: &[u8]) -> ! {
    // The machine ends here, so nothing else will write on the line and
    // the kernel may say the last word on it even after it gave the port
    // to a driver.
    #[cfg(feature = "debug-uart")]
    {
        crate::console::reclaim();
        with_console(|console| console.write_bytes(message));
    }
    #[cfg(not(feature = "debug-uart"))]
    let _ = message;
    #[cfg(feature = "test-exit")]
    QemuExit::new().exit(ExitStatus::Failure);
    halt_forever();
}

/// Ends the machine with a success with `test-exit`, and halts.
pub fn succeed() -> ! {
    #[cfg(feature = "test-exit")]
    QemuExit::new().exit(ExitStatus::Success);
    halt_forever();
}
