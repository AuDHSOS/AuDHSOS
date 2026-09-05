// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The debug console over the first serial port.
//!
//! Invariant: the port range belongs to the kernel while the debug console
//! is enabled; no userland driver may claim it in the same build.

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

impl DebugConsole for SerialConsole {
    fn write_bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            if *byte == b'\n' {
                let _ = self.uart.write_byte(b'\r');
            }
            let _ = self.uart.write_byte(*byte);
        }
    }
}
