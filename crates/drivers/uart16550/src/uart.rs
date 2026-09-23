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

/// Line status bit: a byte arrived at a full FIFO and was lost. The byte
/// at the top of the FIFO is valid (SLLS597E page 37).
pub const LINE_STATUS_OVERRUN: u8 = 1 << 1;

/// Line status bit: the byte at the top of the FIFO has a parity error.
pub const LINE_STATUS_PARITY: u8 = 1 << 2;

/// Line status bit: the byte at the top of the FIFO has no valid stop bit.
pub const LINE_STATUS_FRAMING: u8 = 1 << 3;

/// Line status bit: the byte at the top of the FIFO is the one 0 byte a
/// break loads (SLLS597E page 37).
pub const LINE_STATUS_BREAK: u8 = 1 << 4;

/// Line status bits that mark the byte at the top of the FIFO as no data.
pub const LINE_STATUS_BYTE_ERRORS: u8 =
    LINE_STATUS_PARITY | LINE_STATUS_FRAMING | LINE_STATUS_BREAK;

/// Line status bits 1 to 4, which one read of the register clears
/// (SLLS597E page 37).
pub const LINE_STATUS_ERRORS: u8 = LINE_STATUS_OVERRUN | LINE_STATUS_BYTE_ERRORS;

/// Interrupt enable bit: a received byte raises an interrupt.
pub const INTERRUPT_RECEIVE: u8 = 1 << 0;

/// Interrupt enable bit: an empty holding register raises an interrupt.
pub const INTERRUPT_TRANSMIT: u8 = 1 << 1;

/// Interrupt identification bit: no interrupt is pending when it is set.
pub const INTERRUPT_NONE: u8 = 1 << 0;

/// Interrupt identification bits 3 to 1: the source of a pending
/// interrupt (SLLS597E page 35, table 5).
pub const INTERRUPT_SOURCE: u8 = 0x0E;

/// Line control bit: the divisor latch is visible instead of the data and
/// interrupt enable registers.
pub const LINE_CONTROL_DIVISOR_LATCH: u8 = 1 << 7;

/// Line control value: eight data bits, one stop bit, no parity.
pub const LINE_CONTROL_8N1: u8 = 0x03;

/// FIFO control value: enable, clear both FIFOs, trigger at 14 bytes.
pub const FIFO_CONTROL_ENABLE: u8 = 0xC7;

/// Interrupt identification bits that stand for a working FIFO. SLLS597E
/// page 33 says of them: "These bits are always cleared in TL16C450 mode.
/// They are set when bit 0 of the FIFO control register is set." So a part
/// that answers with both set has the FIFOs the same register just asked
/// for, and a 16C450 answers with neither.
pub const INTERRUPT_FIFO_ENABLED: u8 = 0xC0;

/// How many bytes one THRE licenses on a part with the FIFO. SLLS597E page
/// 41: "The THR is actually a 16-byte FIFO"; page 37, of LSR bit 5: "In the
/// FIFO mode, THRE is set when the transmit FIFO is empty". So one THRE
/// says the whole FIFO is free, and page 34 spends it: "1 to 16 characters
/// may be written to the transmit FIFO".
pub const FIFO_DEPTH: u8 = 16;

/// How many bytes one THRE licenses on a part without the FIFO: the
/// holding register, and nothing behind it.
pub const NO_FIFO_DEPTH: u8 = 1;

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

    /// Writes every byte of `bytes` to one register, in order.
    ///
    /// A block that can hand a whole run over in one operation overrides
    /// this; the default is the run written a byte at a time.
    fn write_all(&mut self, register: Register, bytes: &[u8]) {
        for byte in bytes {
            self.write(register, *byte);
        }
    }
}

/// Why an operation on the controller failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UartError {
    /// The transmitter did not become ready within [`POLL_LIMIT`] polls.
    Timeout,
    /// No byte was waiting.
    WouldBlock,
    /// The line status reported an error; the value holds the bits of
    /// [`LINE_STATUS_ERRORS`] that were set.
    Line(u8),
}

impl fmt::Display for UartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UartError::Timeout => f.write_str("the transmitter did not become ready"),
            UartError::WouldBlock => f.write_str("no byte is waiting"),
            UartError::Line(bits) => write!(f, "line status error {bits:#04x}"),
        }
    }
}

/// What one drain of the receive FIFO took.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Received {
    /// Bytes written to the front of the destination.
    pub count: usize,
    /// Bytes discarded for a parity error, a framing error or a break.
    pub discarded: u8,
    /// Overruns reported; each one lost at least one byte.
    pub overruns: u8,
    /// Every bit of [`LINE_STATUS_ERRORS`] seen during the drain.
    pub errors: u8,
}

impl Received {
    /// Line errors: one per discarded byte and one per overrun.
    #[must_use]
    pub fn line_errors(&self) -> u16 {
        u16::from(self.discarded).saturating_add(u16::from(self.overruns))
    }
}

/// The source of a pending interrupt, decoded from interrupt
/// identification bits 3 to 0 (SLLS597E page 35, table 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InterruptSource {
    /// Bit 0 is set: no interrupt is pending.
    NotPending,
    /// Overrun, parity error, framing error or break; the line status
    /// read resets it.
    LineStatus,
    /// Received data reached the trigger level; the data read resets it.
    ReceivedData,
    /// The FIFO holds data and four character times passed without a byte
    /// received or read; the data read resets it.
    CharacterTimeout,
    /// The transmitter holding register is empty; the identification
    /// read that returned this value reset it.
    TransmitterEmpty,
    /// A modem status input changed; the modem status read resets it.
    ModemStatus,
    /// A code table 5 does not list; the value holds bits 3 to 0.
    Reserved(u8),
}

impl InterruptSource {
    /// Decodes one read of the interrupt identification register.
    #[must_use]
    pub const fn decode(identification: u8) -> Self {
        if identification & INTERRUPT_NONE != 0 {
            return InterruptSource::NotPending;
        }
        match identification & INTERRUPT_SOURCE {
            0x06 => InterruptSource::LineStatus,
            0x04 => InterruptSource::ReceivedData,
            0x0C => InterruptSource::CharacterTimeout,
            0x02 => InterruptSource::TransmitterEmpty,
            0x00 => InterruptSource::ModemStatus,
            bits => InterruptSource::Reserved(bits),
        }
    }
}

/// One 16550 controller.
#[derive(Clone, Copy, Debug)]
pub struct Uart16550<R: Registers> {
    registers: R,
    depth: u8,
}

impl<R: Registers> Uart16550<R> {
    /// Takes the register block over without touching it. Nothing is
    /// asked of the part, so nothing is assumed of it either: one byte a
    /// THRE, which every part licenses.
    pub const fn new(registers: R) -> Self {
        Uart16550 {
            registers,
            depth: NO_FIFO_DEPTH,
        }
    }

    /// Takes the register block over and programs it for 115200 baud,
    /// eight data bits, one stop bit, no parity, with the FIFOs enabled
    /// and the interrupts off.
    pub fn init(registers: R) -> Self {
        let mut uart = Uart16550 {
            registers,
            depth: NO_FIFO_DEPTH,
        };
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
        // The part is asked whether it took the FIFOs, and the answer, not
        // the request, decides how many bytes a THRE is worth.
        if uart.registers.read(Register::FifoControl) & INTERRUPT_FIFO_ENABLED
            == INTERRUPT_FIFO_ENABLED
        {
            uart.depth = FIFO_DEPTH;
        }
        uart
    }

    /// How many bytes one THRE licenses on this part.
    #[must_use]
    pub const fn depth(&self) -> u8 {
        self.depth
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
        self.wait_for_transmitter()?;
        self.registers.write(Register::Data, byte);
        Ok(())
    }

    /// Sends every byte in bursts of [`Uart16550::depth`] and answers with
    /// how many the controller took.
    ///
    /// One wait covers a whole burst: THRE says the transmit FIFO is empty,
    /// which licenses as many bytes as it holds. A burst that finds the
    /// transmitter still busy ends the write, and the count says how far it
    /// came; a byte the controller took is a byte sent, whether the line
    /// has carried it yet or not.
    ///
    /// # Errors
    ///
    /// [`UartError::Timeout`] when the first burst found the transmitter
    /// busy, so that a write which moved nothing is an error and not a
    /// count of zero.
    pub fn write_bytes(&mut self, bytes: &[u8]) -> Result<usize, UartError> {
        let mut written = 0usize;
        for burst in bytes.chunks(self.burst()) {
            if self.wait_for_transmitter().is_err() {
                break;
            }
            self.registers.write_all(Register::Data, burst);
            written = written.saturating_add(burst.len());
        }
        if written == 0 && !bytes.is_empty() {
            return Err(UartError::Timeout);
        }
        Ok(written)
    }

    /// The burst length, never zero, because a chunk of zero has no end.
    fn burst(&self) -> usize {
        usize::from(self.depth).max(usize::from(NO_FIFO_DEPTH))
    }

    /// Waits at most [`POLL_LIMIT`] polls for the transmitter to empty.
    ///
    /// # Errors
    ///
    /// [`UartError::Timeout`] if it stays full.
    fn wait_for_transmitter(&mut self) -> Result<(), UartError> {
        let mut polls = 0u32;
        while polls < POLL_LIMIT {
            if self.registers.read(Register::LineStatus) & LINE_STATUS_TRANSMIT_EMPTY != 0 {
                return Ok(());
            }
            polls = polls.saturating_add(1);
        }
        Err(UartError::Timeout)
    }

    /// Takes the waiting byte, reading the line status once.
    ///
    /// # Errors
    ///
    /// - [`UartError::Line`] if any of line status bits 1 to 4 is set. A
    ///   parity error, a framing error or a break discards the byte at the
    ///   top of the FIFO; an overrun leaves that byte, which is valid, for
    ///   the next call.
    /// - [`UartError::WouldBlock`] if no byte is waiting.
    pub fn read_byte(&mut self) -> Result<u8, UartError> {
        let status = self.registers.read(Register::LineStatus);
        let errors = status & LINE_STATUS_ERRORS;
        if status & LINE_STATUS_DATA_READY != 0 && status & LINE_STATUS_BYTE_ERRORS != 0 {
            let _discarded = self.registers.read(Register::Data);
        }
        if errors != 0 {
            return Err(UartError::Line(errors));
        }
        if status & LINE_STATUS_DATA_READY == 0 {
            return Err(UartError::WouldBlock);
        }
        Ok(self.registers.read(Register::Data))
    }

    /// Drains the receive FIFO into `into`, reading the data register while
    /// line status bit 0 is set, at most [`FIFO_DEPTH`] times per call.
    ///
    /// A byte with a parity error, a framing error or a break is read and
    /// discarded; an overrun is counted and the byte at the top is kept.
    pub fn read_bytes(&mut self, into: &mut [u8]) -> Received {
        let mut received = Received::default();
        for _ in 0..FIFO_DEPTH {
            let Some(slot) = into.get_mut(received.count) else {
                break;
            };
            let status = self.registers.read(Register::LineStatus);
            received.errors |= status & LINE_STATUS_ERRORS;
            if status & LINE_STATUS_OVERRUN != 0 {
                received.overruns = received.overruns.saturating_add(1);
            }
            if status & LINE_STATUS_DATA_READY == 0 {
                break;
            }
            let byte = self.registers.read(Register::Data);
            if status & LINE_STATUS_BYTE_ERRORS != 0 {
                received.discarded = received.discarded.saturating_add(1);
                continue;
            }
            *slot = byte;
            received.count = received.count.saturating_add(1);
        }
        received
    }

    /// Raises an interrupt when a byte arrives, when the holding register
    /// becomes empty, or both.
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

    /// Reads the interrupt identification register once and decodes the
    /// source. The read resets a pending THRE interrupt (SLLS597E page 34),
    /// so [`InterruptSource::TransmitterEmpty`] is the only record of it.
    pub fn interrupt_source(&mut self) -> InterruptSource {
        InterruptSource::decode(self.registers.read(Register::FifoControl))
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
