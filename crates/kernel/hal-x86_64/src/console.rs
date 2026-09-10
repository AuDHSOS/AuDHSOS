// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The debug console over the first serial port.
//!
//! Invariant: the port range belongs to the kernel while the debug console
//! is enabled; no userland driver may claim it in the same build.

use core::sync::atomic::{AtomicBool, Ordering};
use driver_uart16550::{Register, Registers, Uart16550};

use kernel_hal_api::console::DebugConsole;

/// Base port of the first serial controller.
pub const COM1: u16 = 0x3F8;

/// The register block of one serial controller, reached over port I/O.
#[derive(Clone, Copy, Debug)]
pub struct PortRegisters {
    base: u16,
}

impl PortRegisters {
    /// The block at `base`.
    ///
    /// # Safety
    ///
    /// The eight ports from `base` must belong to a serial controller the
    /// kernel owns.
    #[must_use]
    pub const unsafe fn new(base: u16) -> Self {
        PortRegisters { base }
    }

    fn port(self, register: Register) -> u16 {
        self.base.saturating_add(u16::from(register.offset()))
    }
}

impl Registers for PortRegisters {
    fn read(&mut self, register: Register) -> u8 {
        // SAFETY: the constructor promises that the eight ports from the
        // base belong to a serial controller the kernel owns.
        unsafe { crate::instructions::read_port_u8(self.port(register)) }
    }

    fn write(&mut self, register: Register, value: u8) {
        // SAFETY: the constructor promises that the eight ports from the
        // base belong to a serial controller the kernel owns.
        unsafe { crate::instructions::write_port_u8(self.port(register), value) }
    }
}

/// The debug console of the kernel.
#[derive(Clone, Copy, Debug)]
pub struct SerialConsole {
    uart: Uart16550<PortRegisters>,
}

impl SerialConsole {
    /// Programs the controller at `base` and takes it over.
    ///
    /// # Safety
    ///
    /// The eight ports from `base` must belong to a serial controller the
    /// kernel owns, and no other code may drive it.
    #[must_use]
    pub unsafe fn new(base: u16) -> Self {
        // SAFETY: the caller promises that the ports belong to a
        // controller the kernel owns.
        let registers = unsafe { PortRegisters::new(base) };
        SerialConsole {
            uart: Uart16550::init(registers),
        }
    }
}

/// Whether the kernel still owns the serial controller.
///
/// It owns it from the moment the memory bring-up runs until somebody else
/// reaches one of its ports through an `IoPortRange` capability — which is
/// the console driver of the userland taking it over. From then on the
/// kernel writes nothing there: two writers on one line make one stream of
/// interleaved halves and no reader can take them apart.
///
/// The end of the machine is the exception, and it is not one in practice:
/// after a kernel panic or a fault of the kernel itself nothing else
/// writes, so [`reclaim`] takes the line back and the last thing the
/// machine says is said.
static OURS: AtomicBool = AtomicBool::new(true);

/// Gives the controller up, because somebody else has begun to drive it.
pub fn give_up() {
    OURS.store(false, Ordering::SeqCst);
}

/// Takes the controller back, because the machine is ending and nothing
/// else will write on the line.
///
/// This is for the panic handler and for the paths that end the machine,
/// and for nothing else: a kernel that took the line back while a driver
/// still held it would interleave with it.
pub fn reclaim() {
    OURS.store(true, Ordering::SeqCst);
}

/// `true` while the kernel is still the one writing on the line.
#[must_use]
pub fn is_ours() -> bool {
    OURS.load(Ordering::SeqCst)
}

impl DebugConsole for SerialConsole {
    fn write_bytes(&mut self, bytes: &[u8]) {
        if !is_ours() {
            return;
        }
        // The runs between the newlines go out in bursts; a newline takes
        // the return with it, which is what a terminal needs and what the
        // bytes handed in do not carry.
        for part in bytes.split_inclusive(|byte| *byte == b'\n') {
            match part.split_last() {
                Some((b'\n', line)) => {
                    let _ = self.uart.write_bytes(line);
                    let _ = self.uart.write_bytes(b"\r\n");
                }
                _ => {
                    let _ = self.uart.write_bytes(part);
                }
            }
        }
    }
}
