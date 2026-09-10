// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::uart`, covering the catalog items 6.6.17.

#![allow(clippy::arithmetic_side_effects)]

use crate::doubles::{Access, RecordingRegisters};
use crate::uart::{
    FIFO_CONTROL_ENABLE, FIFO_DEPTH, INTERRUPT_FIFO_ENABLED, INTERRUPT_NONE, INTERRUPT_RECEIVE,
    INTERRUPT_TRANSMIT, LINE_CONTROL_8N1, LINE_CONTROL_DIVISOR_LATCH, LINE_STATUS_DATA_READY,
    LINE_STATUS_TRANSMIT_EMPTY, MODEM_CONTROL_READY, NO_FIFO_DEPTH, POLL_LIMIT, REGISTER_COUNT,
    Register, Uart16550, UartError,
};
use test_support::generators::{range, vec};
use test_support::property::check;

fn ready() -> RecordingRegisters {
    let mut registers = RecordingRegisters::new();
    registers.set(Register::LineStatus, LINE_STATUS_TRANSMIT_EMPTY);
    registers
}

/// A part that is always ready to send and answers that it took the FIFOs.
fn with_fifo() -> Uart16550<RecordingRegisters> {
    let mut registers = ready();
    registers.set(Register::FifoControl, INTERRUPT_FIFO_ENABLED);
    let uart = Uart16550::init(registers);
    assert_eq!(uart.depth(), FIFO_DEPTH);
    uart
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
    assert_eq!(
        registers.log().last(),
        Some(&Access::Read(Register::FifoControl, FIFO_CONTROL_ENABLE)),
        "the one read is the last: what the part made of the FIFO request"
    );
    assert_eq!(registers.reads_of(Register::FifoControl), 1);
}

#[test]
fn the_depth_is_what_the_part_answers_and_not_what_it_was_asked() {
    let mut asked = RecordingRegisters::new();
    // SLLS597E page 33: both bits are set when FCR0 is, and a 16C450
    // clears them however the request read.
    asked.script(Register::FifoControl, &[INTERRUPT_FIFO_ENABLED]);
    assert_eq!(Uart16550::init(asked).depth(), FIFO_DEPTH);

    let mut refused = RecordingRegisters::new();
    refused.script(Register::FifoControl, &[0]);
    assert_eq!(Uart16550::init(refused).depth(), NO_FIFO_DEPTH);

    let mut half = RecordingRegisters::new();
    half.script(Register::FifoControl, &[0x80]);
    assert_eq!(
        Uart16550::init(half).depth(),
        NO_FIFO_DEPTH,
        "one bit of the pair is not the pair"
    );
}

#[test]
fn taking_the_block_over_without_initializing_touches_nothing() {
    let uart = Uart16550::new(RecordingRegisters::new());
    assert_eq!(
        uart.depth(),
        NO_FIFO_DEPTH,
        "nothing was asked of the part, so nothing is assumed of it"
    );
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
fn writing_a_string_sends_every_byte_and_counts_them() {
    let mut uart = Uart16550::new(ready());
    assert_eq!(uart.write_bytes(b"ok\n"), Ok(3));
    assert_eq!(
        uart.into_registers().writes(),
        vec![
            (Register::Data, b'o'),
            (Register::Data, b'k'),
            (Register::Data, b'\n'),
        ]
    );

    let mut empty = Uart16550::new(RecordingRegisters::new());
    assert_eq!(empty.write_bytes(&[]), Ok(0));
    assert!(empty.into_registers().log().is_empty());
}

#[test]
fn writing_a_string_stops_at_the_first_burst_that_times_out() {
    // One byte a burst, so the second burst finds the transmitter busy.
    let mut registers = RecordingRegisters::new();
    registers.script(Register::LineStatus, &[LINE_STATUS_TRANSMIT_EMPTY]);
    let mut failing = Uart16550::new(registers);
    assert_eq!(failing.write_bytes(b"ab"), Ok(1));
    assert_eq!(
        failing.into_registers().writes(),
        vec![(Register::Data, b'a')],
        "the second byte was never sent"
    );

    let mut never = Uart16550::new(RecordingRegisters::new());
    assert_eq!(
        never.write_bytes(b"ab"),
        Err(UartError::Timeout),
        "a write that moved nothing is an error, not a count of zero"
    );
    assert!(never.into_registers().writes().is_empty());
}

#[test]
fn one_wait_covers_a_whole_burst_and_no_burst_is_longer_than_the_fifo() {
    let mut uart = with_fifo();
    let line = [b'x'; 40];
    assert_eq!(uart.write_bytes(&line), Ok(40));
    let registers = uart.into_registers();
    assert_eq!(
        registers.reads_of(Register::LineStatus),
        3,
        "forty bytes are three bursts of at most sixteen"
    );
    // Every run of data writes between two status reads is a burst, and
    // SLLS597E page 34 licenses at most sixteen of them per THRE. Counting
    // starts at the first status read, which is where the write begins and
    // initialization has had its say.
    let sending = registers
        .log()
        .iter()
        .skip_while(|access| !matches!(access, Access::Read(Register::LineStatus, _)));
    let mut burst = 0usize;
    let mut sent = 0usize;
    for access in sending {
        match access {
            Access::Read(Register::LineStatus, _) => burst = 0,
            Access::Write(Register::Data, _) => {
                burst += 1;
                sent += 1;
                assert!(burst <= usize::from(FIFO_DEPTH), "burst of {burst}");
            }
            _ => {}
        }
    }
    assert_eq!(sent, 40);
}

/// A block that can take a whole run at once, which is what a console
/// driver over system calls is: it records the runs and not the bytes.
#[derive(Default)]
struct BulkRegisters {
    status: u8,
    runs: Vec<(Register, Vec<u8>)>,
}

impl crate::uart::Registers for BulkRegisters {
    fn read(&mut self, register: Register) -> u8 {
        match register {
            Register::FifoControl => INTERRUPT_FIFO_ENABLED,
            _ => self.status,
        }
    }

    fn write(&mut self, register: Register, value: u8) {
        self.runs.push((register, vec![value]));
    }

    fn write_all(&mut self, register: Register, bytes: &[u8]) {
        self.runs.push((register, bytes.to_vec()));
    }
}

#[test]
fn a_burst_reaches_the_block_as_one_run() {
    let registers = BulkRegisters {
        status: LINE_STATUS_TRANSMIT_EMPTY,
        runs: Vec::new(),
    };
    let mut uart = Uart16550::init(registers);
    assert_eq!(uart.depth(), FIFO_DEPTH);
    uart.registers().runs.clear();
    assert_eq!(uart.write_bytes(b"hello"), Ok(5));
    assert_eq!(
        uart.into_registers().runs,
        vec![(Register::Data, b"hello".to_vec())],
        "one run, not five writes: this is the whole point of the burst"
    );
}

#[test]
fn a_part_without_the_fifo_waits_for_every_byte() {
    let mut registers = ready();
    registers.script(Register::FifoControl, &[0]);
    let mut uart = Uart16550::init(registers);
    assert_eq!(uart.depth(), NO_FIFO_DEPTH);
    assert_eq!(uart.write_bytes(b"abcd"), Ok(4));
    assert_eq!(
        uart.into_registers().reads_of(Register::LineStatus),
        4,
        "one THRE buys one byte where there is no FIFO behind it"
    );
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
