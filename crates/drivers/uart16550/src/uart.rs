// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The 16550 register block and the operations on it.
//!
//! Invariants: every operation touches the hardware only through
//! [`Registers`]; no operation loops without a bound.

use core::fmt;

/// Number of registers in the block.
pub const REGISTER_COUNT: usize = 8;

/// How often the transmit path asks whether the holding register is empty
/// before it gives up.
pub const POLL_LIMIT: u32 = 100_000;

/// Divisor for 115200 baud from the 1.8432 MHz reference clock.
pub const DIVISOR_115200: u16 = 1;

/// Line status bit: the transmitter holding register is empty.
pub const LINE_STATUS_TRANSMIT_EMPTY: u8 = 1 << 5;

/// Line status bit: a received byte is waiting.
pub const LINE_STATUS_DATA_READY: u8 = 1 << 0;

/// Interrupt enable bit: a received byte raises an interrupt.
pub const INTERRUPT_RECEIVE: u8 = 1 << 0;

/// Interrupt enable bit: an empty holding register raises an interrupt.
pub const INTERRUPT_TRANSMIT: u8 = 1 << 1;

/// Interrupt identification bit: no interrupt is pending when it is set.
pub const INTERRUPT_NONE: u8 = 1 << 0;

/// Line control bit: the divisor latch is visible instead of the data and
/// interrupt enable registers.
pub const LINE_CONTROL_DIVISOR_LATCH: u8 = 1 << 7;

/// Line control value: eight data bits, one stop bit, no parity.
pub const LINE_CONTROL_8N1: u8 = 0x03;

/// FIFO control value: enable, clear both FIFOs, trigger at 14 bytes.
pub const FIFO_CONTROL_ENABLE: u8 = 0xC7;

/// Modem control value: data terminal ready, request to send, and the
/// auxiliary output that gates the interrupt line.
pub const MODEM_CONTROL_READY: u8 = 0x0B;

/// One register of the block, named by its offset from the base port.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Register {
    /// Receive and transmit buffer; the divisor's low byte while the
    /// divisor latch is open.
    Data = 0,
    /// Interrupt enable; the divisor's high byte while the divisor latch
    /// is open.
    InterruptEnable = 1,
    /// FIFO control when written, interrupt identification when read.
    FifoControl = 2,
    /// Line control.
    LineControl = 3,
    /// Modem control.
    ModemControl = 4,
    /// Line status.
    LineStatus = 5,
    /// Modem status.
    ModemStatus = 6,
    /// Scratch.
    Scratch = 7,
}

impl Register {
    /// Every register, in offset order.
    pub const ALL: [Register; REGISTER_COUNT] = [
        Register::Data,
        Register::InterruptEnable,
        Register::FifoControl,
        Register::LineControl,
        Register::ModemControl,
        Register::LineStatus,
        Register::ModemStatus,
        Register::Scratch,
    ];

    /// The offset of the register from the base port.
    #[must_use]
    pub const fn offset(self) -> u8 {
        match self {
            Register::Data => 0,
            Register::InterruptEnable => 1,
            Register::FifoControl => 2,
            Register::LineControl => 3,
            Register::ModemControl => 4,
            Register::LineStatus => 5,
            Register::ModemStatus => 6,
            Register::Scratch => 7,
        }
    }

    /// The offset as an index into a register file.
    #[must_use]
    #[expect(
        clippy::as_conversions,
        reason = "widening a byte into an index, in a const fn"
    )]
    pub const fn index(self) -> usize {
        self.offset() as usize
    }
}

/// Access to the register block of one controller.
pub trait Registers {
    /// Reads one register.
    fn read(&mut self, register: Register) -> u8;

    /// Writes one register.
    fn write(&mut self, register: Register, value: u8);
}

/// Why an operation on the controller failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UartError {
    /// The transmitter did not become ready within [`POLL_LIMIT`] polls.
    Timeout,
    /// No byte was waiting.
    WouldBlock,
}

impl fmt::Display for UartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UartError::Timeout => f.write_str("the transmitter did not become ready"),
            UartError::WouldBlock => f.write_str("no byte is waiting"),
        }
    }
}

/// One 16550 controller.
#[derive(Clone, Copy, Debug)]
pub struct Uart16550<R: Registers> {
    registers: R,
}

impl<R: Registers> Uart16550<R> {
    /// Takes the register block over without touching it.
    pub const fn new(registers: R) -> Self {
        Uart16550 { registers }
    }

    /// Takes the register block over and programs it for 115200 baud,
    /// eight data bits, one stop bit, no parity, with the FIFOs enabled
    /// and the interrupts off.
    pub fn init(registers: R) -> Self {
        let mut uart = Uart16550 { registers };
        uart.registers.write(Register::InterruptEnable, 0);
        uart.registers
            .write(Register::LineControl, LINE_CONTROL_DIVISOR_LATCH);
        uart.registers
            .write(Register::Data, divisor_low(DIVISOR_115200));
        uart.registers
            .write(Register::InterruptEnable, divisor_high(DIVISOR_115200));
        uart.registers
            .write(Register::LineControl, LINE_CONTROL_8N1);
        uart.registers
            .write(Register::FifoControl, FIFO_CONTROL_ENABLE);
        uart.registers
            .write(Register::ModemControl, MODEM_CONTROL_READY);
        uart
    }

    /// The register block, for a caller that has to reach it directly: a
    /// test that reads what was written, or a driver that owns the
    /// controller and needs a register this crate has no method for.
    pub const fn registers(&mut self) -> &mut R {
        &mut self.registers
    }

    /// Gives the register block back.
    pub fn into_registers(self) -> R {
        self.registers
    }

    /// Sends one byte, waiting at most [`POLL_LIMIT`] polls for the
    /// holding register to become empty.
    ///
    /// # Errors
    ///
    /// [`UartError::Timeout`] if the holding register stays full.
    pub fn write_byte(&mut self, byte: u8) -> Result<(), UartError> {
        let mut polls = 0u32;
        while polls < POLL_LIMIT {
            if self.registers.read(Register::LineStatus) & LINE_STATUS_TRANSMIT_EMPTY != 0 {
                self.registers.write(Register::Data, byte);
                return Ok(());
            }
            polls = polls.saturating_add(1);
        }
        Err(UartError::Timeout)
    }

    /// Sends every byte, stopping at the first timeout.
    ///
    /// # Errors
    ///
    /// The errors of [`Uart16550::write_byte`].
    pub fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), UartError> {
        for byte in bytes {
            self.write_byte(*byte)?;
        }
        Ok(())
    }

    /// Takes the waiting byte.
    ///
    /// # Errors
    ///
    /// [`UartError::WouldBlock`] if no byte is waiting.
    pub fn read_byte(&mut self) -> Result<u8, UartError> {
        if self.registers.read(Register::LineStatus) & LINE_STATUS_DATA_READY == 0 {
            return Err(UartError::WouldBlock);
        }
        Ok(self.registers.read(Register::Data))
    }

    /// Raises an interrupt when a byte arrives.
    pub fn enable_receive_interrupt(&mut self) {
        self.registers
            .write(Register::InterruptEnable, INTERRUPT_RECEIVE);
    }

    /// Raises an interrupt when a byte arrives and when the holding
    /// register becomes empty.
    pub fn enable_interrupts(&mut self, receive: bool, transmit: bool) {
        let mut value = 0;
        if receive {
            value |= INTERRUPT_RECEIVE;
        }
        if transmit {
            value |= INTERRUPT_TRANSMIT;
        }
        self.registers.write(Register::InterruptEnable, value);
    }

    /// Turns every interrupt off.
    pub fn disable_interrupts(&mut self) {
        self.registers.write(Register::InterruptEnable, 0);
    }

    /// `true` if the controller signals an interrupt.
    pub fn interrupt_pending(&mut self) -> bool {
        self.registers.read(Register::FifoControl) & INTERRUPT_NONE == 0
    }
}

/// The low byte of the baud rate divisor.
#[expect(
    clippy::as_conversions,
    reason = "narrowing a value masked to 8 bits, in a const fn"
)]
const fn divisor_low(divisor: u16) -> u8 {
    (divisor & 0xFF) as u8
}

/// The high byte of the baud rate divisor.
#[expect(
    clippy::as_conversions,
    reason = "narrowing the upper 8 bits of a u16, in a const fn"
)]
const fn divisor_high(divisor: u16) -> u8 {
    (divisor >> 8) as u8
}
