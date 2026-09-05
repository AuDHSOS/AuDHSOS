// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What a kernel test image needs: the harness over the debug console and
//! the exit device, the wiring of the entry point and the panic handler,
//! and the instructions that raise the exceptions the trap tests expect.
//!
//! Invariants: the harness is borrowed only for as long as one protocol
//! line takes, so that a test that panics or traps still finds it free;
//! every test image ends the machine, either through the harness or
//! through the exit device.

use core::arch::asm;
use core::fmt;
use core::panic::PanicInfo;

use audhsos_abi::layout::BOOT_STACK_TOP;
use audhsos_sync::{Global, UncontendedToken};
use kernel_hal_api::console::DebugConsole;
use kernel_hal_api::exit::{ExitStatus, TestExit};
use kernel_hal_api::platform::Platform;
use kernel_test_harness::{Harness, Testable};

use crate::bootinfo::X86Platform;
use crate::console::{COM1, SerialConsole};
use crate::descriptors;
use crate::exit::QemuExit;
use crate::instructions::halt_forever;
use crate::traps::{TrapReport, set_handler};

/// A selector beyond the last descriptor of the global descriptor table.
const INVALID_SELECTOR: u16 = 0x7FF8;

/// The harness of the running test image.
static HARNESS: Global<Harness<SerialConsole, QemuExit>> = Global::new();

/// What a test image does with a trap it expects. A hook that returns
/// lets the processor resume, which only a trap and not a fault survives.
pub type TrapHook = fn(TrapReport);

/// The hook of the running test image.
static HOOK: Global<TrapHook> = Global::new();

/// The test that is running. Its line is written when the test is over,
/// so that a test may write on the console without breaking the protocol.
struct Current {
    name: &'static str,
}

/// The test that is running.
static CURRENT: Global<Current> = Global::new();

/// The machine as the loader described it.
static PLATFORM: Global<X86Platform> = Global::new();

/// The name a report carries before the first test starts.
const OUTSIDE: &str = "the test image";

/// Programs the console, loads the descriptor tables, reads the boot
/// information, runs `test_main`, and ends the machine.
///
/// # Safety
///
/// `boot_info` must be the address the loader passed in the entry call,
/// and this must run once, on the boot processor, with interrupts off.
pub unsafe fn start(boot_info: u64, should_panic: bool, test_main: fn()) -> ! {
    // SAFETY: the first serial controller belongs to the kernel while the
    // debug console is enabled; no userland driver exists yet.
    let console = unsafe { SerialConsole::new(COM1) };
    let harness = if should_panic {
        Harness::expecting_panic(console, QemuExit::new())
    } else {
        Harness::new(console, QemuExit::new())
    };
    if HARNESS.init(harness).is_err() || CURRENT.init(Current { name: OUTSIDE }).is_err() {
        halt_forever();
    }
    set_handler(on_trap);
    // SAFETY: the caller promises that this runs once, on the boot
    // processor, with interrupts off.
    if unsafe { descriptors::install(BOOT_STACK_TOP) }.is_err() {
        fail(format_args!("the descriptor tables are already in place"));
    }
    // SAFETY: the caller promises that the address names the page the
    // loader wrote and mapped readable for the whole run.
    let Ok(platform) = (unsafe { X86Platform::from_address(boot_info) }) else {
        fail(format_args!("the boot information is not usable"));
    };
    if PLATFORM.init(platform).is_err() {
        fail(format_args!("the platform is already in place"));
    }
    test_main();
    finish()
}

/// Runs every test the image declared, one protocol line each.
pub fn run_tests(tests: &[&dyn Testable]) {
    for test in tests {
        set_current(test.name());
        test.run();
        pass();
    }
}

/// Names the test that is running, without writing anything.
fn set_current(name: &'static str) {
    if let Ok(mut current) = CURRENT.borrow(&UncontendedToken) {
        current.name = name;
    }
}

/// The name of the test that is running.
fn current() -> &'static str {
    CURRENT
        .borrow(&UncontendedToken)
        .map_or(OUTSIDE, |current| current.name)
}

/// Reports the running test as passed. A test image whose test is what the
/// machine did rather than what a function did calls this from its trap
/// hook.
pub fn pass() {
    let name = current();
    let _ = with_harness(|harness| {
        harness.begin(name);
        harness.end();
    });
}

/// Writes bytes on the debug console. The line of the running test is not
/// written until the test is over, so this breaks nothing.
pub fn print(bytes: &[u8]) {
    let _ = with_harness(|harness| harness.console().write_bytes(bytes));
}

/// Reports that the running test failed and ends the machine. An image
/// that expects a panic reports a pass instead.
pub fn fail(arguments: fmt::Arguments<'_>) -> ! {
    let name = current();
    let _ = with_harness(|harness| {
        harness.begin(name);
        harness.fail(arguments);
    });
    halt_forever()
}

/// Writes the summary and ends the machine.
pub fn finish() -> ! {
    let _ = with_harness(|harness| harness.run(&[]));
    halt_forever()
}

/// The number of memory regions the loader reported.
#[must_use]
pub fn platform_regions() -> usize {
    PLATFORM
        .borrow(&UncontendedToken)
        .map_or(0, |platform| platform.memory_regions().len())
}

/// Registers what the image does with the next trap. Without a hook a
/// trap is a failure.
pub fn set_trap_hook(hook: TrapHook) {
    let _ = HOOK.init(hook);
}

/// Runs `body` with the harness, if it is reachable. It is not while
/// another borrow is alive, which is what a trap inside a report looks
/// like.
fn with_harness<R>(body: impl FnOnce(&mut Harness<SerialConsole, QemuExit>) -> R) -> Option<R> {
    let mut harness = HARNESS.borrow(&UncontendedToken).ok()?;
    Some(body(&mut harness))
}

/// Reports a panic through the harness and ends the machine.
pub fn on_panic(info: &PanicInfo<'_>) -> ! {
    let name = current();
    let reported = with_harness(|harness| {
        harness.begin(name);
        harness.on_panic(info);
    });
    if reported.is_none() {
        QemuExit::new().exit(ExitStatus::Failure);
    }
    halt_forever()
}

/// Hands a trap to the image's hook, or reports it as a failure.
fn on_trap(report: TrapReport) {
    let hook = HOOK.borrow(&UncontendedToken).ok().map(|hook| *hook);
    match hook {
        Some(hook) => hook(report),
        None => fail(format_args!(
            "trap {} error {:#x} at {:#x} address {:#x}",
            report.vector, report.error_code, report.ip, report.fault_address
        )),
    }
}

/// Executes `int3`, which raises the breakpoint exception and resumes at
/// the next instruction.
pub fn raise_breakpoint() {
    // SAFETY: vector 3 has a handler by the time a test image runs, and a
    // breakpoint changes no register and no memory.
    unsafe {
        asm!("int3", options(nomem, nostack));
    }
}

/// Executes an undefined instruction, which raises the invalid opcode
/// exception. The exception is a fault, so the handler must end the
/// machine instead of resuming.
pub fn raise_invalid_opcode() {
    // SAFETY: vector 6 has a handler by the time a test image runs, and
    // the instruction changes no register and no memory.
    unsafe {
        asm!("ud2", options(nomem, nostack));
    }
}

/// Divides by zero, which raises the divide error exception. The
/// exception is a fault, so the handler must end the machine instead of
/// resuming.
pub fn raise_divide_error() {
    // SAFETY: vector 0 has a handler by the time a test image runs, and
    // the three registers the division uses are declared as clobbered.
    unsafe {
        asm!(
            "xor edx, edx",
            "mov eax, 1",
            "xor ecx, ecx",
            "div ecx",
            out("eax") _,
            out("edx") _,
            out("ecx") _,
            options(nomem, nostack),
        );
    }
}

/// Loads a selector that lies beyond the descriptor table, which raises
/// the general protection exception. The exception is a fault, so the
/// handler must end the machine instead of resuming.
pub fn raise_general_protection() {
    // SAFETY: vector 13 has a handler by the time a test image runs; the
    // load faults before it changes the segment register, and the general
    // purpose register it uses is declared as clobbered.
    unsafe {
        asm!(
            "mov ax, {selector:x}",
            "mov ds, ax",
            selector = in(reg) INVALID_SELECTOR,
            out("eax") _,
            options(nomem, nostack),
        );
    }
}

/// Reads one byte from `address`, which raises the page fault exception
/// when nothing is mapped there. The exception is a fault, so the handler
/// must end the machine instead of resuming.
#[must_use]
pub fn read_byte(address: u64) -> u8 {
    let pointer = core::ptr::without_provenance::<u8>(usize::try_from(address).unwrap_or(0));
    // SAFETY: the caller uses this only to provoke a page fault the trap
    // hook of the image ends the machine in; nothing reads the value.
    unsafe { pointer.read_volatile() }
}

/// Declares the entry point and the panic handler of a test image. The
/// image writes its tests as `#[test_case]` functions; the wiring is
/// written once, here.
#[macro_export]
macro_rules! test_kernel {
    () => {
        $crate::test_kernel!(@image false);
    };
    (should_panic) => {
        $crate::test_kernel!(@image true);
    };
    (@image $should_panic:expr) => {
        /// The entry the loader jumps to, with the address of the boot
        /// information page in the first argument.
        #[unsafe(no_mangle)]
        pub extern "C" fn kernel_entry(boot_info: u64) -> ! {
            // SAFETY: the loader calls this once, with interrupts off, on
            // the stack it set up, and never returns to it.
            unsafe { $crate::testing::start(boot_info, $should_panic, test_main) }
        }

        /// Reports a panic through the test harness.
        #[panic_handler]
        fn panic(info: &::core::panic::PanicInfo<'_>) -> ! {
            $crate::testing::on_panic(info)
        }
    };
}
