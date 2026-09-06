// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::uart`, covering the catalog items 6.6.17.

#![allow(clippy::arithmetic_side_effects)]

use crate::doubles::{Access, RecordingRegisters};
use crate::uart::{
    FIFO_CONTROL_ENABLE, INTERRUPT_NONE, INTERRUPT_RECEIVE, INTERRUPT_TRANSMIT, LINE_CONTROL_8N1,
    LINE_CONTROL_DIVISOR_LATCH, LINE_STATUS_DATA_READY, LINE_STATUS_TRANSMIT_EMPTY,
    MODEM_CONTROL_READY, POLL_LIMIT, REGISTER_COUNT, Register, Uart16550, UartError,
};
use test_support::generators::{range, vec};
use test_support::property::check;

fn ready() -> RecordingRegisters {
    let mut registers = RecordingRegisters::new();
    registers.set(Register::LineStatus, LINE_STATUS_TRANSMIT_EMPTY);
    registers
}

#[test]
fn every_register_reports_its_offset_and_index() {
    for (index, register) in Register::ALL.into_iter().enumerate() {
        assert_eq!(usize::from(register.offset()), index);
        assert_eq!(register.index(), index);
    }
    assert_eq!(Register::ALL.len(), REGISTER_COUNT);
}

#[test]
fn initialization_writes_the_registers_in_the_documented_order() {
    let uart = Uart16550::init(RecordingRegisters::new());
    let registers = uart.into_registers();
    assert_eq!(
        registers.writes(),
        vec![
            (Register::InterruptEnable, 0),
            (Register::LineControl, LINE_CONTROL_DIVISOR_LATCH),
            (Register::Data, 0x01),
            (Register::InterruptEnable, 0x00),
            (Register::LineControl, LINE_CONTROL_8N1),
            (Register::FifoControl, FIFO_CONTROL_ENABLE),
            (Register::ModemControl, MODEM_CONTROL_READY),
        ]
    );
    assert!(
        registers
            .log()
            .iter()
            .all(|access| matches!(access, Access::Write(..))),
        "initialization reads nothing"
    );
}

#[test]
fn taking_the_block_over_without_initializing_touches_nothing() {
    let uart = Uart16550::new(RecordingRegisters::new());
    assert!(uart.into_registers().log().is_empty());
}

#[test]
fn a_write_waits_for_the_transmitter_and_then_sends_the_byte() {
    let mut uart = Uart16550::new(ready());
    assert_eq!(uart.write_byte(b'A'), Ok(()));
    let registers = uart.into_registers();
    assert_eq!(
        registers.log(),
        &[
            Access::Read(Register::LineStatus, LINE_STATUS_TRANSMIT_EMPTY),
            Access::Write(Register::Data, b'A'),
        ]
    );
}

#[test]
fn a_write_polls_until_the_transmitter_becomes_ready() {
    let mut registers = RecordingRegisters::new();
    registers.script(Register::LineStatus, &[0, 0, LINE_STATUS_TRANSMIT_EMPTY]);
    let mut uart = Uart16550::new(registers);
    assert_eq!(uart.write_byte(b'X'), Ok(()));
    let registers = uart.into_registers();
    assert_eq!(registers.reads_of(Register::LineStatus), 3);
    assert_eq!(registers.writes(), vec![(Register::Data, b'X')]);
}

#[test]
fn a_transmitter_that_never_becomes_ready_makes_the_write_time_out() {
    let mut uart = Uart16550::new(RecordingRegisters::new());
    assert_eq!(uart.write_byte(b'A'), Err(UartError::Timeout));
    let registers = uart.into_registers();
    assert_eq!(
        registers.reads_of(Register::LineStatus),
        usize::try_from(POLL_LIMIT).unwrap()
    );
    assert!(registers.writes().is_empty(), "nothing was sent");
}

#[test]
fn writing_a_string_stops_at_the_first_timeout() {
    let mut uart = Uart16550::new(ready());
    assert_eq!(uart.write_bytes(b"ok\n"), Ok(()));
    assert_eq!(
        uart.into_registers().writes(),
        vec![
            (Register::Data, b'o'),
            (Register::Data, b'k'),
            (Register::Data, b'\n'),
        ]
    );

    let mut registers = RecordingRegisters::new();
    registers.script(Register::LineStatus, &[LINE_STATUS_TRANSMIT_EMPTY]);
    let mut failing = Uart16550::new(registers);
    assert_eq!(failing.write_bytes(b"ab"), Err(UartError::Timeout));
    assert_eq!(
        failing.into_registers().writes(),
        vec![(Register::Data, b'a')],
        "the second byte was never sent"
    );

    let mut empty = Uart16550::new(RecordingRegisters::new());
    assert_eq!(empty.write_bytes(&[]), Ok(()));
    assert!(empty.into_registers().log().is_empty());
}

#[test]
fn a_read_without_data_would_block_and_reads_nothing_else() {
    let mut uart = Uart16550::new(RecordingRegisters::new());
    assert_eq!(uart.read_byte(), Err(UartError::WouldBlock));
    let registers = uart.into_registers();
    assert_eq!(registers.reads_of(Register::LineStatus), 1);
    assert_eq!(registers.reads_of(Register::Data), 0);
}

#[test]
fn a_read_with_data_returns_the_byte() {
    let mut registers = RecordingRegisters::new();
    registers.set(Register::LineStatus, LINE_STATUS_DATA_READY);
    registers.set(Register::Data, b'q');
    let mut uart = Uart16550::new(registers);
    assert_eq!(uart.read_byte(), Ok(b'q'));
    let registers = uart.into_registers();
    assert_eq!(registers.reads_of(Register::Data), 1);
    assert!(registers.writes().is_empty(), "a read writes nothing");
}

#[test]
fn the_interrupt_enable_register_carries_the_requested_sources() {
    let mut uart = Uart16550::new(RecordingRegisters::new());
    uart.enable_receive_interrupt();
    uart.enable_interrupts(true, true);
    uart.enable_interrupts(false, true);
    uart.enable_interrupts(false, false);
    uart.disable_interrupts();
    assert_eq!(
        uart.into_registers().writes(),
        vec![
            (Register::InterruptEnable, INTERRUPT_RECEIVE),
            (
                Register::InterruptEnable,
                INTERRUPT_RECEIVE | INTERRUPT_TRANSMIT
            ),
            (Register::InterruptEnable, INTERRUPT_TRANSMIT),
            (Register::InterruptEnable, 0),
            (Register::InterruptEnable, 0),
        ]
    );
}

#[test]
fn a_pending_interrupt_shows_as_a_clear_bit_zero() {
    let mut registers = RecordingRegisters::new();
    registers.script(Register::FifoControl, &[INTERRUPT_NONE, 0x04, 0x02]);
    let mut uart = Uart16550::new(registers);
    assert!(!uart.interrupt_pending(), "bit zero set means no interrupt");
    assert!(uart.interrupt_pending(), "a received byte is pending");
    assert!(
        uart.interrupt_pending(),
        "an empty holding register is pending"
    );
}

#[test]
fn the_error_renders_a_message() {
    for error in [UartError::Timeout, UartError::WouldBlock] {
        assert!(!format!("{error}").is_empty());
    }
}

#[test]
fn property_a_write_sends_exactly_one_byte_when_it_succeeds() {
    check(
        "uart_write_once",
        &vec(range(0u8..=u8::MAX), 0..=8),
        |statuses| {
            let mut registers = RecordingRegisters::new();
            registers.script(Register::LineStatus, statuses);
            registers.set(Register::LineStatus, LINE_STATUS_TRANSMIT_EMPTY);
            let mut uart = Uart16550::new(registers);
            let result = uart.write_byte(b'z');
            let registers = uart.into_registers();
            let writes = registers.writes();
            match result {
                Ok(()) if writes == vec![(Register::Data, b'z')] => Ok(()),
                Ok(()) => Err(format!("sent {writes:?}")),
                Err(error) => Err(format!("unexpected {error}")),
            }
        },
    );
}

#[test]
fn the_register_block_is_reachable_through_the_controller() {
    use crate::uart::Registers as _;
    let mut uart = Uart16550::new(RecordingRegisters::new());
    uart.registers().write(Register::Data, 0x5A);
    assert_eq!(uart.registers().writes(), vec![(Register::Data, 0x5A)]);
}
