// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::console`.

use driver_uart16550::doubles::RecordingRegisters;
use driver_uart16550::uart::{LINE_STATUS_BREAK, LINE_STATUS_DATA_READY, LINE_STATUS_OVERRUN};
use driver_uart16550::{FIFO_DEPTH, Received, Register, UartError};

use crate::console::{Console, DRAIN, DRAIN_WORDS, RECEIVE_CAPACITY, encode_drain};

/// A console over a controller that is always ready to send, with what the
/// initialization wrote forgotten again: it writes the divisor through the
/// data register, and a test that watches what goes out on the line has no
/// use for it.
fn console() -> Console<RecordingRegisters> {
    let mut console = fresh();
    console.uart().registers().clear();
    console
}

/// The same, with everything the initialization did still recorded.
fn fresh() -> Console<RecordingRegisters> {
    let mut registers = RecordingRegisters::new();
    // The transmitter-empty bit, so that a write never waits.
    registers.set(Register::LineStatus, 0x20);
    Console::new(registers)
}

/// Every byte the console wrote to the transmit register.
fn sent(console: &mut Console<RecordingRegisters>) -> Vec<u8> {
    console
        .uart()
        .registers()
        .writes()
        .iter()
        .filter(|(register, _)| *register == Register::Data)
        .map(|(_, value)| *value)
        .collect()
}

#[test]
fn a_new_console_has_nothing_waiting() {
    let console = console();
    assert_eq!(console.waiting(), 0);
    assert_eq!(console.dropped(), 0);
}

#[test]
fn what_is_written_reaches_the_controller() {
    let mut console = console();
    assert_eq!(console.write(b"hello").unwrap(), 5);
    assert_eq!(sent(&mut console), b"hello".to_vec());
}

#[test]
fn a_write_of_no_bytes_writes_nothing_and_is_no_failure() {
    let mut console = console();
    assert_eq!(console.write(b"").unwrap(), 0);
    assert!(sent(&mut console).is_empty());
}

#[test]
fn a_transmitter_that_never_becomes_ready_is_a_failure_and_not_a_count() {
    let mut registers = RecordingRegisters::new();
    // The transmitter-empty bit stays clear, so every poll finds it busy.
    registers.set(Register::LineStatus, 0);
    let mut console = Console::new(registers);
    assert_eq!(console.write(b"hello").unwrap_err(), UartError::Timeout);
}

#[test]
fn a_byte_that_arrives_waits_until_it_is_read() {
    let mut console = console();
    assert!(console.receive(b'a'));
    assert!(console.receive(b'b'));
    assert_eq!(console.waiting(), 2);
    let mut into = [0u8; 8];
    assert_eq!(console.read(8, &mut into), 2);
    assert_eq!(into.get(..2).unwrap(), b"ab");
    assert_eq!(console.waiting(), 0);
}

#[test]
fn a_read_of_an_empty_ring_takes_nothing() {
    let mut console = console();
    let mut into = [0u8; 8];
    assert_eq!(console.read(8, &mut into), 0);
}

#[test]
fn a_read_takes_no_more_than_it_was_asked_for() {
    let mut console = console();
    for byte in b"abcdef" {
        console.receive(*byte);
    }
    let mut into = [0u8; 8];
    assert_eq!(console.read(3, &mut into), 3);
    assert_eq!(into.get(..3).unwrap(), b"abc");
    assert_eq!(console.waiting(), 3);
    assert_eq!(console.read(8, &mut into), 3);
    assert_eq!(into.get(..3).unwrap(), b"def");
}

#[test]
fn a_read_takes_no_more_than_the_destination_holds() {
    let mut console = console();
    for byte in b"abcdef" {
        console.receive(*byte);
    }
    let mut into = [0u8; 2];
    assert_eq!(console.read(8, &mut into), 2);
    assert_eq!(&into, b"ab");
    assert_eq!(console.waiting(), 4);
}

#[test]
fn a_full_ring_drops_the_oldest_byte_and_counts_it() {
    let mut console = console();
    for index in 0..RECEIVE_CAPACITY {
        let byte = u8::try_from(index % 251).unwrap();
        assert!(console.receive(byte), "byte {index} did not fit");
    }
    assert_eq!(console.waiting(), RECEIVE_CAPACITY);
    assert!(!console.receive(0xFF), "the ring was full");
    assert_eq!(console.dropped(), 1);
    assert_eq!(console.waiting(), RECEIVE_CAPACITY);

    // The oldest went, the newest is there.
    let mut into = [0u8; RECEIVE_CAPACITY];
    assert_eq!(console.read(RECEIVE_CAPACITY, &mut into), RECEIVE_CAPACITY);
    assert_eq!(
        *into.first().unwrap(),
        1,
        "the second byte is now the first"
    );
    assert_eq!(*into.last().unwrap(), 0xFF);
}

#[test]
fn the_byte_the_controller_has_is_taken_from_it() {
    let mut registers = RecordingRegisters::new();
    // The data-ready bit, and the byte behind it.
    registers.script(Register::LineStatus, &[0x21]);
    registers.script(Register::Data, b"z");
    let mut console = Console::new(registers);
    assert_eq!(console.take_from_controller().count, 1);
    let mut into = [0u8; 2];
    assert_eq!(console.read(2, &mut into), 1);
    assert_eq!(into.first(), Some(&b'z'));
}

#[test]
fn an_interrupt_that_is_no_byte_arriving_takes_nothing() {
    let mut registers = RecordingRegisters::new();
    // No data-ready bit: the controller raises one interrupt for several
    // reasons, and this was one of the others.
    registers.set(Register::LineStatus, 0x20);
    let mut console = Console::new(registers);
    assert_eq!(console.take_from_controller().count, 0);
    assert_eq!(console.waiting(), 0);
}

/// Regression for #328: one interrupt drains the whole FIFO into the ring.
#[test]
fn one_interrupt_drains_the_whole_fifo() {
    let mut registers = RecordingRegisters::new();
    registers.set(Register::LineStatus, LINE_STATUS_DATA_READY);
    registers.set(Register::Data, b'x');
    let mut console = Console::new(registers);
    assert_eq!(
        console.take_from_controller().count,
        usize::from(FIFO_DEPTH)
    );
    assert_eq!(console.waiting(), usize::from(FIFO_DEPTH));
}

/// Regression for #324: a break is counted and does not reach a client.
#[test]
fn a_line_error_is_counted_and_is_no_byte_for_a_client() {
    let mut registers = RecordingRegisters::new();
    registers.script(
        Register::LineStatus,
        &[
            LINE_STATUS_DATA_READY | LINE_STATUS_BREAK,
            LINE_STATUS_DATA_READY | LINE_STATUS_OVERRUN,
        ],
    );
    registers.script(Register::Data, &[0, b'y']);
    let mut console = Console::new(registers);
    let received = console.take_from_controller();
    assert_eq!(received.count, 1);
    assert_eq!(console.line_errors(), 2, "one break, one overrun");
    let mut into = [0u8; 4];
    assert_eq!(console.read(4, &mut into), 1);
    assert_eq!(into.first(), Some(&b'y'));
}

#[test]
fn a_drain_message_carries_the_errors_and_the_bytes() {
    let received = Received {
        count: 3,
        discarded: 1,
        overruns: 1,
        errors: 0,
    };
    let mut words = [0u64; DRAIN_WORDS];
    assert_eq!(encode_drain(&received, b"abcX", &mut words), 4);
    assert_eq!(words.get(..4).unwrap(), &[2, 0x61, 0x62, 0x63]);

    let mut console = console();
    console.take_drain(words.get(..4).unwrap());
    assert_eq!(console.line_errors(), 2);
    let mut into = [0u8; 4];
    assert_eq!(console.read(4, &mut into), 3);
    assert_eq!(into.get(..3).unwrap(), b"abc");

    console.take_drain(&[]);
    assert_eq!(console.line_errors(), 2, "an empty message changes nothing");
}

#[test]
fn a_full_drain_fits_the_message() {
    let received = Received {
        count: DRAIN,
        ..Received::default()
    };
    let mut words = [0u64; DRAIN_WORDS];
    assert_eq!(
        encode_drain(&received, &[b'q'; 64], &mut words),
        DRAIN_WORDS
    );
    assert_eq!(words.first(), Some(&0));
}

#[test]
fn taking_the_console_over_turns_the_receive_interrupt_on() {
    let mut console = fresh();
    let writes = console.uart().registers().writes();
    assert!(
        writes
            .iter()
            .any(|(register, value)| *register == Register::InterruptEnable && *value & 1 != 0),
        "the receive interrupt was not enabled: {writes:?}"
    );
}
